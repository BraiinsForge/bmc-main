// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! The two BOS REST reads behind the pickaxe, authenticated with the local API token
//! boser issues to on-device clients:
//! a single line at [`DEFAULT_TOKEN_PATH`], sent as a bearer token.
//! The token is independent of the miner password,
//! so a user changing that password does not touch the pickaxe,
//! and the overlay never learns the password at all.
//!
//! The miner-info widget takes the password from the user and logs in with it instead.
//! Its side lives in the `widgets-wasm` workspace, built for wasm32,
//! so the handful of constants it would share are restated here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use ureq::Agent;
use ureq::http::StatusCode;

use crate::mining::{Board, Readings, TunerState};

/// boser, on the board the overlay runs on; the scheme supplies the port.
/// A literal address spares every poll a name lookup,
/// which ureq runs on a thread of its own once a timeout is set.
pub const DEFAULT_API_URL: &str = "http://127.0.0.1/api/v1";

/// Points the poller elsewhere, at a `bmc-netsim` instance for one.
/// A development convenience, not product configuration.
pub const API_URL_ENV: &str = "BMC_MINING_API_URL";

/// Where boser writes its token.
pub const DEFAULT_TOKEN_PATH: &str = "/var/run/boser-api.token";

/// Points the poller at another token file, alongside [`API_URL_ENV`]:
/// `bmc-netsim` checks no token, but the poller sends nothing without one.
pub const TOKEN_PATH_ENV: &str = "BMC_MINING_API_TOKEN_FILE";

const TUNER_STATE_PATH: &str = "performance/tuner-state";
const HASHBOARDS_PATH: &str = "miner/hw/hashboards";

/// Per-call cap on every ureq operation, the same as the miner-info widget's.
/// ureq 3.x has no timeout unless one is set, and a stalled boser would
/// otherwise hang the poller for the OS-level TCP timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

/// Idle connections the pool keeps: one thread polls one host in sequence,
/// so a single socket serves every request.
/// This process already carries the widget host's own pool.
const IDLE_CONNECTIONS: usize = 1;

/// Both bodies run to a few hundred bytes, so this leaves two orders of
/// magnitude of headroom and still bounds a misbehaving origin.
const MAX_BODY_BYTES: u64 = 64 * 1024;

/// Why one poll produced no readings. Every variant counts against the same
/// retry budget; the split is for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollFailure {
    /// No HTTP answer at all: connection refused, timeout, read error.
    Transport(String),
    /// The token file yielded nothing to send: unreadable, empty, or not one line.
    /// The reason names the path and the OS error, never the contents.
    NoToken { reason: String },
    /// A read was answered with something other than success.
    /// boser sends 412 here while bosminer is not running.
    Status { path: &'static str, code: u16 },
    /// A success whose body did not parse.
    Malformed { path: &'static str, reason: String },
    /// The poll panicked, and this is the message it left behind.
    Panicked(String),
}

impl std::fmt::Display for PollFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(reason) => write!(f, "no answer: {reason}"),
            Self::NoToken { reason } => write!(f, "no token to send: {reason}"),
            Self::Status { path, code } => write!(f, "{path} answered HTTP {code}"),
            Self::Malformed { path, reason } => write!(f, "{path} body did not parse: {reason}"),
            Self::Panicked(message) => write!(f, "the poll panicked: {message}"),
        }
    }
}

/// One poll's outcome, as the poller publishes it.
pub type Poll = Result<Readings, PollFailure>;

/// The token as the file holds it. Its `Debug` hides the value,
/// so no `{:?}` of a client can put it in a log.
#[derive(PartialEq, Eq)]
struct Token(String);

impl Token {
    fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// The file's single line, or why it yields no token.
fn read_token(path: &Path) -> Result<Token, PollFailure> {
    let no_token = |reason: String| PollFailure::NoToken { reason };
    let text = std::fs::read_to_string(path)
        .map_err(|err| no_token(format!("cannot read {}: {err}", path.display())))?;
    let token = text.trim();
    if token.is_empty() {
        return Err(no_token(format!("{} is empty", path.display())));
    }
    if token.chars().any(|c| c.is_ascii_control()) {
        return Err(no_token(format!(
            "{} does not hold a single-line token",
            path.display()
        )));
    }
    Ok(Token(token.to_owned()))
}

#[derive(Deserialize)]
struct TunerStateResponse {
    overall_tuner_state: TunerStateWire,
}

/// `overall_tuner_state` as it arrives: BOS numbers its `TunerState` from 1.
#[derive(Deserialize)]
#[serde(transparent)]
struct TunerStateWire(i32);

impl From<TunerStateWire> for TunerState {
    fn from(wire: TunerStateWire) -> Self {
        match wire.0 {
            1 => Self::Disabled,
            2 => Self::Stable,
            3 => Self::Tuning,
            4 => Self::Error,
            5 => Self::Continuous,
            6 => Self::Preheat,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Deserialize)]
struct HashboardsResponse {
    hashboards: Vec<HashboardWire>,
}

#[derive(Deserialize)]
struct HashboardWire {
    enabled: bool,
    stats: Option<StatsWire>,
}

#[derive(Deserialize)]
struct StatsWire {
    nominal_hashrate: Option<HashrateWire>,
    real_hashrate: Option<RealHashrateWire>,
}

#[derive(Deserialize)]
struct RealHashrateWire {
    last_1m: Option<HashrateWire>,
    last_5m: Option<HashrateWire>,
}

#[derive(Deserialize)]
struct HashrateWire {
    gigahash_per_second: Option<f64>,
}

impl From<HashboardWire> for Board {
    fn from(wire: HashboardWire) -> Self {
        let ghs = |rate: Option<HashrateWire>| rate.and_then(|rate| rate.gigahash_per_second);
        let (nominal, real) = wire.stats.map_or((None, None), |stats| {
            (stats.nominal_hashrate, stats.real_hashrate)
        });
        let (last_1m, last_5m) = real.map_or((None, None), |real| (real.last_1m, real.last_5m));
        Self {
            enabled: wire.enabled,
            nominal_ghs: ghs(nominal),
            last_1m_ghs: ghs(last_1m),
            last_5m_ghs: ghs(last_5m),
        }
    }
}

/// One boser, read as the bearer of the token in `token_path`.
/// The token is read on the first poll and kept until a read comes back 401:
/// boser restarted and issued a new one, which the file already holds.
#[derive(Debug)]
pub struct BosClient {
    agent: Agent,
    base_url: String,
    token_path: PathBuf,
    token: Option<Token>,
}

impl BosClient {
    #[must_use]
    pub fn new(base_url: impl Into<String>, token_path: impl Into<PathBuf>) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .max_idle_connections(IDLE_CONNECTIONS)
            .max_idle_connections_per_host(IDLE_CONNECTIONS)
            .build()
            .into();
        Self {
            agent,
            base_url: base_url.into(),
            token_path: token_path.into(),
            token: None,
        }
    }

    /// The local boser and its token file,
    /// unless [`API_URL_ENV`] or [`TOKEN_PATH_ENV`] point elsewhere.
    #[must_use]
    pub fn from_env() -> Self {
        let base_url = std::env::var(API_URL_ENV).unwrap_or_else(|_| DEFAULT_API_URL.to_owned());
        if base_url != DEFAULT_API_URL {
            tracing::info!(url = %base_url, "mining status polls a non-default BOS API");
        }
        let token_path = std::env::var_os(TOKEN_PATH_ENV)
            .map_or_else(|| PathBuf::from(DEFAULT_TOKEN_PATH), PathBuf::from);
        if token_path != Path::new(DEFAULT_TOKEN_PATH) {
            tracing::info!(
                path = %token_path.display(),
                "mining status reads a non-default token file"
            );
        }
        Self::new(base_url, token_path)
    }

    /// Read the tuner state and the hashboards.
    pub fn poll(&mut self) -> Poll {
        let tuner: TunerStateResponse = self.get(TUNER_STATE_PATH)?;
        let boards: HashboardsResponse = self.get(HASHBOARDS_PATH)?;
        Ok(Readings {
            tuner: TunerState::from(tuner.overall_tuner_state),
            boards: boards.hashboards.into_iter().map(Board::from).collect(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url.trim_end_matches('/'))
    }

    /// One authenticated read. A 401 re-reads the token file,
    /// and a token other than the refused one is tried once more within the same poll,
    /// so a boser restart costs no failed poll.
    fn get<T: serde::de::DeserializeOwned>(
        &mut self,
        path: &'static str,
    ) -> Result<T, PollFailure> {
        let answered = |status: StatusCode| PollFailure::Status {
            path,
            code: status.as_u16(),
        };
        let mut response = self.send(path)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            // Dropped before the retry: an unread body holds its connection,
            // and the retry would open a second one to boser.
            drop(response);
            if !self.refresh_token()? {
                return Err(answered(StatusCode::UNAUTHORIZED));
            }
            response = self.send(path)?;
        }
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            self.token = None;
        }
        if !status.is_success() {
            return Err(answered(status));
        }
        read_json(response, path)
    }

    fn send(&mut self, path: &str) -> Result<ureq::http::Response<ureq::Body>, PollFailure> {
        let url = self.url(path);
        let token = match &self.token {
            Some(token) => token,
            None => &*self.token.insert(read_token(&self.token_path)?),
        };
        self.agent
            .get(url)
            .header("Authorization", token.bearer())
            .call()
            .map_err(|err| PollFailure::Transport(err.to_string()))
    }

    /// Re-read the token file after a refusal.
    /// The token is kept only when it differs from the refused one;
    /// the same token again is dropped, so the next poll reads the file afresh.
    fn refresh_token(&mut self) -> Result<bool, PollFailure> {
        let refused = self.token.take();
        let fresh = read_token(&self.token_path)?;
        if refused.as_ref() == Some(&fresh) {
            return Ok(false);
        }
        self.token = Some(fresh);
        Ok(true)
    }
}

fn read_json<T: serde::de::DeserializeOwned>(
    response: ureq::http::Response<ureq::Body>,
    path: &'static str,
) -> Result<T, PollFailure> {
    let text = response
        .into_body()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .map_err(|err| PollFailure::Malformed {
            path,
            reason: err.to_string(),
        })?;
    serde_json::from_str(&text).map_err(|err| PollFailure::Malformed {
        path,
        reason: err.to_string(),
    })
}

/// A loopback API URL nothing answers on: the port was bound and released just now.
#[cfg(test)]
pub(crate) fn unanswered_api_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("BUG: an ephemeral loopback port should bind");
    let port = listener
        .local_addr()
        .expect("BUG: a bound listener has an address")
        .port();
    drop(listener);
    format!("http://127.0.0.1:{port}/api/v1")
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use tempfile::NamedTempFile;

    use super::*;

    /// One scripted HTTP answer: the status line's code and the JSON body.
    type Reply = (u16, &'static str);

    /// Spelled so that no field name or path can contain them by accident.
    const TOKEN: &str = "s3cr3t-a1b2";
    const ROTATED_TOKEN: &str = "s3cr3t-c3d4";

    /// A token file holding `token` on its one line, as boser writes it.
    fn token_file(token: &str) -> NamedTempFile {
        let file = NamedTempFile::new().expect("BUG: create a temporary token file");
        std::fs::write(file.path(), format!("{token}\n")).expect("BUG: write the token file");
        file
    }

    /// A loopback boser that answers `replies` in order
    /// and hands back every request it read, with a token file of its own.
    struct FakeBoser {
        base_url: String,
        requests: mpsc::Receiver<String>,
        token_file: NamedTempFile,
    }

    impl FakeBoser {
        fn start(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
            let addr = listener
                .local_addr()
                .expect("BUG: a bound listener has an address");
            let (tx, requests) = mpsc::channel();
            // Detached: the client parks the connection in its pool rather
            // than closing it, so the reader outlives the test that made it.
            std::thread::spawn(move || {
                let mut replies = replies.into_iter();
                'accept: while let Ok((mut socket, _)) = listener.accept() {
                    // ureq may reuse the connection or open a new one; either
                    // way the replies are served in the order they were scripted.
                    loop {
                        let Some(request) = read_request(&mut socket) else {
                            continue 'accept;
                        };
                        tx.send(request).expect("BUG: report the request");
                        let Some((code, body)) = replies.next() else {
                            return;
                        };
                        let response = format!(
                            "HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\n\r\n{body}",
                            body.len()
                        );
                        socket
                            .write_all(response.as_bytes())
                            .expect("BUG: write the response");
                    }
                }
            });
            Self {
                base_url: format!("http://{addr}/api/v1"),
                requests,
                token_file: token_file(TOKEN),
            }
        }

        fn client(&self) -> BosClient {
            BosClient::new(self.base_url.clone(), self.token_file.path())
        }

        /// What boser does on a restart: the file holds a new token.
        fn issue_token(&self, token: &str) {
            std::fs::write(self.token_file.path(), format!("{token}\n"))
                .expect("BUG: rewrite the token file");
        }

        /// The next request the server read; loopback needs none of the five seconds.
        fn request(&self) -> String {
            self.requests
                .recv_timeout(Duration::from_secs(5))
                .expect("BUG: the client must send a request")
        }

        fn requests(&self, count: usize) -> Vec<String> {
            (0..count).map(|_| self.request()).collect()
        }

        /// Whether the client, already returned, sent nothing.
        fn saw_no_request(&self) -> bool {
            self.requests.try_recv().is_err()
        }
    }

    /// Read one request's headers plus the body its `Content-Length` promises.
    fn read_request(socket: &mut std::net::TcpStream) -> Option<String> {
        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut chunk = [0_u8; 1_024];
            let read = socket.read(&mut chunk).expect("BUG: read the request");
            if read == 0 {
                return None;
            }
            request.extend_from_slice(&chunk[..read]);
        }
        let mut text = String::from_utf8(request).expect("BUG: the request is UTF-8");
        let length: usize = text
            .to_ascii_lowercase()
            .split("content-length:")
            .nth(1)
            .and_then(|rest| rest.split("\r\n").next())
            .map_or(0, |value| value.trim().parse().unwrap_or(0));
        let head = text.find("\r\n\r\n").expect("BUG: the headers just ended") + 4;
        let mut body = vec![0_u8; length.saturating_sub(text.len() - head)];
        if !body.is_empty() {
            socket.read_exact(&mut body).expect("BUG: read the body");
            text.push_str(&String::from_utf8(body).expect("BUG: the body is UTF-8"));
        }
        Some(text)
    }

    const TUNER_REPLY: Reply = (200, r#"{"overall_tuner_state":2}"#);
    const BOARDS_REPLY: Reply = (
        200,
        r#"{"hashboards":[{"id":"0","enabled":true,"stats":{
            "nominal_hashrate":{"gigahash_per_second":500.0},
            "real_hashrate":{"last_1m":{"gigahash_per_second":480.0},
                             "last_5m":{"gigahash_per_second":470.0}}}}]}"#,
    );
    const UNAUTHORIZED_REPLY: Reply = (401, "{}");

    /// Assert one `GET` of `path` carrying `token` as a bearer.
    fn assert_read(request: &str, path: &str, token: &str) {
        assert!(
            request.starts_with(&format!("GET /api/v1/{path} ")),
            "{request}"
        );
        assert!(
            request.contains(&format!("authorization: Bearer {token}\r\n")),
            "{request}"
        );
    }

    #[test]
    fn a_hashboard_maps_its_three_rates_and_tolerates_nulls() {
        let wire: HashboardWire = serde_json::from_str(
            r#"{
                "id": 0,
                "enabled": true,
                "chip_type": "BM1370",
                "stats": {
                    "nominal_hashrate": { "gigahash_per_second": 500.0 },
                    "real_hashrate": {
                        "last_1m": { "gigahash_per_second": 480.5 },
                        "last_5m": null,
                        "last_1h": { "gigahash_per_second": 490.0 }
                    }
                }
            }"#,
        )
        .expect("BUG: fixture is valid JSON");
        assert_eq!(
            Board::from(wire),
            Board {
                enabled: true,
                nominal_ghs: Some(500.0),
                last_1m_ghs: Some(480.5),
                last_5m_ghs: None,
            }
        );
    }

    #[test]
    fn a_hashboard_without_stats_has_no_rates() {
        let wire: HashboardWire =
            serde_json::from_str(r#"{ "enabled": false }"#).expect("BUG: fixture is valid JSON");
        assert_eq!(
            Board::from(wire),
            Board {
                enabled: false,
                nominal_ghs: None,
                last_1m_ghs: None,
                last_5m_ghs: None,
            }
        );
    }

    #[test]
    fn the_six_known_tuner_states_map_and_the_rest_stay_unknown() {
        let states = (1..=6).map(|value| TunerState::from(TunerStateWire(value)));
        assert!(states.eq([
            TunerState::Disabled,
            TunerState::Stable,
            TunerState::Tuning,
            TunerState::Error,
            TunerState::Continuous,
            TunerState::Preheat,
        ]));
        assert_eq!(
            TunerState::from(TunerStateWire(42)),
            TunerState::Unknown(42)
        );
    }

    #[test]
    fn urls_join_without_doubling_the_slash() {
        let client = BosClient::new("http://127.0.0.1:20400/api/v1/", DEFAULT_TOKEN_PATH);
        assert_eq!(
            client.url(TUNER_STATE_PATH),
            "http://127.0.0.1:20400/api/v1/performance/tuner-state"
        );
        let client = BosClient::new(DEFAULT_API_URL, DEFAULT_TOKEN_PATH);
        assert_eq!(
            client.url(HASHBOARDS_PATH),
            "http://127.0.0.1/api/v1/miner/hw/hashboards"
        );
    }

    #[test]
    fn the_token_file_s_line_is_read_without_its_newline() {
        let file = token_file(&format!("  {TOKEN}  "));
        assert_eq!(read_token(file.path()), Ok(Token(TOKEN.to_owned())));
    }

    #[test]
    fn an_empty_or_multiline_token_file_yields_no_token() {
        for contents in ["", "\n", "line\nmore\n"] {
            let file = token_file(contents);
            assert!(
                matches!(read_token(file.path()), Err(PollFailure::NoToken { .. })),
                "{contents:?}"
            );
        }
    }

    #[test]
    fn a_poll_reads_both_endpoints_as_the_bearer_of_the_file_s_token() {
        let boser = FakeBoser::start(vec![TUNER_REPLY, BOARDS_REPLY]);
        let mut client = boser.client();

        let readings = client.poll().expect("the scripted answers make a poll");

        assert_eq!(
            readings,
            Readings {
                tuner: TunerState::Stable,
                boards: vec![Board {
                    enabled: true,
                    nominal_ghs: Some(500.0),
                    last_1m_ghs: Some(480.0),
                    last_5m_ghs: Some(470.0),
                }],
            }
        );
        assert_read(&boser.request(), TUNER_STATE_PATH, TOKEN);
        assert_read(&boser.request(), HASHBOARDS_PATH, TOKEN);
    }

    #[test]
    fn the_token_is_kept_across_polls_while_boser_accepts_it() {
        let boser = FakeBoser::start(vec![TUNER_REPLY, BOARDS_REPLY, TUNER_REPLY, BOARDS_REPLY]);
        let mut client = boser.client();

        assert!(client.poll().is_ok());
        boser.issue_token(ROTATED_TOKEN);
        assert!(client.poll().is_ok());

        for request in boser.requests(4) {
            assert!(
                request.contains(&format!("Bearer {TOKEN}\r\n")),
                "the file is not re-read without a 401: {request}"
            );
        }
    }

    #[test]
    fn a_401_after_boser_rotated_the_token_retries_with_the_new_one() {
        let boser = FakeBoser::start(vec![
            TUNER_REPLY,
            BOARDS_REPLY,
            UNAUTHORIZED_REPLY,
            TUNER_REPLY,
            BOARDS_REPLY,
        ]);
        let mut client = boser.client();
        assert!(client.poll().is_ok());
        let _first_poll = boser.requests(2);

        boser.issue_token(ROTATED_TOKEN);
        let poll = client.poll();
        assert!(poll.is_ok(), "the restart costs no failed poll: {poll:?}");

        assert_read(&boser.request(), TUNER_STATE_PATH, TOKEN);
        assert_read(&boser.request(), TUNER_STATE_PATH, ROTATED_TOKEN);
        assert_read(&boser.request(), HASHBOARDS_PATH, ROTATED_TOKEN);
    }

    #[test]
    fn a_401_with_the_file_unchanged_fails_the_poll_and_re_reads_next_time() {
        let boser = FakeBoser::start(vec![UNAUTHORIZED_REPLY, TUNER_REPLY, BOARDS_REPLY]);
        let mut client = boser.client();

        assert_eq!(
            client.poll(),
            Err(PollFailure::Status {
                path: TUNER_STATE_PATH,
                code: 401
            })
        );
        assert!(client.token.is_none(), "a refused token is dropped");
        assert_read(&boser.request(), TUNER_STATE_PATH, TOKEN);
        assert!(boser.saw_no_request(), "the same token is not retried");

        boser.issue_token(ROTATED_TOKEN);
        assert!(client.poll().is_ok(), "the next poll reads the file again");
        assert_read(&boser.request(), TUNER_STATE_PATH, ROTATED_TOKEN);
    }

    #[test]
    fn a_missing_token_file_fails_the_poll_before_any_request() {
        let boser = FakeBoser::start(vec![TUNER_REPLY, BOARDS_REPLY]);
        let dir = tempfile::tempdir().expect("BUG: create a temporary directory");
        let path = dir.path().join("boser-api.token");
        let mut client = BosClient::new(boser.base_url.clone(), &path);

        assert!(matches!(client.poll(), Err(PollFailure::NoToken { .. })));
        assert!(boser.saw_no_request());

        std::fs::write(&path, format!("{TOKEN}\n")).expect("BUG: write the token file");
        assert!(
            client.poll().is_ok(),
            "the file is read again on every poll"
        );
    }

    #[test]
    fn neither_a_failure_nor_debug_output_carries_the_token() {
        let boser = FakeBoser::start(vec![UNAUTHORIZED_REPLY]);
        let mut client = boser.client();
        assert!(client.token.is_none());
        let failure = client.poll().expect_err("the read is refused");
        client.token = Some(Token(TOKEN.to_owned()));

        for text in [
            failure.to_string(),
            format!("{failure:?}"),
            format!("{client:?}"),
        ] {
            assert!(!text.contains(TOKEN), "{text}");
        }
        assert_eq!(
            format!("{:?}", Token(TOKEN.to_owned())),
            "Token(<redacted>)"
        );
    }

    #[test]
    fn a_412_while_bosminer_is_down_is_a_status_failure() {
        let boser = FakeBoser::start(vec![(412, "{}")]);

        assert_eq!(
            boser.client().poll(),
            Err(PollFailure::Status {
                path: TUNER_STATE_PATH,
                code: 412
            })
        );
    }

    #[test]
    fn a_refused_connection_is_a_transport_failure() {
        let file = token_file(TOKEN);
        let mut client = BosClient::new(unanswered_api_url(), file.path());
        assert!(matches!(client.poll(), Err(PollFailure::Transport(_))));
    }
}

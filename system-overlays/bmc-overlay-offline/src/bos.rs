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

//! The two BOS REST reads behind the pickaxe,
//! over the login mechanism the miner-info widget also uses:
//! a token from `auth/login`, sent back bare in the `Authorization` header.
//! The widget takes the password from the user and polls nothing without one;
//! the pickaxe stands on the factory credential instead.
//! The widget's side lives in the `widgets-wasm` workspace, built for wasm32,
//! so the handful of constants it would share are restated here,
//! as is its login backoff.

use std::time::{Duration, Instant};

use serde::Deserialize;
use ureq::Agent;

use crate::mining::{Board, Readings, TunerState};

/// boser, on the board the overlay runs on; the scheme supplies the port.
/// A literal address spares every poll a name lookup,
/// which ureq runs on a thread of its own once a timeout is set.
pub const DEFAULT_API_URL: &str = "http://127.0.0.1/api/v1";

/// Points the poller elsewhere, at a `bmc-netsim` instance for one.
/// A development convenience, not product configuration.
pub const API_URL_ENV: &str = "BMC_MINING_API_URL";

/// BOS authenticates only `root`. The default password stands in until boser
/// issues on-device clients a local token; a user who changed it gets 401s
/// here, and the indicator goes red once the retry budget runs out.
const USERNAME: &str = "root";
const PASSWORD: &str = "root";

/// A refused login is retried on a doubling delay, the miner-info widget's schedule,
/// so a wrong password does not hit `auth/login` on every poll for the life of the device.
const LOGIN_RETRY_BASE: Duration = Duration::from_secs(10);
const LOGIN_RETRY_CAP: Duration = Duration::from_mins(5);

fn login_retry_delay(refusals: u32) -> Duration {
    1_u32
        .checked_shl(refusals)
        .and_then(|doublings| LOGIN_RETRY_BASE.checked_mul(doublings))
        .map_or(LOGIN_RETRY_CAP, |delay| delay.min(LOGIN_RETRY_CAP))
}

/// Holds login attempts back after a refusal: any answer that yields no token.
/// A login that got no answer at all is retried at once; the miner rejected nothing.
#[derive(Debug, Default)]
struct LoginGate {
    refusals: u32,
    held_until: Option<Instant>,
}

impl LoginGate {
    /// Time left on the hold at `now`; `None` when an attempt may go out.
    fn held_for(&self, now: Instant) -> Option<Duration> {
        self.held_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|left| !left.is_zero())
    }

    fn refused(&mut self, now: Instant) {
        self.held_until = Some(now + login_retry_delay(self.refusals));
        self.refusals = self.refusals.saturating_add(1);
    }

    fn accepted(&mut self) {
        *self = Self::default();
    }
}

const LOGIN_PATH: &str = "auth/login";
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
    /// The login was answered but yielded no token.
    LoginRefused { code: u16 },
    /// No login was attempted: the backoff after a refusal has not run out.
    LoginHeld { retry_in: Duration },
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
            Self::LoginRefused { code } => write!(f, "login refused with HTTP {code}"),
            Self::LoginHeld { retry_in } => {
                write!(f, "login held back for {retry_in:?} after a refusal")
            }
            Self::Status { path, code } => write!(f, "{path} answered HTTP {code}"),
            Self::Malformed { path, reason } => write!(f, "{path} body did not parse: {reason}"),
            Self::Panicked(message) => write!(f, "the poll panicked: {message}"),
        }
    }
}

/// One poll's outcome, as the poller publishes it.
pub type Poll = Result<Readings, PollFailure>;

#[derive(Deserialize)]
struct LoginResponse {
    token: String,
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

/// A logged-in view of one boser. Holds the token across polls and drops it
/// on the first 401, so the next poll logs in again.
#[derive(Debug)]
pub struct BosClient {
    agent: Agent,
    base_url: String,
    token: Option<String>,
    login_gate: LoginGate,
}

impl BosClient {
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
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
            token: None,
            login_gate: LoginGate::default(),
        }
    }

    /// The local boser, unless [`API_URL_ENV`] points elsewhere.
    #[must_use]
    pub fn from_env() -> Self {
        let base_url = std::env::var(API_URL_ENV).unwrap_or_else(|_| DEFAULT_API_URL.to_owned());
        if base_url != DEFAULT_API_URL {
            tracing::info!(url = %base_url, "mining status polls a non-default BOS API");
        }
        Self::new(base_url)
    }

    /// Log in if needed, then read the tuner state and the hashboards.
    pub fn poll(&mut self) -> Poll {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> Poll {
        if self.token.is_none() {
            if let Some(retry_in) = self.login_gate.held_for(now) {
                return Err(PollFailure::LoginHeld { retry_in });
            }
            match self.login() {
                Ok(token) => {
                    self.login_gate.accepted();
                    self.token = Some(token);
                }
                Err(failure) => {
                    if !matches!(failure, PollFailure::Transport(_)) {
                        self.login_gate.refused(now);
                    }
                    return Err(failure);
                }
            }
        }
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

    fn login(&self) -> Result<String, PollFailure> {
        let body = serde_json::json!({ "username": USERNAME, "password": PASSWORD }).to_string();
        let response = self
            .agent
            .post(self.url(LOGIN_PATH))
            .header("content-type", "application/json")
            .send(body.as_bytes())
            .map_err(|err| PollFailure::Transport(err.to_string()))?;
        let code = response.status().as_u16();
        if !response.status().is_success() {
            return Err(PollFailure::LoginRefused { code });
        }
        let login: LoginResponse = read_json(response, LOGIN_PATH)?;
        if login.token.is_empty() {
            return Err(PollFailure::LoginRefused { code });
        }
        Ok(login.token)
    }

    fn get<T: serde::de::DeserializeOwned>(
        &mut self,
        path: &'static str,
    ) -> Result<T, PollFailure> {
        let token = self
            .token
            .as_deref()
            .expect("BUG: poll logs in before it reads");
        let response = self
            .agent
            .get(self.url(path))
            .header("Authorization", token)
            .call()
            .map_err(|err| PollFailure::Transport(err.to_string()))?;
        let status = response.status();
        if status == ureq::http::StatusCode::UNAUTHORIZED {
            self.token = None;
        }
        if !status.is_success() {
            return Err(PollFailure::Status {
                path,
                code: status.as_u16(),
            });
        }
        read_json(response, path)
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

    use super::*;

    /// One scripted HTTP answer: the status line's code and the JSON body.
    type Reply = (u16, &'static str);

    /// A loopback boser that answers `replies` in order
    /// and hands back every request it read.
    struct FakeBoser {
        base_url: String,
        requests: mpsc::Receiver<String>,
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
            }
        }

        fn client(&self) -> BosClient {
            BosClient::new(self.base_url.clone())
        }

        /// The next request the server read; loopback needs none of the five seconds.
        fn request(&self) -> String {
            self.requests
                .recv_timeout(Duration::from_secs(5))
                .expect("BUG: the client must send a request")
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

    const TOKEN_REPLY: Reply = (200, r#"{"token":"tok"}"#);
    const TUNER_REPLY: Reply = (200, r#"{"overall_tuner_state":2}"#);
    const BOARDS_REPLY: Reply = (
        200,
        r#"{"hashboards":[{"id":"0","enabled":true,"stats":{
            "nominal_hashrate":{"gigahash_per_second":500.0},
            "real_hashrate":{"last_1m":{"gigahash_per_second":480.0},
                             "last_5m":{"gigahash_per_second":470.0}}}}]}"#,
    );

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
        let client = BosClient::new("http://127.0.0.1:20400/api/v1/");
        assert_eq!(
            client.url(TUNER_STATE_PATH),
            "http://127.0.0.1:20400/api/v1/performance/tuner-state"
        );
        let client = BosClient::new(DEFAULT_API_URL);
        assert_eq!(
            client.url(HASHBOARDS_PATH),
            "http://127.0.0.1/api/v1/miner/hw/hashboards"
        );
    }

    #[test]
    fn login_retry_delay_doubles_from_the_base_and_caps() {
        assert_eq!(login_retry_delay(0), Duration::from_secs(10));
        assert_eq!(login_retry_delay(1), Duration::from_secs(20));
        assert_eq!(login_retry_delay(4), Duration::from_secs(160));
        assert_eq!(login_retry_delay(5), LOGIN_RETRY_CAP);
        assert_eq!(login_retry_delay(u32::MAX), LOGIN_RETRY_CAP);
    }

    #[test]
    fn the_gate_holds_after_a_refusal_and_lets_go_when_the_delay_runs_out() {
        let start = Instant::now();
        let mut gate = LoginGate::default();
        assert_eq!(gate.held_for(start), None);

        gate.refused(start);
        assert_eq!(gate.held_for(start), Some(Duration::from_secs(10)));
        assert_eq!(
            gate.held_for(start + Duration::from_secs(4)),
            Some(Duration::from_secs(6))
        );
        assert_eq!(gate.held_for(start + Duration::from_secs(10)), None);
    }

    #[test]
    fn repeated_refusals_lengthen_the_hold_and_an_accepted_login_resets_it() {
        let start = Instant::now();
        let mut gate = LoginGate::default();
        gate.refused(start);
        gate.refused(start);
        assert_eq!(gate.held_for(start), Some(Duration::from_secs(20)));

        gate.accepted();
        assert_eq!(gate.held_for(start), None);
        gate.refused(start);
        assert_eq!(gate.held_for(start), Some(Duration::from_secs(10)));
    }

    #[test]
    fn a_held_login_is_reported_without_touching_the_network() {
        let start = Instant::now();
        // A real attempt would come back as Transport.
        let mut client = BosClient::new(unanswered_api_url());
        client.login_gate.refused(start);
        assert_eq!(
            client.poll_at(start),
            Err(PollFailure::LoginHeld {
                retry_in: Duration::from_secs(10)
            })
        );
    }

    #[test]
    fn a_poll_logs_in_then_reads_both_endpoints_with_the_token() {
        let boser = FakeBoser::start(vec![TOKEN_REPLY, TUNER_REPLY, BOARDS_REPLY]);
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
        let login = boser.request();
        assert!(
            login.starts_with(&format!("POST /api/v1/{LOGIN_PATH} ")),
            "{login}"
        );
        for credential in [r#""username":"root""#, r#""password":"root""#] {
            assert!(login.contains(credential), "{login}");
        }
        for path in [TUNER_STATE_PATH, HASHBOARDS_PATH] {
            let read = boser.request();
            assert!(read.starts_with(&format!("GET /api/v1/{path} ")), "{read}");
            assert!(
                read.contains("authorization: tok\r\n"),
                "the token rides bare: {read}"
            );
        }
    }

    #[test]
    fn a_412_while_bosminer_is_down_is_a_status_failure() {
        let boser = FakeBoser::start(vec![TOKEN_REPLY, (412, "{}")]);

        assert_eq!(
            boser.client().poll(),
            Err(PollFailure::Status {
                path: TUNER_STATE_PATH,
                code: 412
            })
        );
    }

    #[test]
    fn a_401_drops_the_token_so_the_next_poll_logs_in_again() {
        let boser = FakeBoser::start(vec![
            TOKEN_REPLY,
            (401, "{}"),
            TOKEN_REPLY,
            TUNER_REPLY,
            BOARDS_REPLY,
        ]);
        let mut client = boser.client();

        assert!(matches!(
            client.poll(),
            Err(PollFailure::Status { code: 401, .. })
        ));
        assert!(client.token.is_none(), "a 401 drops the token");
        assert!(client.poll().is_ok(), "the next poll logs in again");

        let requests: Vec<String> = (0..5).map(|_| boser.request()).collect();
        let logins = requests
            .iter()
            .filter(|request| request.starts_with("POST"))
            .count();
        assert_eq!(logins, 2, "{requests:#?}");
    }

    #[test]
    fn a_refused_connection_is_a_transport_failure() {
        let mut client = BosClient::new(unanswered_api_url());
        assert!(matches!(client.poll(), Err(PollFailure::Transport(_))));
    }
}

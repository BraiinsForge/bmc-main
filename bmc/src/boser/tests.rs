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

use super::{Health, StateSink, StreamConfig, StreamError, Timing, read_token, spawn};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::{Stream, StreamExt, stream};
use serde::Deserialize;
use std::convert::Infallible;
use std::future::{Future, IntoFuture};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::IntervalStream;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

pub(crate) const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const BEARER_TOKEN: &str = "Bearer 0123456789abcdef0123456789abcdef";
const KEEP_ALIVE: &str = ": keep-alive\n\n";
/// JSON the event parser accepts and the sink's state type does not.
const UNKNOWN_STATE: &str = "data: {\"state\":\"SOMETHING_NEW\"}\n\n";
/// A frame whose data is not JSON at all.
const CORRUPT_STATE: &str = "data: {not json\n\n";
pub(crate) const WAIT: Duration = Duration::from_secs(10);
/// Comment period for the bodies that must stay open;
/// frequent enough that the idle timer never fires on them.
const KEEP_ALIVE_PERIOD: Duration = Duration::from_millis(100);

/// A state type of no consequence: the transport decodes whatever its sink
/// declares, so these tests need nothing of the upgrade wire contract.
#[derive(Debug, Deserialize)]
struct Step {
    step: String,
}

/// What the sink was told, in the order it was told.
#[derive(Debug, Default)]
struct Record {
    steps: Vec<String>,
    mismatches: usize,
    losses: usize,
}

struct Recorder(watch::Sender<Record>);

impl StateSink for Recorder {
    type State = Step;

    const PATH: &'static str = "/api/v1/example/state/events";

    fn observe(&mut self, state: &Step) {
        self.0
            .send_modify(|record| record.steps.push(state.step.clone()));
    }

    fn contract_mismatch(&mut self) {
        self.0.send_modify(|record| record.mismatches += 1);
    }

    fn stream_lost(&mut self) {
        self.0.send_modify(|record| record.losses += 1);
    }
}

/// Deadlines for the tests that make one fire: short enough to stay quick,
/// long enough that a loaded CI box does not trip them on a loopback socket.
pub(crate) fn timing() -> Timing {
    Timing {
        response: Duration::from_secs(1),
        first_event: Duration::from_secs(1),
        idle: Duration::from_secs(2),
        reconnect: Duration::from_millis(10),
    }
}

/// Deadlines for the tests that assert no reconnect happened:
/// far enough out that only a regression, never a scheduler stall, trips one.
fn generous_timing() -> Timing {
    Timing {
        response: Duration::from_secs(10),
        first_event: Duration::from_secs(10),
        idle: Duration::from_secs(30),
        reconnect: Duration::from_millis(10),
    }
}

fn state(step: &str) -> String {
    format!("data: {{\"step\":\"{step}\"}}\n\n")
}

fn named_event(name: &str, step: &str) -> String {
    format!("event: {name}\ndata: {{\"step\":\"{step}\"}}\n\n")
}

fn building() -> String {
    state("building")
}

fn comments_every(period: Duration) -> impl Stream<Item = String> + Send {
    IntervalStream::new(tokio::time::interval(period)).map(|_tick| KEEP_ALIVE.to_owned())
}

pub(crate) fn sse(body: impl Stream<Item = String> + Send + 'static) -> Response {
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body.map(Ok::<_, Infallible>)),
    )
        .into_response()
}

/// A state followed by an open, silent connection.
pub(crate) fn state_then_silence(first: String) -> Response {
    sse(stream::iter([first]).chain(stream::pending()))
}

/// A state followed by a connection held open by keep-alives.
fn state_then_keep_alives(first: String) -> Response {
    sse(stream::iter([first]).chain(comments_every(KEEP_ALIVE_PERIOD)))
}

#[derive(Clone, Default)]
pub(crate) struct Server {
    attempts: Arc<AtomicUsize>,
}

impl Server {
    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }

    /// Counts the connection and admits it, unless the token is wrong.
    fn admit(&self, headers: &HeaderMap) -> Option<usize> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        headers
            .get(header::AUTHORIZATION)
            .is_some_and(|value| value == BEARER_TOKEN)
            .then_some(attempt)
    }

    pub(crate) async fn wait_for_attempts(&self, expected: usize) {
        tokio::time::timeout(WAIT, async {
            while self.attempts() < expected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the observer must reconnect in time");
    }
}

async fn serve<F, Fut>(server: &Server, respond: F) -> SocketAddr
where
    F: Fn(usize) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    serve_at(server, Recorder::PATH, respond).await
}

/// Serves the state stream at `path`; `respond` sees the 1-based attempt number.
pub(crate) async fn serve_at<F, Fut>(server: &Server, path: &str, respond: F) -> SocketAddr
where
    F: Fn(usize) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    let server = server.clone();
    let router = Router::new().route(
        path,
        get(move |headers: HeaderMap| {
            let server = server.clone();
            let respond = respond.clone();
            async move {
                if let Some(attempt) = server.admit(&headers) {
                    respond(attempt).await
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("BUG: a loopback listener binds");
    let address = listener
        .local_addr()
        .expect("BUG: a bound listener has an address");
    tokio::spawn(axum::serve(listener, router).into_future());
    address
}

struct Observer {
    record: watch::Receiver<Record>,
    token_path: PathBuf,
    _dir: tempfile::TempDir,
    task: JoinHandle<()>,
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn observe(address: SocketAddr, token: Option<&str>) -> Observer {
    observe_with(address, token, timing())
}

fn observe_with(address: SocketAddr, token: Option<&str>, timing: Timing) -> Observer {
    let dir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
    let token_path = dir.path().join("token");
    if let Some(token) = token {
        std::fs::write(&token_path, token).expect("BUG: the token file writes");
    }
    let (sender, record) = watch::channel(Record::default());
    let task = spawn(
        StreamConfig {
            address,
            token_path: token_path.clone(),
            timing,
        },
        Recorder(sender),
    );
    Observer {
        record,
        token_path,
        _dir: dir,
        task,
    }
}

impl Observer {
    async fn wait_for_steps(&mut self, expected: &[&str]) {
        tokio::time::timeout(
            WAIT,
            self.record.wait_for(|record| {
                record
                    .steps
                    .iter()
                    .map(String::as_str)
                    .eq(expected.iter().copied())
            }),
        )
        .await
        .expect("the states must arrive in time")
        .expect("BUG: the record sender outlives the test");
    }

    async fn wait_for_mismatches(&mut self, expected: usize) {
        tokio::time::timeout(
            WAIT,
            self.record.wait_for(|record| record.mismatches == expected),
        )
        .await
        .expect("the undecodable states must arrive in time")
        .expect("BUG: the record sender outlives the test");
    }

    async fn wait_for_losses(&mut self, expected: usize) {
        tokio::time::timeout(
            WAIT,
            self.record.wait_for(|record| record.losses == expected),
        )
        .await
        .expect("the stream must be reported lost in time")
        .expect("BUG: the record sender outlives the test");
    }

    fn steps(&self) -> Vec<String> {
        self.record.borrow().steps.clone()
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn lines_with(&self, needle: &str) -> Vec<String> {
        let bytes = self.0.lock().expect("BUG: log lock poisoned");
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| line.contains(needle))
            .map(str::to_owned)
            .collect()
    }

    async fn wait_for_line(&self, needle: &str) -> String {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(line) = self.lines_with(needle).into_iter().next() {
                    return line;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the log line must appear in time")
    }
}

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("BUG: log lock poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Logs {
    type Writer = Logs;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The subscriber binds to the calling thread: on a `multi_thread` runtime
/// the observer task would land elsewhere and nothing would be captured.
fn capture_logs() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(logs.clone()),
    );
    (logs, tracing::subscriber::set_default(subscriber))
}

#[tokio::test]
async fn a_blank_token_file_counts_as_no_token() {
    let dir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
    let path = dir.path().join("token");
    std::fs::write(&path, "\n").expect("BUG: the token file writes");

    let error = read_token(&path)
        .await
        .expect_err("a blank token must not be sent");

    assert!(matches!(error, StreamError::TokenEmpty), "{error}");
}

#[test]
fn an_unreadable_token_before_the_first_state_warns() {
    let mut health = Health::new(Recorder::PATH);

    health.stream_lost(&StreamError::TokenUnavailable(
        std::io::ErrorKind::PermissionDenied.into(),
    ));

    // A token Boser has not written yet is the expected boot order;
    // one that cannot be read is a misconfiguration nobody would otherwise find.
    assert!(health.warned);
}

#[test]
fn a_missing_token_warns_after_the_boot_order_grace() {
    let mut health = Health::new(Recorder::PATH);

    for _ in 0..=Health::BOOT_ORDER_ATTEMPTS {
        health.stream_lost(&StreamError::TokenUnavailable(
            std::io::ErrorKind::NotFound.into(),
        ));
    }

    assert!(health.warned);
}

/// A real refusal, so these tests pin the error a cold boot actually produces
/// rather than a stand-in that happens to take the same branch.
async fn refused() -> StreamError {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("BUG: a loopback listener binds");
    let address = listener
        .local_addr()
        .expect("BUG: a bound listener has an address");
    drop(listener);
    let error = reqwest::Client::new()
        .get(format!("http://{address}/"))
        .send()
        .await
        .expect_err("a closed port must refuse the connection");
    StreamError::Request(error)
}

#[tokio::test]
async fn a_refusal_before_the_first_state_is_quiet_until_the_grace_runs_out() {
    let mut health = Health::new(Recorder::PATH);

    // Boser's init script runs after the compositor's,
    // so a cold boot refuses bmc's first connections: boot order, not a fault.
    for _attempt in 0..Health::BOOT_ORDER_ATTEMPTS {
        health.stream_lost(&refused().await);
        assert!(
            !health.warned,
            "the grace must cover a cold boot's refusals"
        );
    }

    health.stream_lost(&refused().await);

    assert!(health.warned, "a Boser that never comes up must warn once");
}

#[tokio::test]
async fn a_refusal_after_the_first_state_warns_without_a_grace() {
    let mut health = Health::new(Recorder::PATH);
    health.state_decoded();

    health.stream_lost(&refused().await);

    assert!(
        health.warned,
        "once Boser has answered, no boot order explains a refusal"
    );
}

#[tokio::test]
async fn replays_the_initial_state() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async {
        state_then_keep_alives(building())
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_steps(&["building"]).await;
    assert_eq!(server.attempts(), 1);
}

#[tokio::test]
async fn reconnects_after_the_stream_ends() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async {
        sse(stream::iter([building()]))
    })
    .await;
    let mut observer = observe(address, Some(TOKEN));

    observer.wait_for_steps(&["building"]).await;
    server.wait_for_attempts(2).await;
}

#[tokio::test]
async fn a_server_that_never_answers_hits_the_response_deadline() {
    let server = Server::default();
    let address = serve(&server, |_attempt| std::future::pending::<Response>()).await;
    let observer = observe(address, Some(TOKEN));

    server.wait_for_attempts(2).await;
    assert!(observer.steps().is_empty());
}

#[tokio::test]
async fn headers_without_a_state_hit_the_first_event_deadline_despite_keep_alives() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async {
        sse(comments_every(Duration::from_millis(50)))
    })
    .await;
    let observer = observe(address, Some(TOKEN));

    // Boser replays the current state first; a connection that only keeps
    // itself alive would leave the sink holding a stale state.
    server.wait_for_attempts(2).await;
    assert!(observer.steps().is_empty());
}

#[tokio::test]
async fn a_named_event_does_not_satisfy_the_first_state_deadline() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async {
        sse(stream::iter([named_event("ping", "ignored")])
            .chain(comments_every(Duration::from_millis(50))))
    })
    .await;
    let observer = observe(address, Some(TOKEN));

    server.wait_for_attempts(2).await;
    assert!(observer.steps().is_empty());
}

#[tokio::test]
async fn a_silent_body_after_the_first_state_hits_the_idle_deadline() {
    let server = Server::default();
    let address = serve(&server, |attempt| async move {
        if attempt == 1 {
            state_then_silence(building())
        } else {
            std::future::pending().await
        }
    })
    .await;
    let mut observer = observe(address, Some(TOKEN));

    observer.wait_for_steps(&["building"]).await;
    server.wait_for_attempts(2).await;
    // The sink hears of the drop, so it can release what the stream
    // no longer vouches for; the states it was given stand.
    observer.wait_for_losses(1).await;
    assert_eq!(observer.steps(), ["building"]);
}

#[tokio::test]
async fn keep_alives_after_the_first_state_keep_the_stream_open() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async {
        sse(stream::iter([building()])
            .chain(comments_every(KEEP_ALIVE_PERIOD).take(30))
            .chain(stream::iter([state("activating")]))
            .chain(comments_every(KEEP_ALIVE_PERIOD)))
    })
    .await;
    let mut observer = observe(address, Some(TOKEN));

    observer.wait_for_steps(&["building"]).await;
    // The comments span longer than the 2 s idle limit;
    // the second state arrives on the same connection only if each resets it.
    observer.wait_for_steps(&["building", "activating"]).await;
    assert_eq!(server.attempts(), 1);
}

#[tokio::test]
async fn named_events_are_ignored_between_states() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async {
        sse(stream::iter([
            building(),
            named_event("ping", "ignored"),
            state("activating"),
        ])
        .chain(comments_every(KEEP_ALIVE_PERIOD)))
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_steps(&["building", "activating"]).await;
    assert_eq!(server.attempts(), 1);
}

#[tokio::test]
async fn an_undecodable_state_reports_a_mismatch_and_keeps_the_stream() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    // Each frame is released only once the previous one has been observed,
    // so the reactions cannot coalesce between the assertions.
    let unknown = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let address = serve(&server, {
        let unknown = Arc::clone(&unknown);
        let resume = Arc::clone(&resume);
        move |_attempt| {
            let unknown = Arc::clone(&unknown);
            let resume = Arc::clone(&resume);
            async move {
                sse(stream::iter([building()])
                    .chain(stream::once(async move {
                        unknown.notified().await;
                        UNKNOWN_STATE.to_owned()
                    }))
                    .chain(stream::once(async move {
                        resume.notified().await;
                        state("activating")
                    }))
                    .chain(comments_every(KEEP_ALIVE_PERIOD)))
            }
        }
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_steps(&["building"]).await;

    unknown.notify_one();

    observer.wait_for_mismatches(1).await;
    let warning = logs.wait_for_line("SOMETHING_NEW").await;
    assert!(warning.contains("WARN"), "{warning}");
    assert_eq!(server.attempts(), 1);

    resume.notify_one();

    observer.wait_for_steps(&["building", "activating"]).await;
    assert_eq!(server.attempts(), 1);
}

#[tokio::test]
async fn undecodable_states_warn_once_until_one_decodes() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    // Held back so the count below cannot race the frame that raises it.
    let again = Arc::new(Notify::new());
    let address = serve(&server, {
        let again = Arc::clone(&again);
        move |_attempt| {
            let again = Arc::clone(&again);
            async move {
                sse(
                    stream::iter([format!("{}{}", UNKNOWN_STATE.repeat(3), building())])
                        .chain(stream::once(async move {
                            again.notified().await;
                            UNKNOWN_STATE.to_owned()
                        }))
                        .chain(comments_every(KEEP_ALIVE_PERIOD)),
                )
            }
        }
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_steps(&["building"]).await;

    // Boser ticks download progress every 100 ms: a state this build cannot decode
    // must not warn on every tick of a whole download.
    assert_eq!(
        logs.lines_with("WARN").len(),
        1,
        "{:?}",
        logs.lines_with("WARN")
    );

    again.notify_one();

    observer.wait_for_mismatches(4).await;
    // The state that decoded in between ended the run of repeats.
    assert_eq!(
        logs.lines_with("WARN").len(),
        2,
        "{:?}",
        logs.lines_with("WARN")
    );
}

#[tokio::test]
async fn a_corrupt_state_drops_the_stream_and_reconnects() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    // Released only once the first state has been observed,
    // so the corruption cannot coalesce with it.
    let corrupt = Arc::new(Notify::new());
    let address = serve(&server, {
        let corrupt = Arc::clone(&corrupt);
        move |attempt| {
            let corrupt = Arc::clone(&corrupt);
            async move {
                if attempt == 1 {
                    sse(stream::iter([building()])
                        .chain(stream::once(async move {
                            corrupt.notified().await;
                            CORRUPT_STATE.to_owned()
                        }))
                        .chain(comments_every(KEEP_ALIVE_PERIOD)))
                } else {
                    state_then_keep_alives(state("activating"))
                }
            }
        }
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_steps(&["building"]).await;

    corrupt.notify_one();

    // The outage warns and tells the sink before it sleeps to reconnect,
    // so both are settled once the loss is recorded.
    observer.wait_for_losses(1).await;
    let warnings = logs.lines_with("malformed event stream");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("WARN"), "{warnings:?}");

    server.wait_for_attempts(2).await;
    observer.wait_for_steps(&["building", "activating"]).await;
}

#[tokio::test]
async fn a_missing_token_is_retried_quietly_and_picked_up_once_written() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    let address = serve(&server, |_attempt| async { state_then_silence(building()) }).await;
    let mut observer = observe(address, None);

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.attempts(), 0);
    assert!(logs.lines_with("WARN").is_empty());
    assert!(!logs.lines_with("token").is_empty());

    std::fs::write(&observer.token_path, TOKEN).expect("BUG: the token file writes");
    observer.wait_for_steps(&["building"]).await;
}

#[tokio::test]
async fn a_trailing_newline_in_the_token_is_accepted() {
    let server = Server::default();
    let address = serve(&server, |_attempt| async { state_then_silence(building()) }).await;
    let mut observer = observe(address, Some(&format!("{TOKEN}\n")));

    observer.wait_for_steps(&["building"]).await;
}

#[tokio::test]
async fn warns_once_per_outage_and_reports_recovery() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    let address = serve(&server, |attempt| async move {
        // One attempt past the boot-order grace,
        // so exactly one of these refusals is the operator's, not the boot's.
        if attempt <= Health::BOOT_ORDER_ATTEMPTS + 1 {
            StatusCode::UNAUTHORIZED.into_response()
        } else {
            // Keep-alives, not silence: a body that could hit the idle deadline
            // would race a second warning against the count below.
            state_then_keep_alives(building())
        }
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_steps(&["building"]).await;
    let warnings = logs.lines_with("WARN");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    // The status is the operator's only clue that Boser rejected the token.
    assert!(warnings[0].contains("401"), "{}", warnings[0]);
    let recovered = logs.wait_for_line("recovered").await;
    assert!(recovered.contains("INFO"), "{recovered}");
    assert_eq!(
        logs.lines_with("recovered").len(),
        1,
        "{:?}",
        logs.lines_with("recovered")
    );
}

#[tokio::test]
async fn a_redirect_is_rejected_rather_than_read_as_a_stream() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    let address = serve(&server, |attempt| async move {
        if attempt == 1 {
            (StatusCode::FOUND, [(header::LOCATION, "/elsewhere")]).into_response()
        } else {
            state_then_keep_alives(building())
        }
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    // bmc's own :80 proxy and a captive portal can redirect. Preserve the
    // response status instead of reporting that its finite body ended.
    let rejected = logs.wait_for_line("response status").await;
    assert!(rejected.contains("302"), "{rejected}");
    observer.wait_for_steps(&["building"]).await;
}

#[tokio::test]
async fn a_frame_this_build_cannot_read_does_not_report_recovery() {
    let (logs, _guard) = capture_logs();
    let server = Server::default();
    // Held back so the mismatch below cannot coalesce with the state after it.
    let decodable = Arc::new(Notify::new());
    let address = serve(&server, {
        let decodable = Arc::clone(&decodable);
        move |attempt| {
            let decodable = Arc::clone(&decodable);
            async move {
                if attempt <= Health::BOOT_ORDER_ATTEMPTS + 1 {
                    StatusCode::UNAUTHORIZED.into_response()
                } else {
                    sse(stream::iter([UNKNOWN_STATE.to_owned()])
                        .chain(stream::once(async move {
                            decodable.notified().await;
                            building()
                        }))
                        .chain(comments_every(KEEP_ALIVE_PERIOD)))
                }
            }
        }
    })
    .await;
    let mut observer = observe_with(address, Some(TOKEN), generous_timing());

    observer.wait_for_mismatches(1).await;

    // A frame this build cannot read is no answer to a warned outage:
    // "recovered" beside "undecodable" would call the stream healthy
    // while nothing usable has come over it.
    assert!(
        logs.lines_with("recovered").is_empty(),
        "{:?}",
        logs.lines_with("recovered")
    );

    decodable.notify_one();

    observer.wait_for_steps(&["building"]).await;
    assert_eq!(
        logs.lines_with("recovered").len(),
        1,
        "{:?}",
        logs.lines_with("recovered")
    );
}

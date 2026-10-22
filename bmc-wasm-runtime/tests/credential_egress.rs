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

//! Substitution and the egress pin through the real `host_fetch`
//! and `host_ws_*` imports.
//!
//! The unit tests already prove the decisions; these catch the wiring going
//! wrong — moving the check after the dispatch, or dropping an early return,
//! leaves every unit test green while the secret goes out.
//!
//! The destination is a listener these tests own, because the fetch
//! interceptor, the event fixtures and the hermetic guard all answer
//! *before* the pin and so cannot stand in for a real dispatch.

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::collections::BTreeMap;
use std::io::{Read as _, Write};
use std::net::TcpListener;
use std::sync::{Mutex, Once, mpsc};
use std::time::{Duration, Instant};

use bmc_widget_manifest::credential;

use bmc_wasm_protocol::{WebsocketId, WsEventKind, WsHostClose};
use bmc_wasm_runtime::{
    BoundCredential, CredentialView, FixtureEvent, FixtureEventKind, InterceptedReply,
    RuntimeConfig, RuntimeResourceLimits, WasmWidgetRuntime,
};

mod common;
use common::headless_egl;

/// Generous, because the fetch crosses a spawned thread
/// and a slow CI box must not read as a refusal.
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a test waits to be sure something did *not* arrive:
/// a wrongly dialled socket or fetch would reach the listener from another thread.
const NO_ARRIVAL_GRACE: Duration = Duration::from_millis(300);

/// Fails the test if anything reaches `received` within [`NO_ARRIVAL_GRACE`].
fn assert_nothing_arrives(received: &mpsc::Receiver<String>, why: &str) {
    assert_eq!(
        received.recv_timeout(NO_ARRIVAL_GRACE),
        Err(mpsc::RecvTimeoutError::Timeout),
        "{why}",
    );
}

fn fetch_wat(url: &str) -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch"
        (func $host_fetch
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GET{url}")

      (global $response_count (mut i32) (i32.const 0))
      (global $last_status (mut i32) (i32.const -1))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {version})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        (drop
          (call $host_fetch
            (i32.const 10000)  ;; timeout_ms
            (i32.const 0)      ;; method_ptr
            (i32.const 3)      ;; method_len
            (i32.const 3)      ;; url_ptr
            (i32.const {url_len})
            (i32.const 0)      ;; headers_ptr
            (i32.const 0)      ;; headers_len
            (i32.const 0)      ;; body_ptr
            (i32.const 0))))   ;; body_len

      (func (export "__on_fetch_response")
        (param $request_id i32)
        (param $status i32)
        (param $body_ptr i32)
        (param $body_len i32)
        global.get $response_count
        i32.const 1
        i32.add
        global.set $response_count
        local.get $status
        global.set $last_status)

      (func (export "render") (param i32))

      (func (export "response_count") (result i32)
        global.get $response_count)

      (func (export "last_status") (result i32)
        global.get $last_status))
    "#,
        url = url,
        url_len = url.len(),
        version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

/// The slot is named `pool`, matching the placeholder both guests embed.
///
/// `allow_hosts` is the account's own pin, and replaces the type's where set.
/// It is omitted when empty, as the firmware writes it, so an unpinned test
/// exercises the absent-key path rather than an empty list.
fn config_with(type_id: &str, token: &str, allow_hosts: &[&str]) -> RuntimeConfig {
    let mut view = BTreeMap::new();
    view.insert(
        "pool".to_owned(),
        BoundCredential {
            type_id: type_id.to_owned(),
            account_name: "My account".to_owned(),
        },
    );

    let mut slot = serde_json::Map::new();
    slot.insert("fields".to_owned(), serde_json::json!({ "token": token }));
    if !allow_hosts.is_empty() {
        slot.insert("allow_hosts".to_owned(), serde_json::json!(allow_hosts));
    }

    let mut secrets = serde_json::Map::new();
    secrets.insert("pool".to_owned(), serde_json::Value::Object(slot));

    RuntimeConfig {
        credentials: CredentialView::new(view),
        credential_secrets: bmc_widget_protocol::CredentialSecrets::new(secrets),
        // Adding an interceptor or hermetic mode here would answer the fetch
        // before the pin is ever consulted, quietly voiding both tests.
        ..RuntimeConfig::default()
    }
}

/// Every log line the process emits, from any thread.
///
/// Process-wide rather than per-thread on purpose: a fetch is dispatched on a
/// spawned thread, and a secret logged from there is exactly the leak this
/// file exists to catch. A caller reads back the slice written while its own
/// body ran, which may interleave lines from a test running beside it — so
/// assert on text unique to your own request, not on text any of them emits.
static CAPTURED: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Run `body` with every log the process emits during it collected into a
/// string. The fetch logs are `debug!`, hence the level.
///
/// The subscriber is global, and installed once:
/// the spawned fetch thread inherits no thread-scoped dispatcher,
/// so a scoped one would miss the very lines this file reads back.
fn capture_logs(body: impl FnOnce()) -> String {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(|| SharedWriter)
            .init();
    });

    let start = captured().len();
    body();

    String::from_utf8_lossy(&captured()[start..]).into_owned()
}

fn captured() -> std::sync::MutexGuard<'static, Vec<u8>> {
    CAPTURED
        .lock()
        .expect("BUG: the capture buffer is never held across a panic")
}

struct SharedWriter;

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        captured().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One-shot: reports the first request received, answers 200, then stops.
fn capturing_listener() -> (u16, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("BUG: listener has a local address")
        .port();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 4096];
            let read = stream.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..read]).into_owned());
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        }
    });

    (port, rx)
}

fn runtime_for(
    url: &str,
    config: RuntimeConfig,
    gl: &headless_egl::HeadlessGl,
) -> WasmWidgetRuntime {
    runtime_from(&fetch_wat(url), config, gl)
}

fn runtime_from(
    wat: &str,
    config: RuntimeConfig,
    gl: &headless_egl::HeadlessGl,
) -> WasmWidgetRuntime {
    let _force_use = (&gl.display, gl.fbo_id, gl.proc_address());
    let wasm = wat::parse_str(wat).expect("BUG: test WAT must parse");

    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_ui::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: runtime construct")
}

#[test]
fn a_permitted_destination_receives_the_resolved_secret() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, received) = capturing_listener();
    let url = format!("http://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");

    // `GenericToken` carries no pin, so this destination is permitted.
    let config = config_with(
        credential::BuiltinType::GenericToken.id(),
        "s3cr3t-on-the-wire",
        &[],
    );
    let _runtime = runtime_for(&url, config, &gl);

    let request = received
        .recv_timeout(ARRIVAL_TIMEOUT)
        .expect("BUG: a permitted request must reach the listener");

    assert!(
        request.contains("t=s3cr3t-on-the-wire"),
        "the host must substitute the value before the request leaves: {request}"
    );
    assert!(
        !request.contains("credential.pool.token"),
        "the placeholder itself must never reach the wire: {request}"
    );
}

/// The mirror of the test above: the same run that puts the secret on the wire
/// must keep it out of the log.
/// Only the guest's placeholder form may be logged,
/// and the fetch key holds that form because it is built before substitution.
#[test]
fn the_fetch_log_names_the_placeholder_form_never_the_resolved_secret() {
    // Unlike its siblings this one must not skip: a security assertion that
    // silently does not run reads exactly like one that passed.
    let gl = headless_egl::try_init(64, 64)
        .expect("BUG: headless EGL is required to prove the fetch logs hold no secret");
    let (port, received) = capturing_listener();
    let url = format!("http://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");
    let secret = "s3cr3t-that-must-not-be-logged";
    let config = config_with(credential::BuiltinType::GenericToken.id(), secret, &[]);

    let mut request = String::new();
    let logged = capture_logs(|| {
        let mut runtime = runtime_for(&url, config, &gl);
        request = received
            .recv_timeout(ARRIVAL_TIMEOUT)
            .expect("BUG: a permitted request must reach the listener");

        // The listener has the request, but the answer is still crossing the
        // fetch thread. Polling once here usually runs before it lands, and
        // the outcome line this test reads would simply not exist yet.
        let deadline = Instant::now() + ARRIVAL_TIMEOUT;
        while runtime.call_export_i32("response_count") == Some(0) {
            assert!(Instant::now() < deadline, "the fetch never settled");
            runtime
                .poll_deliveries()
                .expect("BUG: the probe widget must settle its fetch without trapping");
        }
    });

    // Without this the test is vacuous: a substitution that never resolved
    // would put no secret on the wire, and every assertion below would hold
    // for the one reason they exist to rule out.
    assert!(
        request.contains(secret),
        "the run has to resolve the placeholder, or the log has nothing to leak: {request}",
    );

    // The outcome line specifically, not merely some line: `starting HTTP
    // fetch` carries the placeholder too, so a laxer search would stay green
    // with the outcome line demoted, stripped of its url, or deleted.
    let outcome = logged
        .lines()
        .find(|line| line.contains("fetch succeeded"))
        .expect("BUG: a settled fetch must log an outcome line");
    assert!(
        outcome.contains(&url),
        "the outcome line must carry the guest's placeholder form: {outcome}",
    );
    assert!(
        !logged.contains(secret),
        "the resolved secret must never reach a log line: {logged}",
    );
}

#[test]
fn a_pinned_credential_is_refused_before_it_can_leave_for_another_host() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, received) = capturing_listener();
    let url = format!("http://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");

    // `BraiinsPool` is pinned to api.braiins.com, so this listener is off-pin
    // however reachable it happens to be.
    let config = config_with(
        credential::BuiltinType::BraiinsPool.id(),
        "s3cr3t-must-not-travel",
        &[],
    );
    let mut runtime = runtime_for(&url, config, &gl);

    // A refusal is synchronous, so it is ready on the first poll;
    // a dispatch would still be in its worker thread, leaving the count
    // at zero. That asymmetry is what makes this specific to a refusal,
    // with no timeout to wait on.
    runtime
        .poll_deliveries()
        .expect("BUG: fixture delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("response_count"),
        Some(1),
        "an off-pin request must be answered immediately, not dispatched",
    );
    let refused = i32::try_from(bmc_wasm_protocol::FetchOutcome::Refused.to_wire())
        .expect("BUG: the refusal wire value fits an i32");
    assert_eq!(
        runtime.call_export_i32("last_status"),
        Some(refused),
        "the widget must see a refusal, not a response",
    );
    assert_nothing_arrives(
        &received,
        "a pinned credential reached a host outside its policy",
    );
}

#[test]
fn an_account_pin_carries_a_credential_its_type_would_have_refused() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, received) = capturing_listener();
    let url = format!("http://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");

    // The inverse of the test above, on the same type.
    // `BraiinsPool` admits nothing but its own API;
    // the account's pin sends this request here regardless.
    // The pair is what holds replace-not-narrow to the real dispatch.
    let config = config_with(
        credential::BuiltinType::BraiinsPool.id(),
        "s3cr3t-the-account-allows",
        &[&format!("127.0.0.1:{port}")],
    );
    let _runtime = runtime_for(&url, config, &gl);

    let request = received
        .recv_timeout(ARRIVAL_TIMEOUT)
        .expect("BUG: the account's own pin must admit its destination");

    assert!(
        request.contains("t=s3cr3t-the-account-allows"),
        "an admitted request still has to carry the substituted value: {request}"
    );
}

/// How much of the endpoint an operator must still be able to read back,
/// so that a clip down to a stub fails here rather than passing as bounded.
const LEGIBLE_URL_BYTES: usize = 500;

/// Drive one failing fetch whose url is far longer than any line should be,
/// and read the outcome line `message` names back out of the log.
///
/// `slot` is appended to the url: a placeholder naming no configured secret
/// falls through the interceptor and is refused, taking the other arm.
fn outcome_line_for(
    message: &str,
    destination: &str,
    slot: &str,
    gl: &headless_egl::HeadlessGl,
) -> String {
    /// Wide enough that no clip the runtime might take is a coincidence.
    const GUEST_URL_BYTES: usize = 8_000;

    let url = format!("{destination}{}{slot}", "p".repeat(GUEST_URL_BYTES));
    // Answered in-process, so the line under test is reached
    // without dialling anything and without a connect failure to wait on.
    let config = RuntimeConfig {
        fetch_interceptor: Some(Box::new(|_method, url: &str| {
            (!url.contains("{{")).then(|| InterceptedReply::new(500, Vec::new()))
        })),
        ..RuntimeConfig::default()
    };

    let logged = capture_logs(|| {
        let mut runtime = runtime_for(&url, config, gl);
        let deadline = Instant::now() + ARRIVAL_TIMEOUT;
        while runtime.call_export_i32("response_count") == Some(0) {
            assert!(Instant::now() < deadline, "the fetch never settled");
            runtime
                .poll_deliveries()
                .expect("BUG: the probe widget must settle its fetch without trapping");
        }
    });

    let outcome = logged
        .lines()
        .find(|line| line.contains(message) && line.contains(destination))
        .unwrap_or_else(|| panic!("BUG: a settled failure must log {message:?}"))
        .to_owned();
    assert!(
        !outcome.contains(&url),
        "the outcome line must carry a clip of the url, not the whole of it",
    );
    let legible = url
        .get(..LEGIBLE_URL_BYTES)
        .expect("BUG: the url is ASCII and far longer than its legible prefix");
    assert!(
        outcome.contains(legible),
        "the outcome line must carry enough of the url to read the endpoint back: {outcome}",
    );
    outcome
}

/// The unit test proves the clip; this proves delivery renders through it.
/// The outcome line is a `warn!` that stays on in production,
/// so a guest asking for an 8 kB url must not earn an 8 kB line
/// on every failing attempt.
///
/// Each failure arm is its own `warn!` with its own url field,
/// so a clip on one is no evidence about the other.
#[test]
fn every_fetch_outcome_line_stays_bounded_however_long_the_guest_url_is() {
    /// The log's own bound, not the constant the clip is taken against.
    const READABLE_LINE_BYTES: usize = 4_096;
    /// The capture buffer is process-wide and no sibling test names these
    /// hosts, so this is what makes the lines found below ours.
    const FAILED: &str = "http://outcome-line.test/";
    const REFUSED: &str = "http://refused-line.test/";

    let gl = headless_egl::try_init(64, 64)
        .expect("BUG: headless EGL is required to prove the outcome line stays bounded");

    for outcome in [
        outcome_line_for("fetch failed", FAILED, "", &gl),
        outcome_line_for(
            "refusing fetch",
            REFUSED,
            "{{credential.missing.token}}",
            &gl,
        ),
    ] {
        assert!(
            outcome.len() <= READABLE_LINE_BYTES,
            "an outcome line {} bytes wide is a flood: {outcome}",
            outcome.len(),
        );
    }
}

/// Connects to `url` in `init` and, when the host hands out a socket,
/// sends `message` on it straight away: the host queues it until the socket opens.
fn ws_wat(url: &str, message: &str, sdk: (u16, u16, u16)) -> String {
    format!(
        r#"
    (module
      (import "env" "host_ws_connect"
        (func $host_ws_connect (param i32 i32 i32 i32) (result i32)))
      (import "env" "host_ws_send"
        (func $host_ws_send (param i32 i32 i32) (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "{url}{message}")

      (global $ws_id (mut i32) (i32.const 0))
      (global $close_code (mut i32) (i32.const 0))
      (global $host_close (mut i32) (i32.const 0))
      (global $text_kind (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {version})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        (global.set $ws_id
          (call $host_ws_connect
            (i32.const 0)            ;; url_ptr
            (i32.const {url_len})
            (i32.const 0)            ;; headers_ptr
            (i32.const 0)))          ;; headers_len
        (if (global.get $ws_id)
          (then
            (drop
              (call $host_ws_send
                (global.get $ws_id)
                (i32.const {url_len})  ;; msg_ptr
                (i32.const {message_len}))))))

      ;; Event 2 is Close, its two data bytes the code;
      ;; event 3 is HostClosed, its one data byte the reason;
      ;; events 1 and 4 are text, unchecked and checked.
      (func (export "__on_ws_event")
        (param $ws_id i32)
        (param $kind i32)
        (param $data_ptr i32)
        (param $data_len i32)
        (if (i32.or
              (i32.eq (local.get $kind) (i32.const 1))
              (i32.eq (local.get $kind) (i32.const 4)))
          (then
            (global.set $text_kind (local.get $kind))))
        (if (i32.eq (local.get $kind) (i32.const 2))
          (then
            (global.set $close_code (i32.load16_u (local.get $data_ptr)))))
        (if (i32.eq (local.get $kind) (i32.const 3))
          (then
            (global.set $host_close (i32.load8_u (local.get $data_ptr))))))

      (func (export "render") (param i32))

      (func (export "ws_id") (result i32)
        global.get $ws_id)

      (func (export "close_code") (result i32)
        global.get $close_code)

      (func (export "host_close") (result i32)
        global.get $host_close)

      (func (export "text_kind") (result i32)
        global.get $text_kind))
    "#,
        url_len = url.len(),
        message_len = message.len(),
        version = bmc_wasm_protocol::version_pack(sdk)
    )
}

/// A Home Assistant style auth message, spending the slot every test binds.
const AUTH_MESSAGE: &str = "auth {{ credential.pool.token }}";

/// One-shot WebSocket server. Reports `accepted` once a connection arrives,
/// so even a dial whose handshake fails shows,
/// then the handshake's request target,
/// then the first frame the widget sends, as `text …` or `close <code>`.
/// After a close it reports `ended` once the widget's side has gone,
/// which the host's socket thread does only after queueing its own close event.
#[expect(
    clippy::result_large_err,
    reason = "tungstenite's handshake callback must return its own ErrorResponse"
)]
fn ws_listener() -> (u16, mpsc::Receiver<String>) {
    use tungstenite::Message;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("BUG: listener has a local address")
        .port();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let _ = tx.send("accepted".to_owned());
        let target_tx = tx.clone();
        let report_target = move |request: &Request, response: Response| {
            let _ = target_tx.send(request.uri().to_string());
            Ok::<Response, ErrorResponse>(response)
        };
        let Ok(mut socket) = tungstenite::accept_hdr(stream, report_target) else {
            return;
        };
        let frame = match socket.read() {
            Ok(Message::Text(text)) => format!("text {text}"),
            Ok(Message::Close(frame)) => {
                format!("close {}", frame.map_or(0, |frame| u16::from(frame.code)))
            }
            Ok(
                other @ (Message::Binary(_)
                | Message::Ping(_)
                | Message::Pong(_)
                | Message::Frame(_)),
            ) => format!("{other:?}"),
            Err(error) => format!("error {error}"),
        };
        let closed = frame.starts_with("close");
        let _ = tx.send(frame);
        if closed {
            let _ = tx.send(wait_for_the_end(socket));
        }
    });

    (port, rx)
}

/// Reads `socket` until the other side drops the connection:
/// `ended` then, or what went wrong.
fn wait_for_the_end(mut socket: tungstenite::WebSocket<std::net::TcpStream>) -> String {
    while socket.read().is_ok() {}
    let stream = socket.get_mut();
    if let Err(error) = stream.set_read_timeout(Some(ARRIVAL_TIMEOUT)) {
        return format!("error {error}");
    }
    let mut rest = [0_u8; 64];
    loop {
        match stream.read(&mut rest) {
            Ok(0) => return "ended".to_owned(),
            Ok(_) => {}
            Err(error) => return format!("error {error}"),
        }
    }
}

/// A port nothing listens on, so a connect to it fails at once.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("BUG: bind an ephemeral port")
        .local_addr()
        .expect("BUG: listener has a local address")
        .port()
}

fn ws_runtime(
    url: &str,
    message: &str,
    config: RuntimeConfig,
    gl: &headless_egl::HeadlessGl,
) -> WasmWidgetRuntime {
    runtime_from(
        &ws_wat(url, message, bmc_wasm_protocol::PROTOCOL_VERSION),
        config,
        gl,
    )
}

/// The last protocol before the host's closes became events of their own.
const PROTOCOL_BEFORE_HOST_CLOSE: (u16, u16, u16) = (0, 9, 0);

/// The host close the probe widget was delivered, after one delivery pass.
fn host_close_after_a_delivery(runtime: &mut WasmWidgetRuntime) -> Option<WsHostClose> {
    runtime
        .poll_deliveries()
        .expect("BUG: the probe widget must take its host close without trapping");
    let raw = runtime
        .call_export_i32("host_close")
        .expect("BUG: the probe widget exports its host close");
    WsHostClose::from_wire(u8::try_from(raw).expect("BUG: the probe stores one byte"))
}

/// The next thing the listener reports past the bare `accepted`.
fn received(frames: &mpsc::Receiver<String>, what: &str) -> String {
    loop {
        let frame = frames
            .recv_timeout(ARRIVAL_TIMEOUT)
            .unwrap_or_else(|_| panic!("BUG: the listener never saw {what}"));
        if frame != "accepted" {
            return frame;
        }
    }
}

/// Delivers until the probe widget sees a close, and returns its code.
fn close_code_after_deliveries(runtime: &mut WasmWidgetRuntime) -> i32 {
    let deadline = Instant::now() + ARRIVAL_TIMEOUT;
    loop {
        runtime
            .poll_deliveries()
            .expect("BUG: the probe widget must take its close without trapping");
        match runtime.call_export_i32("close_code") {
            Some(0) | None => assert!(Instant::now() < deadline, "the socket never closed"),
            Some(code) => return code,
        }
    }
}

/// One-shot server that answers the upgrade with `status` and no WebSocket,
/// sending `location` along when given.
/// Reports `accepted`, then the handshake's request target.
#[expect(
    clippy::result_large_err,
    reason = "tungstenite's handshake callback must return its own ErrorResponse"
)]
fn refusing_listener(status: u16, location: Option<String>) -> (u16, mpsc::Receiver<String>) {
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("BUG: listener has a local address")
        .port();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let _ = tx.send("accepted".to_owned());
        let refuse = move |request: &Request, _: Response| {
            let _ = tx.send(request.uri().to_string());
            let mut answer = ErrorResponse::new(None);
            *answer.status_mut() =
                tungstenite::http::StatusCode::from_u16(status).expect("BUG: a valid test status");
            if let Some(location) = &location {
                answer.headers_mut().insert(
                    "Location",
                    location.parse().expect("BUG: a valid test header"),
                );
            }
            Err(answer)
        };
        let _ = tungstenite::accept_hdr(stream, refuse);
    });

    (port, rx)
}

#[test]
fn a_permitted_socket_carries_the_resolved_secret_in_its_url_and_messages() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, frames) = ws_listener();
    let url = format!("ws://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");
    let config = config_with(
        credential::BuiltinType::GenericToken.id(),
        "s3cr3t-on-the-socket",
        &[],
    );
    let _runtime = ws_runtime(&url, AUTH_MESSAGE, config, &gl);

    assert_eq!(
        received(&frames, "the handshake"),
        "/?t=s3cr3t-on-the-socket",
        "the connect URL must be substituted before it is dialled",
    );
    assert_eq!(
        received(&frames, "the auth message"),
        "text auth s3cr3t-on-the-socket",
        "the message must be substituted before it is sent",
    );
}

/// The connect and connect-failure lines both name a URL,
/// and so both must name the placeholder form.
#[test]
fn the_socket_logs_name_the_placeholder_form_never_the_resolved_secret() {
    // Must not skip, for the reason the fetch twin above gives.
    let gl = headless_egl::try_init(64, 64)
        .expect("BUG: headless EGL is required to prove the socket logs hold no secret");
    let secret = "s3cr3t-the-socket-must-not-log";
    let (port, frames) = ws_listener();
    let reachable = format!("ws://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");
    let unreachable = format!(
        "ws://127.0.0.1:{}/?t={{{{ credential.pool.token }}}}",
        closed_port()
    );
    let (refusing_port, _refused) = refusing_listener(400, None);
    let refusing = format!("ws://127.0.0.1:{refusing_port}/?t={{{{ credential.pool.token }}}}");
    let config = || config_with(credential::BuiltinType::GenericToken.id(), secret, &[]);

    let mut frame = String::new();
    let logged = capture_logs(|| {
        let _connected = ws_runtime(&reachable, AUTH_MESSAGE, config(), &gl);
        received(&frames, "the handshake");
        // The thread logs the connect before it sends anything,
        // so once the message is here, so is the line.
        frame = received(&frames, "the auth message");

        for url in [&unreachable, &refusing] {
            close_code_after_deliveries(&mut ws_runtime(url, AUTH_MESSAGE, config(), &gl));
        }
    });

    assert!(
        frame.contains(secret),
        "the run has to resolve the placeholder, or the log has nothing to leak: {frame}",
    );
    for (line, url) in [
        ("WS connected", &reachable),
        ("WS connect failed", &unreachable),
        ("WS handshake failed", &refusing),
    ] {
        assert!(
            logged
                .lines()
                .any(|logged| logged.contains(line) && logged.contains(url.as_str())),
            "the {line:?} line must carry the guest's placeholder form: {logged}",
        );
    }
    assert!(
        !logged.contains(secret),
        "the resolved secret must never reach a log line: {logged}",
    );
}

/// The widget still gets a socket,
/// so it learns of the refusal from a host callback, not inside its own connect.
#[test]
fn a_connect_spending_a_credential_off_its_pin_is_refused_without_dialling() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, frames) = ws_listener();
    let url = format!("ws://127.0.0.1:{port}/?t={{{{ credential.pool.token }}}}");
    // `BraiinsPool` is pinned to api.braiins.com.
    let config = config_with(
        credential::BuiltinType::BraiinsPool.id(),
        "s3cr3t-must-not-travel",
        &[],
    );
    let mut runtime = ws_runtime(&url, AUTH_MESSAGE, config, &gl);

    assert_ne!(
        runtime.call_export_i32("ws_id"),
        Some(0),
        "a refused connect must still hand the widget a socket",
    );
    assert_eq!(
        host_close_after_a_delivery(&mut runtime),
        Some(WsHostClose::Credential),
    );
    assert_nothing_arrives(
        &frames,
        "a pinned credential reached a host outside its policy",
    );
}

/// The egress pin judged the URL's host, so a redirect must not carry the socket elsewhere.
#[test]
fn a_redirect_on_the_handshake_is_never_followed() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (elsewhere, reached) = ws_listener();
    let (port, asked) = refusing_listener(302, Some(format!("ws://127.0.0.1:{elsewhere}/")));
    let url = format!("ws://127.0.0.1:{port}/");
    let config = config_with(credential::BuiltinType::GenericToken.id(), "s3cr3t", &[]);
    let mut runtime = ws_runtime(&url, AUTH_MESSAGE, config, &gl);

    received(&asked, "the handshake");
    close_code_after_deliveries(&mut runtime);
    assert_nothing_arrives(
        &reached,
        "a redirect carried the socket to a host the pin never judged",
    );
}

/// No TLS is built in, so a `wss://` socket would otherwise handshake in cleartext.
#[test]
fn a_wss_socket_is_never_dialled() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, frames) = ws_listener();
    let url = format!("wss://127.0.0.1:{port}/");
    let config = config_with(credential::BuiltinType::GenericToken.id(), "s3cr3t", &[]);
    let mut runtime = ws_runtime(&url, AUTH_MESSAGE, config, &gl);

    close_code_after_deliveries(&mut runtime);
    assert_nothing_arrives(&frames, "a wss:// socket was dialled in cleartext");
}

/// Refused at the call, as every other resource is past its limit:
/// no handle, so nothing for the host to keep until a delivery reports it.
#[test]
fn a_connect_past_the_socket_limit_gets_no_socket() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, frames) = ws_listener();
    let url = format!("ws://127.0.0.1:{port}/");
    for protocol in [
        bmc_wasm_protocol::PROTOCOL_VERSION,
        PROTOCOL_BEFORE_HOST_CLOSE,
    ] {
        let config = RuntimeConfig {
            resource_limits: RuntimeResourceLimits {
                max_websockets: 0,
                ..RuntimeResourceLimits::default()
            },
            ..config_with(credential::BuiltinType::GenericToken.id(), "s3cr3t", &[])
        };
        let mut runtime = runtime_from(&ws_wat(&url, AUTH_MESSAGE, protocol), config, &gl);

        assert_eq!(runtime.call_export_i32("ws_id"), Some(0), "{protocol:?}");
        assert!(
            !runtime.has_active_websockets(),
            "{protocol:?}: a refused connect must leave nothing in the host's map",
        );
        assert_eq!(
            host_close_after_a_delivery(&mut runtime),
            None,
            "{protocol:?}"
        );
    }
    assert_nothing_arrives(&frames, "a socket past the limit was dialled");
}

/// The socket opens, since its URL spends nothing,
/// and then has to be closed rather than carry the message.
#[test]
fn a_refused_message_never_leaves_and_closes_the_socket_as_refused() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };

    for (type_id, message, why) in [
        (
            credential::BuiltinType::BraiinsPool.id(),
            AUTH_MESSAGE,
            "a destination outside the pin",
        ),
        (
            credential::BuiltinType::GenericToken.id(),
            "auth {{ credential.weather.token }}",
            "an unbound slot",
        ),
        (
            credential::BuiltinType::GenericToken.id(),
            "auth {{ credential.pool.tokne }}",
            "an unknown field",
        ),
    ] {
        let (port, frames) = ws_listener();
        let url = format!("ws://127.0.0.1:{port}/");
        let mut runtime = ws_runtime(&url, message, config_with(type_id, "s3cr3t", &[]), &gl);

        received(&frames, "the handshake");
        assert_eq!(
            received(&frames, "the first frame"),
            "close 1008",
            "{why}: the peer must get the close, never the message",
        );
        // The socket thread queues its own close before it drops the connection,
        // so from here the host's close has the thread's to win over.
        assert_eq!(received(&frames, "the connection's end"), "ended");
        assert_eq!(
            host_close_after_a_delivery(&mut runtime),
            Some(WsHostClose::Credential),
            "the widget must learn of {why} as the host's close",
        );
        assert_eq!(
            runtime.call_export_i32("close_code"),
            Some(0),
            "{why}: the host's close is none a peer could have sent",
        );
    }
}

/// The event kinds a widget gets follow the protocol it declared, not the host's own.
#[test]
fn each_widget_gets_the_websocket_events_its_protocol_reads() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let ws_id = WebsocketId::from_wire(1).expect("BUG: the first socket's id is 1");
    let fixtures = || {
        [
            FixtureEventKind::WsOpen { ws_id },
            FixtureEventKind::WsMessage {
                ws_id,
                data: b"hello".to_vec(),
            },
            FixtureEventKind::WsHostClosed {
                ws_id,
                reason: WsHostClose::Credential,
            },
        ]
        .into_iter()
        .map(|kind| FixtureEvent { at_ms: 0, kind })
        .collect()
    };
    for (protocol, text, close_code, host_close) in [
        (
            PROTOCOL_BEFORE_HOST_CLOSE,
            WsEventKind::Text,
            bmc_wasm_protocol::LEGACY_HOST_CLOSE_CODE,
            None,
        ),
        (
            bmc_wasm_protocol::PROTOCOL_VERSION,
            WsEventKind::Text,
            0,
            Some(WsHostClose::Credential),
        ),
    ] {
        let config = RuntimeConfig {
            event_fixtures: fixtures(),
            ..config_with(credential::BuiltinType::GenericToken.id(), "s3cr3t", &[])
        };
        let mut runtime =
            runtime_from(&ws_wat("ws://127.0.0.1:9/", "hello", protocol), config, &gl);
        runtime.inject_fixture_events(0);

        assert_eq!(
            host_close_after_a_delivery(&mut runtime),
            host_close,
            "{protocol:?}"
        );
        assert_eq!(
            runtime.call_export_i32("text_kind"),
            i32::try_from(text.to_wire()).ok(),
            "{protocol:?}"
        );
        assert_eq!(
            runtime.call_export_i32("close_code"),
            Some(i32::from(close_code)),
            "{protocol:?}"
        );
    }
}

/// A widget built before the host's closes were events still hears of them, as close codes.
#[test]
fn an_older_widget_gets_the_hosts_close_as_a_close_code() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let (port, frames) = ws_listener();
    let url = format!("ws://127.0.0.1:{port}/");
    let config = config_with(credential::BuiltinType::BraiinsPool.id(), "s3cr3t", &[]);
    let mut runtime = runtime_from(
        &ws_wat(&url, AUTH_MESSAGE, PROTOCOL_BEFORE_HOST_CLOSE),
        config,
        &gl,
    );

    received(&frames, "the handshake");
    assert_eq!(received(&frames, "the first frame"), "close 1008");
    assert_eq!(received(&frames, "the connection's end"), "ended");
    assert_eq!(
        close_code_after_deliveries(&mut runtime),
        i32::from(bmc_wasm_protocol::LEGACY_HOST_CLOSE_CODE),
        "an older widget reads the host's close as the abnormal closure it always saw",
    );
    assert_eq!(
        runtime.call_export_i32("host_close"),
        Some(0),
        "an older widget has no event 3 to read",
    );
}

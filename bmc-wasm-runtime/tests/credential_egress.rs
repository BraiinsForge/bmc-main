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

//! Substitution and the egress pin through the real `host_fetch` import.
//!
//! The unit tests already prove the decisions; these catch the wiring going
//! wrong — moving the check after the dispatch, or dropping an early return,
//! leaves every unit test green while the secret goes out.
//!
//! The destination is a listener these tests own, because the fetch
//! interceptor and the hermetic guard both answer *before* the pin
//! and so cannot stand in for a real dispatch.

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::collections::BTreeMap;
use std::io::{Read as _, Write};
use std::net::TcpListener;
use std::sync::{Mutex, Once, mpsc};
use std::time::{Duration, Instant};

use bmc_widget_manifest::credential;

use bmc_wasm_runtime::{BoundCredential, CredentialView, RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

/// Generous, because the fetch crosses a spawned thread
/// and a slow CI box must not read as a refusal.
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(10);

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
        version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
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
    let _force_use = (&gl.display, gl.fbo_id, gl.proc_address());
    let wasm = wat::parse_str(fetch_wat(url)).expect("BUG: fetch WAT must parse");

    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
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
    assert!(
        received.try_recv().is_err(),
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
            (!url.contains("{{")).then(|| (500, Vec::new()))
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

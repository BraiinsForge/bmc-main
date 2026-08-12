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
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

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

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

//! Fetch log admission through immediate, delayed, refused and cancelled
//! runtime delivery paths.

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Local;

use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

const FAILURE_REMINDER_INTERVAL_MS: u64 = 30 * 60 * 1_000;

fn fetch_probe_wat(url: &str) -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch"
        (func $host_fetch
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))
      (import "env" "host_fetch_cancel"
        (func $host_fetch_cancel (param i32) (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GET{url}")

      (global $request_id (mut i32) (i32.const 0))
      (global $response_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {version})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init"))

      (func (export "fetch_now") (result i32)
        (global.set $request_id
          (call $host_fetch
            (i32.const 10000)
            (i32.const 0)
            (i32.const 3)
            (i32.const 3)
            (i32.const {url_len})
            (i32.const 0)
            (i32.const 0)
            (i32.const 0)
            (i32.const 0)))
        global.get $request_id)

      (func (export "fetch_delayed") (result i32)
        (global.set $request_id
          (call $host_fetch_after
            (i32.const 0)
            (i32.const 10000)
            (i32.const 0)
            (i32.const 3)
            (i32.const 3)
            (i32.const {url_len})
            (i32.const 0)
            (i32.const 0)
            (i32.const 0)
            (i32.const 0)))
        global.get $request_id)

      (func (export "cancel") (result i32)
        (call $host_fetch_cancel (global.get $request_id)))

      (func (export "__on_fetch_response")
        (param $request_id i32)
        (param $status i32)
        (param $body_ptr i32)
        (param $body_len i32)
        global.get $response_count
        i32.const 1
        i32.add
        global.set $response_count)

      (func (export "render") (param i32))

      (func (export "response_count") (result i32)
        global.get $response_count))
    "#,
        url_len = url.len(),
        version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn runtime_for(
    url: &str,
    status: Arc<AtomicU32>,
    gl: &headless_egl::HeadlessGl,
) -> WasmWidgetRuntime {
    let _force_use = (&gl.display, gl.fbo_id, gl.proc_address());
    let wasm = wat::parse_str(fetch_probe_wat(url)).expect("BUG: fetch probe WAT must parse");
    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        Local::now().fixed_offset(),
        RuntimeConfig {
            // The interceptor settles a fetch before `credentials::spend` runs,
            // so a URL carrying a placeholder has to fall through it to reach
            // the refusal path at all. Nothing dials out: the placeholder names
            // a slot no secret is bound to, so the request is refused first.
            fetch_interceptor: Some(Box::new(move |_method, url| {
                (!url.contains("{{")).then(|| (status.load(Ordering::Relaxed), Vec::new()))
            })),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct")
}

fn fetch_and_deliver(runtime: &mut WasmWidgetRuntime, export: &str) {
    assert_ne!(
        runtime.call_export_i32(export),
        Some(0),
        "the runtime must accept the test fetch",
    );
    runtime
        .poll_deliveries()
        .expect("BUG: the probe widget must settle its fetch without trapping");
}

#[test]
fn immediate_and_delayed_failures_share_an_episode_and_emit_a_reminder() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let status = Arc::new(AtomicU32::new(500));
    let mut runtime = runtime_for("https://example.test/data", status, &gl);

    fetch_and_deliver(&mut runtime, "fetch_now");
    assert_eq!(runtime.test_fetch_failure_log_count(), 1);

    fetch_and_deliver(&mut runtime, "fetch_delayed");
    assert_eq!(
        runtime.test_fetch_failure_log_count(),
        1,
        "the delayed path must share suppression with the immediate path",
    );

    runtime.set_time(Local::now().fixed_offset(), FAILURE_REMINDER_INTERVAL_MS);
    fetch_and_deliver(&mut runtime, "fetch_delayed");
    assert_eq!(runtime.test_fetch_failure_log_count(), 2);
}

#[test]
fn changed_status_is_admitted_and_success_resets_the_episode() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let status = Arc::new(AtomicU32::new(500));
    let mut runtime = runtime_for("https://example.test/data", Arc::clone(&status), &gl);

    fetch_and_deliver(&mut runtime, "fetch_now");
    status.store(503, Ordering::Relaxed);
    fetch_and_deliver(&mut runtime, "fetch_now");
    assert_eq!(runtime.test_fetch_failure_log_count(), 2);

    status.store(200, Ordering::Relaxed);
    fetch_and_deliver(&mut runtime, "fetch_now");
    assert_eq!(runtime.test_fetch_failure_log_count(), 2);

    status.store(503, Ordering::Relaxed);
    fetch_and_deliver(&mut runtime, "fetch_now");
    assert_eq!(
        runtime.test_fetch_failure_log_count(),
        3,
        "a success must make the next failure the first of a new episode",
    );
}

#[test]
fn identical_keys_are_isolated_between_widget_runtimes() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let mut first = runtime_for(
        "https://example.test/data",
        Arc::new(AtomicU32::new(500)),
        &gl,
    );
    let mut second = runtime_for(
        "https://example.test/data",
        Arc::new(AtomicU32::new(500)),
        &gl,
    );

    fetch_and_deliver(&mut first, "fetch_now");
    fetch_and_deliver(&mut second, "fetch_now");

    assert_eq!(first.test_fetch_failure_log_count(), 1);
    assert_eq!(second.test_fetch_failure_log_count(), 1);
}

#[test]
fn aborted_settlement_is_neutral_and_does_not_hide_the_next_failure() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let status = Arc::new(AtomicU32::new(500));
    let mut runtime = runtime_for("https://example.test/data", status, &gl);

    assert_ne!(runtime.call_export_i32("fetch_now"), Some(0));
    assert_eq!(
        runtime.call_export_i32("cancel"),
        Some(0),
        "the intercepted response is already in flight and must settle as aborted",
    );
    runtime
        .poll_deliveries()
        .expect("BUG: the probe widget must settle its fetch without trapping");
    assert_eq!(runtime.test_fetch_failure_log_count(), 0);

    fetch_and_deliver(&mut runtime, "fetch_now");
    assert_eq!(runtime.test_fetch_failure_log_count(), 1);
}

#[test]
fn repeated_credential_refusal_is_suppressed_through_runtime_delivery() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let mut runtime = runtime_for(
        "https://example.test/{{credential.missing.token}}",
        Arc::new(AtomicU32::new(500)),
        &gl,
    );

    fetch_and_deliver(&mut runtime, "fetch_now");
    fetch_and_deliver(&mut runtime, "fetch_now");

    assert_eq!(
        runtime.test_last_fetch_refusal(),
        Some(
            r#"credential placeholder unresolved err=no secret available for credential slot "missing""#
        ),
        "the second attempt must itself be refused for want of a secret, \
         rather than settled by the interceptor behind a suppressed line",
    );
    assert_eq!(runtime.test_fetch_failure_log_count(), 1);
    assert_eq!(runtime.call_export_i32("response_count"), Some(2));
}

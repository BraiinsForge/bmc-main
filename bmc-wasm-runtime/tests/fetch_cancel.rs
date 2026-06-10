// Copyright (C) 2026  Braiins Systems s.r.o.

//! `host_fetch_cancel` removes a still-queued delayed fetch and reports it,
//! but reports a no-op for anything not in the delayed queue (already in
//! flight or unknown), so the guest knows to wait for the response instead.

#![cfg(all(target_os = "linux", feature = "testing"))]

use chrono::Local;

use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

fn fetch_cancel_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))
      (import "env" "host_fetch_cancel"
        (func $host_fetch_cancel (param i32) (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://example.test/delayed")

      (global $req_id (mut i32) (i32.const 0))
      (global $response_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        (global.set $req_id
          (call $host_fetch_after
            (i32.const 10)    ;; delay_ms
            (i32.const 10000) ;; timeout_ms
            (i32.const 0)     ;; method_ptr
            (i32.const 3)     ;; method_len
            (i32.const 3)     ;; url_ptr
            (i32.const 28)    ;; url_len
            (i32.const 0)     ;; headers_ptr
            (i32.const 0)     ;; headers_len
            (i32.const 0)     ;; body_ptr
            (i32.const 0)))) ;; body_len

      (func (export "cancel_it") (result i32)
        (call $host_fetch_cancel (global.get $req_id)))

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
    "#
}

#[test]
fn cancel_removes_queued_delayed_fetch_and_reports_status() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let _force_use = (&gl.display, gl.fbo_id, gl.proc_address());

    let wasm = wat::parse_str(fetch_cancel_wat()).expect("BUG: fetch-cancel WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| Some((200, b"ok".to_vec())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    assert_eq!(
        runtime.call_export_i32("cancel_it"),
        Some(1),
        "cancelling a queued delayed fetch should report it was removed",
    );
    assert_eq!(
        runtime.call_export_i32("cancel_it"),
        Some(0),
        "cancelling again should report a no-op once the fetch is gone",
    );

    runtime.set_time(Local::now().fixed_offset(), 10);
    runtime.poll_deliveries();
    assert_eq!(
        runtime.call_export_i32("response_count"),
        Some(0),
        "a cancelled delayed fetch must never deliver a response",
    );
}

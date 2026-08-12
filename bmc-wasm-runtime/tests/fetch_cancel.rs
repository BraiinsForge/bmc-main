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

//! `host_fetch_cancel` settles the request as aborted whatever the transport
//! managed to do: a still-queued delayed fetch is dropped and owed its
//! settlement, one already away has the reply it brings rewritten. It reports
//! only which of the two happened.

#![cfg(all(target_os = "linux", feature = "testing"))]

use chrono::Local;

use bmc_wasm_protocol::FetchOutcome;
use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

fn fetch_cancel_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))
      (import "env" "host_fetch_cancel"
        (func $host_fetch_cancel (param i32) (result i32)))
      (import "env" "host_fetch"
        (func $host_fetch
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://example.test/delayed")

      (global $req_id (mut i32) (i32.const 0))
      (global $response_count (mut i32) (i32.const 0))
      (global $last_status (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

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

      (func (export "fetch_now") (result i32)
        (global.set $req_id
          (call $host_fetch
            (i32.const 10000) ;; timeout_ms
            (i32.const 0)     ;; method_ptr
            (i32.const 3)     ;; method_len
            (i32.const 3)     ;; url_ptr
            (i32.const 28)    ;; url_len
            (i32.const 0)     ;; headers_ptr
            (i32.const 0)     ;; headers_len
            (i32.const 0)     ;; body_ptr
            (i32.const 0)))  ;; body_len
        (global.get $req_id))

      (func (export "__on_fetch_response")
        (param $request_id i32)
        (param $status i32)
        (param $body_ptr i32)
        (param $body_len i32)
        (global.set $last_status (local.get $status))
        global.get $response_count
        i32.const 1
        i32.add
        global.set $response_count)

      (func (export "render") (param i32))

      (func (export "last_status") (result i32)
        global.get $last_status)

      (func (export "response_count") (result i32)
        global.get $response_count))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Removed whole — no callback owed, slot freed in-call — so a poll's
/// cancel-and-resend swap never costs two slots.
#[test]
fn cancel_removes_a_queued_delayed_fetch_entirely() {
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
        chrono::Local::now().fixed_offset(),
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
    runtime
        .poll_deliveries()
        .expect("BUG: fixture delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("response_count"),
        Some(0),
        "a cancelled delayed fetch must never deliver a response",
    );
}

/// The transport cannot be called back once it is away, so the reply it brings
/// is rewritten — otherwise a cancel would land as data the caller asked to forget,
/// which is what makes a rebound widget merge the wrong account's page.
#[test]
fn cancel_rewrites_the_settlement_of_a_fetch_already_away() {
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
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| Some((200, b"ok".to_vec())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    // The interceptor answers at request time, so the reply is already waiting
    // when the cancel arrives — the host is past stopping it.
    assert_ne!(
        runtime.call_export_i32("fetch_now"),
        Some(0),
        "the fetch must be accepted for the cancel to have a target",
    );
    assert_eq!(
        runtime.call_export_i32("cancel_it"),
        Some(0),
        "a fetch the host cannot stop reports no removal",
    );

    runtime
        .poll_deliveries()
        .expect("BUG: fixture delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("response_count"),
        Some(1),
        "the reply still settles the request",
    );
    assert_eq!(
        runtime.call_export_i32("last_status"),
        Some(FetchOutcome::Aborted.to_wire().cast_signed()),
        "and it reads as the cancel it was, not as the origin's 200",
    );
}

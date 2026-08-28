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

//! Delayed fetch delivery depends on host-provided monotonic time.
//!
//! The multi-widget host must call `set_time` before polling deliveries, even
//! for dormant/non-rendering slots. Otherwise delayed fetches stay queued
//! forever while `has_pending_io()` keeps waking the host.

#![cfg(all(target_os = "linux", feature = "testing"))]

use chrono::Local;

use bmc_wasm_runtime::{InterceptedReply, RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

fn delayed_fetch_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://example.test/delayed")

      (global $response_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        (drop
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
            (i32.const 0))))  ;; body_len

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
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

#[test]
fn delayed_fetch_fires_only_after_host_advances_monotonic_time() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let _force_use = (&gl.display, gl.fbo_id, gl.proc_address());

    let wasm = wat::parse_str(delayed_fetch_wat()).expect("BUG: delayed-fetch WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply::new(200, b"ok".to_vec()))
            })),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    assert_eq!(runtime.call_export_i32("response_count"), Some(0));

    runtime
        .poll_deliveries()
        .expect("BUG: fixture delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("response_count"),
        Some(0),
        "delayed fetch fired before the host advanced monotonic time",
    );

    runtime.set_time(Local::now().fixed_offset(), 10);
    runtime
        .poll_deliveries()
        .expect("BUG: fixture delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("response_count"),
        Some(1),
        "delayed fetch did not fire after monotonic time reached its deadline",
    );
}

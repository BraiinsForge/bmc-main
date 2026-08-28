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

//! A trap inside a delivery callback has to reach the host.
//!
//! The trap unwinds the guest's frames without running their epilogues,
//! so `__stack_pointer` keeps whatever value it had when the trap fired.
//! A swallowed trap therefore leaves an instance that still answers calls
//! while permanently short of stack, and a few of them exhaust the 64 KiB
//! reservation outright. `poll_deliveries` reports the trap so the host
//! tears the slot down rather than driving it again.

#![cfg(all(target_os = "linux", feature = "testing"))]

use chrono::Local;

use bmc_wasm_runtime::{InterceptedReply, RuntimeConfig, RuntimeDisplayInfo, WasmWidgetRuntime};

fn trapping_callback_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://example.test/trap")

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        (drop
          (call $host_fetch_after
            (i32.const 0)     ;; delay_ms
            (i32.const 10000) ;; timeout_ms
            (i32.const 0)     ;; method_ptr
            (i32.const 3)     ;; method_len
            (i32.const 3)     ;; url_ptr
            (i32.const 25)    ;; url_len
            (i32.const 0)     ;; headers_ptr
            (i32.const 0)     ;; headers_len
            (i32.const 0)     ;; body_ptr
            (i32.const 0)))) ;; body_len

      (func (export "__on_fetch_response")
        (param $request_id i32)
        (param $status i32)
        (param $body_ptr i32)
        (param $body_len i32)
        unreachable)

      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

#[test]
fn delivery_callback_trap_is_reported_not_swallowed() {
    let wasm = wat::parse_str(trapping_callback_wat()).expect("BUG: trapping WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        RuntimeDisplayInfo {
            width: 64,
            height: 64,
            shape: bmc_wasm_protocol::DisplayShape::Rectangular,
            dpi: 1,
        },
        Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply::new(200, b"ok".to_vec()))
            })),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    runtime.set_time(Local::now().fixed_offset(), 1);
    let error = runtime
        .poll_deliveries()
        .expect_err("a trapping __on_fetch_response must be reported to the host");
    assert!(
        error.to_string().contains("__on_fetch_response"),
        "the reported trap should name the callback that took it, got: {error}"
    );
}

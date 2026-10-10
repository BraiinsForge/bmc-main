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

//! `host_http_listen` refuses a listener whose port cannot be bound.

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::net::TcpListener;

use chrono::Local;

use bmc_wasm_runtime::{
    RuntimeConfig, RuntimeDisplayInfo, RuntimeResourceLimits, WasmWidgetRuntime,
};

/// A guest whose `init` listens on `taken_port` and then on an ephemeral port,
/// keeping both listener ids.
fn listen_wat(taken_port: u16) -> String {
    format!(
        r#"
    (module
      (import "env" "host_http_listen" (func $listen (param i32) (result i32)))

      (memory (export "memory") 1)

      (global $on_taken_port (mut i32) (i32.const 0))
      (global $on_ephemeral_port (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {sdk_version})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        (global.set $on_taken_port (call $listen (i32.const {taken_port})))
        (global.set $on_ephemeral_port (call $listen (i32.const 0))))

      (func (export "render") (param i32))

      (func (export "on_taken_port") (result i32) (global.get $on_taken_port))
      (func (export "on_ephemeral_port") (result i32) (global.get $on_ephemeral_port)))
    "#,
        sdk_version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn runtime(wat: &str, max_http_listeners: usize) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(wat).expect("BUG: listener WAT must parse");
    let config = RuntimeConfig {
        resource_limits: RuntimeResourceLimits {
            max_http_listeners,
            ..RuntimeResourceLimits::default()
        },
        ..RuntimeConfig::default()
    };
    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_ui::ViewportShape::Rectangular,
        RuntimeDisplayInfo {
            width: 64,
            height: 64,
            shape: bmc_ui::DisplayShape::Rectangular,
            dpi: 1,
        },
        Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: runtime construct")
}

fn taken_port() -> (TcpListener, u16) {
    let holder = TcpListener::bind(("0.0.0.0", 0)).expect("BUG: an ephemeral port must bind");
    let port = holder
        .local_addr()
        .expect("BUG: a bound listener has an address")
        .port();
    (holder, port)
}

#[test]
fn a_listener_whose_port_is_taken_is_refused() {
    let (_holder, port) = taken_port();

    let mut runtime = runtime(&listen_wat(port), 2);

    assert_eq!(runtime.call_export_i32("on_taken_port"), Some(0));
}

#[test]
fn a_refused_listener_leaves_its_slot_free() {
    let (_holder, port) = taken_port();

    let mut runtime = runtime(&listen_wat(port), 1);

    assert!(
        runtime
            .call_export_i32("on_ephemeral_port")
            .is_some_and(|id| id != 0),
        "the only listener slot must still be free for the ephemeral port",
    );
}

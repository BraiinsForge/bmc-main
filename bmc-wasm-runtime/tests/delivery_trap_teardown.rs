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
//!
//! A full guest heap is not a trap: `__alloc` returning 0 settles the reply
//! as `BodyTooLarge`, and the widget keeps running.

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::io::Write as _;
use std::time::{Duration, Instant};

use chrono::Local;

use bmc_wasm_protocol::WebsocketId;
use bmc_wasm_runtime::{
    FixtureEvent, FixtureEventKind, InterceptedReply, RuntimeConfig, RuntimeDisplayInfo,
    WasmWidgetRuntime,
};

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
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

fn trapping_alloc_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://example.test/trap")
      (global $entries (export "entries") (mut i32) (i32.const 0))

      (func $entered
        (global.set $entries (i32.add (global.get $entries) (i32.const 1))))

      (func $fetch
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

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "__alloc") (param $len i32) (result i32)
        call $entered
        unreachable)

      (func (export "init")
        call $fetch
        call $fetch)

      (func (export "__on_fetch_response")
        (param $request_id i32)
        (param $status i32)
        (param $body_ptr i32)
        (param $body_len i32)
        call $entered)

      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

fn exhausted_heap_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_fetch_after"
        (func $host_fetch_after
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))
      (import "env" "host_fetch_header"
        (func $host_fetch_header (param i32 i32 i32 i32 i32) (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://example.test/trap")
      (data (i32.const 32) "content-type")
      (global $callbacks (mut i32) (i32.const 0))
      (global $status (mut i32) (i32.const 0))
      (global $body_len (mut i32) (i32.const -1))
      (global $content_type_len (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 0))

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
        (global.set $status (local.get $status))
        (global.set $body_len (local.get $body_len))
        (global.set $content_type_len
          (call $host_fetch_header
            (local.get $request_id)
            (i32.const 32)    ;; name_ptr
            (i32.const 12)    ;; name_len
            (i32.const 64)    ;; out_ptr
            (i32.const 32)))  ;; out_cap
        (global.set $callbacks (i32.add (global.get $callbacks) (i32.const 1))))

      (func (export "callbacks") (result i32)
        global.get $callbacks)

      (func (export "status") (result i32)
        global.get $status)

      (func (export "body_len") (result i32)
        global.get $body_len)

      (func (export "content_type_len") (result i32)
        global.get $content_type_len)

      (func (export "first_byte") (result i32)
        (i32.load8_u (i32.const 0)))

      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

fn spinning_timer_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_timer_start" (func $start (param i32) (result i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "init")
        (drop (call $start (i32.const 1000))))

      (func (export "__on_timer") (param $id i32) (param $elapsed i32)
        (loop $spin (br $spin)))

      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

/// Opens a websocket whose payloads `__alloc` cannot place without trapping.
fn trapping_ws_alloc_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_ws_connect"
        (func $host_ws_connect (param i32 i32 i32 i32) (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "ws://127.0.0.1:9/")
      (global $entries (export "entries") (mut i32) (i32.const 0))

      (func $entered
        (global.set $entries (i32.add (global.get $entries) (i32.const 1))))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "__alloc") (param $len i32) (result i32)
        call $entered
        unreachable)

      (func (export "init")
        (drop
          (call $host_ws_connect
            (i32.const 0)    ;; url_ptr
            (i32.const 17)   ;; url_len
            (i32.const 0)    ;; headers_ptr
            (i32.const 0)))) ;; headers_len

      (func (export "__on_ws_event")
        (param $ws_id i32)
        (param $kind i32)
        (param $data_ptr i32)
        (param $data_len i32)
        call $entered)

      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

/// A websocket payload `__alloc` cannot place, and a timer due in the same pass.
fn trapping_ws_alloc_with_timer_wat() -> String {
    let wat = trapping_ws_alloc_wat()
        .replacen(
            "(func $host_ws_connect (param i32 i32 i32 i32) (result i32)))",
            "(func $host_ws_connect (param i32 i32 i32 i32) (result i32)))\n      (import \"env\" \"host_timer_start\" (func $start (param i32) (result i32)))",
            1,
        )
        .replacen(
            "(func (export \"init\")",
            "(func (export \"init\")\n        (drop (call $start (i32.const 1000)))",
            1,
        )
        .replacen(
            "(func (export \"render\")",
            "(func (export \"__on_timer\") (param i32 i32)\n        call $entered)\n\n      (func (export \"render\")",
            1,
        );
    assert!(
        wat.contains("__on_timer") && wat.contains("host_timer_start"),
        "BUG: the websocket fixture changed shape; the timer was not spliced in"
    );
    wat
}

/// Listens on an ephemeral port, exported as `port`,
/// for requests whose method `__alloc` cannot place without trapping.
fn trapping_http_alloc_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_http_listen" (func $listen (param i32) (result i32)))
      (import "env" "host_http_get_port" (func $get_port (param i32) (result i32)))

      (memory (export "memory") 1)
      (global $entries (export "entries") (mut i32) (i32.const 0))
      (global $port (export "port") (mut i32) (i32.const 0))

      (func $entered
        (global.set $entries (i32.add (global.get $entries) (i32.const 1))))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {version})

      (func (export "__alloc") (param $len i32) (result i32)
        call $entered
        unreachable)

      (func (export "init")
        (global.set $port (call $get_port (call $listen (i32.const 0)))))

      (func (export "__on_http_request")
        (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)
        call $entered)

      (func (export "render") (param i32)))
    "#,
        version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::PROTOCOL_VERSION)
    )
}

fn runtime(wat: &str, config: RuntimeConfig) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(wat).expect("BUG: fixture WAT must parse");
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

#[test]
fn delivery_callback_trap_is_reported_not_swallowed() {
    let mut runtime = runtime(
        &trapping_callback_wat(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply::new(200, b"ok".to_vec()))
            })),
            ..RuntimeConfig::default()
        },
    );

    runtime.set_time(Local::now().fixed_offset(), 1);
    let error = runtime
        .poll_deliveries()
        .expect_err("a trapping __on_fetch_response must be reported to the host");
    assert!(
        error.to_string().contains("__on_fetch_response"),
        "the reported trap should name the callback that took it, got: {error}"
    );
}

#[test]
fn a_trap_in_the_allocator_ends_the_widget_like_one_in_a_callback() {
    let mut runtime = runtime(
        &trapping_alloc_wat(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply::new(200, b"ok".to_vec()))
            })),
            ..RuntimeConfig::default()
        },
    );

    runtime.set_time(Local::now().fixed_offset(), 1);
    let error = runtime
        .poll_deliveries()
        .expect_err("a trapping __alloc must be reported to the host");

    assert!(
        error.to_string().contains("__alloc"),
        "the reported trap should name the export that took it, got: {error}"
    );
    assert_eq!(
        runtime.exported_global_i32("entries"),
        Some(1),
        "neither the second response's __alloc nor any callback may run after the trap"
    );
}

#[test]
fn a_trap_in_the_allocator_stops_the_payloads_queued_behind_it() {
    let ws_id = WebsocketId::from_wire(1).expect("BUG: the first socket's id is 1");
    let mut runtime = runtime(
        &trapping_ws_alloc_wat(),
        RuntimeConfig {
            event_fixtures: ["first", "second"]
                .into_iter()
                .map(|text| FixtureEvent {
                    at_ms: 0,
                    kind: FixtureEventKind::WsMessage {
                        ws_id,
                        data: text.as_bytes().to_vec(),
                    },
                })
                .collect(),
            ..RuntimeConfig::default()
        },
    );

    runtime.inject_fixture_events(0);
    runtime
        .poll_deliveries()
        .expect_err("a trapping __alloc must be reported to the host");

    assert_eq!(
        runtime.exported_global_i32("entries"),
        Some(1),
        "the second message must not reach __alloc on the instance the first one trapped"
    );
}

#[test]
fn a_trap_in_one_channel_keeps_the_later_channels_of_the_pass_from_the_guest() {
    let ws_id = WebsocketId::from_wire(1).expect("BUG: the first socket's id is 1");
    let mut runtime = runtime(
        &trapping_ws_alloc_with_timer_wat(),
        RuntimeConfig {
            event_fixtures: vec![FixtureEvent {
                at_ms: 1_000,
                kind: FixtureEventKind::WsMessage {
                    ws_id,
                    data: b"payload".to_vec(),
                },
            }],
            ..RuntimeConfig::default()
        },
    );
    runtime.set_time(Local::now().fixed_offset(), 0);
    runtime
        .poll_deliveries()
        .expect("BUG: neither the message nor the tick is due yet");

    runtime.set_time(Local::now().fixed_offset(), 1_000);
    runtime.inject_fixture_events(1_000);
    runtime
        .poll_deliveries()
        .expect_err("a trapping __alloc must be reported to the host");

    assert_eq!(
        runtime.exported_global_i32("entries"),
        Some(1),
        "the timer is delivered after websockets, so its tick must not reach the trapped instance"
    );
}

#[test]
fn a_trap_placing_a_request_keeps_the_rest_of_it_from_the_guest() {
    let mut runtime = runtime(&trapping_http_alloc_wat(), RuntimeConfig::default());
    let port = runtime
        .exported_global_i32("port")
        .and_then(|port| u16::try_from(port).ok())
        .filter(|&port| port != 0)
        .expect("BUG: the widget must listen on an ephemeral port");
    let mut client =
        std::net::TcpStream::connect(("127.0.0.1", port)).expect("BUG: the widget must listen");
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: widget\r\n\r\n")
        .expect("BUG: the listener must accept the request");

    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.poll_deliveries().is_ok() {
        assert!(
            Instant::now() < deadline,
            "the request never reached the widget"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        runtime.exported_global_i32("entries"),
        Some(1),
        "after the method's __alloc traps, neither the other parts' __alloc nor __on_http_request may run"
    );
}

#[test]
fn a_reply_the_guest_heap_cannot_hold_settles_as_too_large_and_the_widget_lives() {
    let mut runtime = runtime(
        &exhausted_heap_wat(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply::new(200, b"ok".to_vec()))
            })),
            ..RuntimeConfig::default()
        },
    );

    runtime.set_time(Local::now().fixed_offset(), 1);
    runtime
        .poll_deliveries()
        .expect("an exhausted heap costs the body, not the widget");

    assert_eq!(
        runtime.call_export_i32("first_byte"),
        Some(i32::from(b'G')),
        "0 from __alloc is no pointer, so nothing may be written to guest address 0"
    );
    assert_eq!(
        runtime.call_export_i32("callbacks"),
        Some(1),
        "the request must still settle, or the widget waits on it for good"
    );
    assert_eq!(
        runtime.call_export_i32("body_len"),
        Some(0),
        "a body that was never placed must not reach the callback"
    );
    assert_eq!(
        runtime
            .call_export_i32("status")
            .and_then(|status| u32::try_from(status).ok())
            .and_then(bmc_wasm_protocol::FetchOutcome::from_wire),
        Some(bmc_wasm_protocol::FetchOutcome::BodyTooLarge),
        "the origin's 200 must not read as success with its body gone"
    );
}

#[test]
fn a_reply_the_guest_heap_cannot_hold_shows_none_of_the_origins_headers() {
    let mut runtime = runtime(
        &exhausted_heap_wat(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply {
                    status: 200,
                    headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
                    body: b"{}".to_vec(),
                })
            })),
            ..RuntimeConfig::default()
        },
    );

    runtime.set_time(Local::now().fixed_offset(), 1);
    runtime
        .poll_deliveries()
        .expect("an exhausted heap costs the body, not the widget");

    assert_eq!(
        runtime.call_export_i32("content_type_len"),
        Some(-1),
        "a widget told its reply was too large must not read the absent body by the origin's content type"
    );
}

#[test]
fn an_empty_reply_reaches_a_widget_without_an_allocator_as_sent() {
    let wat = exhausted_heap_wat().replace(
        r#"(func (export "__alloc") (param $len i32) (result i32)
        (i32.const 0))"#,
        "",
    );
    assert!(!wat.contains("__alloc"), "the fixture must drop __alloc");
    let mut runtime = runtime(
        &wat,
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| {
                Some(InterceptedReply::new(204, Vec::new()))
            })),
            ..RuntimeConfig::default()
        },
    );

    runtime.set_time(Local::now().fixed_offset(), 1);
    runtime
        .poll_deliveries()
        .expect("an empty reply needs no allocation");

    assert_eq!(
        runtime.call_export_i32("status"),
        Some(204),
        "an empty body has nothing to place, so it cannot be too large for the heap"
    );
}

#[test]
fn a_timer_tick_out_of_fuel_is_reported_not_swallowed() {
    let mut runtime = runtime(
        &spinning_timer_wat(),
        RuntimeConfig {
            fuel_per_frame: 10_000,
            ..RuntimeConfig::default()
        },
    );
    runtime.set_time(Local::now().fixed_offset(), 0);
    runtime
        .poll_deliveries()
        .expect("BUG: no tick is due before the first period ends");

    runtime.set_time(Local::now().fixed_offset(), 1_000);
    let error = runtime
        .poll_deliveries()
        .expect_err("a tick that runs out of fuel must be reported to the host");

    assert!(
        error.to_string().contains("__on_timer"),
        "the reported trap should name the callback that took it, got: {error}"
    );
}

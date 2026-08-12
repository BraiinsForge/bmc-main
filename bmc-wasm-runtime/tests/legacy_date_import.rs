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

//! Widgets from SDK 0.5 and earlier import the instant parser as `host_parse_date`
//! and decode a 20-byte layout from `host_tz_convert`. Both are pinned here.

use bmc_wasm_protocol::{DisplayShape, ViewportShape};
use bmc_wasm_runtime::{RuntimeConfig, RuntimeDisplayInfo, WasmWidgetRuntime};

/// The version the fixture widget declares — the last one before the rename.
const LEGACY_SDK_VERSION: (u16, u16, u16) = (0, 5, 0);

/// `2026-08-21T10:30:00Z` as a unix timestamp.
const KNOWN_INSTANT: i64 = 1_787_308_200;

/// Minimal widget importing the parser under its retired name.
fn legacy_date_probe_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_parse_date"
        (func $parse_date (param i32 i32) (result i64)))

      (memory (export "memory") 1)
      (data (i32.const 0) "2026-08-21T10:30:00Z")

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {sdk})

      (func (export "render") (param i32))

      (func (export "parses_a_known_instant") (result i32)
        i32.const 0
        i32.const 20
        call $parse_date
        i64.const {instant}
        i64.eq))
    "#,
        sdk = bmc_wasm_protocol::version_pack(LEGACY_SDK_VERSION),
        instant = KNOWN_INSTANT,
    )
}

#[test]
fn the_retired_date_import_still_parses_an_instant() {
    let wasm = wat::parse_str(legacy_date_probe_wat()).expect("BUG: probe WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        480,
        480,
        ViewportShape::Rectangular,
        RuntimeDisplayInfo {
            width: 480,
            height: 480,
            shape: DisplayShape::Rectangular,
            dpi: 220,
        },
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: a widget importing host_parse_date must still instantiate");

    assert_eq!(
        runtime.call_export_i32("parses_a_known_instant"),
        Some(1),
        "host_parse_date must reach the same parser as host_parse_datetime"
    );
}

/// Minimal widget decoding `host_tz_convert` the way an SDK 0.5 binary did.
fn legacy_tz_probe_wat() -> String {
    // `KNOWN_INSTANT` in Europe/Prague: 12:30 CEST, UTC+02:00, a Friday.
    format!(
        r#"
    (module
      (import "env" "host_tz_convert"
        (func $tz_convert (param i64 i32 i32 i32) (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "Europe/Prague")

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {sdk})

      (func (export "render") (param i32))

      (func (export "decodes_the_legacy_layout") (result i32)
        (local $rc i32)
        i64.const {instant}
        i32.const 0
        i32.const 13
        i32.const 32
        call $tz_convert
        local.set $rc

        (i32.eqz (local.get $rc))
        (i64.eq (i64.load (i32.const 32)) (i64.const {instant}))
        i32.and
        (i32.eq (i32.load (i32.const 40)) (i32.const 7200))
        i32.and
        (i32.eq (i32.load16_u (i32.const 44)) (i32.const 2026))
        i32.and
        (i32.eq (i32.load8_u (i32.const 46)) (i32.const 8))
        i32.and
        (i32.eq (i32.load8_u (i32.const 47)) (i32.const 21))
        i32.and
        (i32.eq (i32.load8_u (i32.const 48)) (i32.const 12))
        i32.and
        (i32.eq (i32.load8_u (i32.const 49)) (i32.const 30))
        i32.and
        (i32.eq (i32.load8_u (i32.const 50)) (i32.const 0))
        i32.and
        (i32.eq (i32.load8_u (i32.const 51)) (i32.const 4))
        i32.and))
    "#,
        sdk = bmc_wasm_protocol::version_pack(LEGACY_SDK_VERSION),
        instant = KNOWN_INSTANT,
    )
}

#[test]
fn the_legacy_tz_layout_keeps_its_bytes() {
    let wasm = wat::parse_str(legacy_tz_probe_wat()).expect("BUG: probe WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        480,
        480,
        ViewportShape::Rectangular,
        RuntimeDisplayInfo {
            width: 480,
            height: 480,
            shape: DisplayShape::Rectangular,
            dpi: 220,
        },
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: a widget importing host_tz_convert must still instantiate");

    assert_eq!(
        runtime.call_export_i32("decodes_the_legacy_layout"),
        Some(1),
        "a 0.5 binary must keep decoding the 20-byte host_tz_convert answer"
    );
}

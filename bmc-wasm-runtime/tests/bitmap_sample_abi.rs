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

//! Bitmap sampling is gone, but widgets built against SDK 0.2.x still import
//! `host_bitmap_sample`.
//! The runtime keeps an inert function under that name so those binaries load;
//! this test pins both halves of the contract.

use bmc_wasm_protocol::{DisplayShape, ViewportShape};
use bmc_wasm_runtime::{RuntimeConfig, RuntimeDisplayInfo, WasmWidgetRuntime};

/// Pinned rather than read from `SDK_VERSION`: the fixture is a widget built
/// before the removal and has to stay one as the host version moves on.
const LEGACY_SDK_VERSION: (u16, u16, u16) = (0, 2, 0);

/// Minimal 0.2.x widget importing the retired sampling function.
///
/// The sampled ID is a valid one, so a reinstated implementation would reach
/// for the renderer instead of bailing out early — that is what makes the
/// assertion below fail if sampling ever comes back under this name.
fn bitmap_sample_probe_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_bitmap_sample"
        (func $bitmap_sample (param i32 i32 i32 i32 i32) (result i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      (func (export "sample_registered_bitmap") (result i32)
        i32.const 1
        i32.const 0
        i32.const 0
        i32.const 16
        i32.const 16
        call $bitmap_sample))
    "#,
        bmc_wasm_protocol::version_pack(LEGACY_SDK_VERSION)
    )
}

#[test]
fn legacy_bitmap_sample_import_instantiates_and_returns_no_colour() {
    let wasm = wat::parse_str(bitmap_sample_probe_wat()).expect("BUG: probe WAT must parse");
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
    .expect("BUG: a widget importing host_bitmap_sample must still instantiate");

    // Called outside a render scope, so anything reaching for the renderer
    // traps — and `call_export_i32` reports a trap as `None`.
    assert_eq!(
        runtime.call_export_i32("sample_registered_bitmap"),
        Some(0),
        "retired host_bitmap_sample must return 0 without touching the renderer"
    );
}

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

//! The geometry host imports (`host_widget_viewport_shape`, `host_display_size`,
//! `host_display_shape_dpi`) are visible to the guest with the values the
//! runtime was constructed with. Wayland initial-state event coverage is Stage
//! 3's (verified in Task 1); this is the host-import boundary only.

use bmc_wasm_protocol::{DisplayShape, ViewportShape};
use bmc_wasm_runtime::{RuntimeConfig, RuntimeDisplayInfo, WasmWidgetRuntime};

/// Minimal widget that re-exports the geometry imports.
fn geometry_probe_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_widget_viewport_shape" (func $vp_shape (result i32)))
      (import "env" "host_display_size" (func $disp_size (result i64)))
      (import "env" "host_display_shape_dpi" (func $disp_shape_dpi (result i64)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      (func (export "read_viewport_shape") (result i32)
        call $vp_shape)

      ;; high 32 bits of the packed display size = width
      (func (export "read_display_width") (result i32)
        call $disp_size
        i64.const 32
        i64.shr_u
        i32.wrap_i64)

      ;; low 32 bits of the packed shape/dpi = dpi
      (func (export "read_display_dpi") (result i32)
        call $disp_shape_dpi
        i32.wrap_i64))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

#[test]
fn geometry_imports_reach_the_guest() {
    let wasm = wat::parse_str(geometry_probe_wat()).expect("BUG: probe WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        480,
        480,
        ViewportShape::Round,
        RuntimeDisplayInfo {
            width: 480,
            height: 480,
            shape: DisplayShape::Round,
            dpi: 220,
        },
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct");

    // ViewportShape::Round has wire value 1.
    assert_eq!(runtime.call_export_i32("read_viewport_shape"), Some(1));
    assert_eq!(runtime.call_export_i32("read_display_width"), Some(480));
    assert_eq!(runtime.call_export_i32("read_display_dpi"), Some(220));
}

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

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::time::{Duration, Instant};

use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

fn probe_wat() -> String {
    format!(
        r#"
    (module
      (memory (export "memory") 1)
      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})
      (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

#[test]
fn two_runtimes_one_process_see_each_others_mdns_announcements() {
    let gl = headless_egl::try_init(256, 256).expect(
        "BUG: headless EGL initialization required to run this test; \
                 fail loud rather than silent-pass on environments lacking EGL",
    );
    let _force_use = (&gl.display, gl.fbo_id, gl.proc_address());
    let wasm = wat::parse_str(probe_wat()).expect("BUG: probe WAT must parse");

    let mut a = WasmWidgetRuntime::new(
        &wasm,
        256,
        256,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(256, 256),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime A must construct");
    let mut b = WasmWidgetRuntime::new(
        &wasm,
        256,
        256,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(256, 256),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime B must construct");

    // Start browse + registration through the runtime path, not by talking to mdns-sd directly.
    a.test_start_mdns_browse("_bdk469-test._tcp.local.");
    b.test_start_mdns_browse("_bdk469-test._tcp.local.");
    a.test_register_mdns(
        "_bdk469-test._tcp.local.",
        "instance-a",
        "host-a.local.",
        "127.0.0.1",
        12001,
    );
    b.test_register_mdns(
        "_bdk469-test._tcp.local.",
        "instance-b",
        "host-b.local.",
        "127.0.0.1",
        12002,
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_b_on_a = false;
    let mut saw_a_on_b = false;
    while Instant::now() < deadline && (!saw_b_on_a || !saw_a_on_b) {
        a.poll_deliveries()
            .expect("BUG: fixture delivery must not trap");
        b.poll_deliveries()
            .expect("BUG: fixture delivery must not trap");

        for event in a.test_take_mdns_events() {
            if event.fullname.contains("instance-b") {
                saw_b_on_a = true;
            }
        }
        for event in b.test_take_mdns_events() {
            if event.fullname.contains("instance-a") {
                saw_a_on_b = true;
            }
        }

        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(saw_b_on_a, "runtime A did not receive instance-b within 2s");
    assert!(saw_a_on_b, "runtime B did not receive instance-a within 2s");
}

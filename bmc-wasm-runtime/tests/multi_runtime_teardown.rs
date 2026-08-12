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

//! Teardown-bound coverage. Stop-channel-aware workers (mdns, ws_connect, ssdp, udp, http)
//! are required to exit within ~200 ms of Drop. Workers that block on external I/O
//! without a stop channel — fetch threads spawned from `delivery::fire_ready_delayed_fetches`
//! — are NOT covered here; their exit latency is bounded only by the I/O timeout of their
//! inner call.

use std::thread;
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
fn stop_channel_workers_join_within_200ms_on_drop() {
    // Fail loud on missing EGL rather than silently passing — a CI runner without
    // a usable EGL stack would otherwise green-light a test that asserts nothing.
    // Skip explicitly is fine; silent pass is not (feedback_verify_before_claiming).
    let gl = headless_egl::try_init(256, 256).expect(
        "BUG: headless EGL initialization required to run this test; \
                 if the CI environment lacks EGL, gate the entire crate's test set \
                 behind `--ignored` or a Cargo feature rather than silently passing",
    );
    // Touch every `pub` item in `tests/common/mod.rs` exactly once: each
    // integration test binary is compiled independently and rejects unused
    // `pub` items, but `#[allow(dead_code)]` is forbidden workspace-wide.
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

    // Spawn the workers we will observe through drop. Every kick helper returns a
    // JoinHandle for a thread that is *actually blocked* on a live receiver — not the
    // dummy `_stop_rx` pattern that would orphan the receiver before drop ever runs.
    let mut a_workers: Vec<std::thread::JoinHandle<()>> = vec![
        a.test_kick_fetch(),
        a.test_kick_ws_connect(),
        a.test_kick_mdns_browse(),
    ];
    // The sibling runtime gets its own workers so its delivery channels stay warm.
    let _b_warmup_workers: Vec<std::thread::JoinHandle<()>> =
        vec![b.test_kick_ws_connect(), b.test_kick_mdns_browse()];

    assert!(
        a_workers.iter().all(|h| !h.is_finished()),
        "fixture invariant: all `a` workers must still be running before drop",
    );

    drop(a);

    // (1) Bounded-window join: every worker `a` spawned must exit within 200 ms.
    //     Polling is per-worker so a slow exit on one thread does not hide a
    //     completed-but-untaken handle on another.
    let deadline = Instant::now() + Duration::from_millis(200);
    while !a_workers.is_empty() && Instant::now() < deadline {
        let still_running: Vec<_> = a_workers
            .drain(..)
            .filter_map(|h| {
                if h.is_finished() {
                    h.join()
                        .expect("BUG: worker thread panicked during shutdown");
                    None
                } else {
                    Some(h)
                }
            })
            .collect();
        a_workers = still_running;
        if !a_workers.is_empty() {
            thread::sleep(Duration::from_millis(2));
        }
    }
    assert!(
        a_workers.is_empty(),
        "{} worker thread(s) did not exit within 200 ms after runtime A was dropped",
        a_workers.len(),
    );

    // (2) Sibling forward progress: kick a post-drop fetch on `b` and verify the
    //     `delivered_events` counter advances. The counter is incremented inside
    //     each `deliver_*` method's drain loop (Step 5), so observing an advance
    //     proves `poll_deliveries()` actually drained something — i.e. the
    //     sibling's I/O machinery is alive after runtime A's teardown.
    let baseline = b.test_progress_counter();
    let _b_post_drop_worker = b.test_kick_fetch();

    let progress_deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < progress_deadline && b.test_progress_counter() == baseline {
        b.poll_deliveries()
            .expect("BUG: fixture delivery must not trap");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        b.test_progress_counter() > baseline,
        "sibling runtime made no observable progress after runtime A was dropped",
    );

    // Drop b last so the warmup workers and the post-drop worker exit cleanly via
    // channel disconnect. The test does not join them — their JoinHandles are
    // detached by the `_` bindings — but the OS reaps them before process exit.
    drop(b);
}

#[test]
#[ignore = "documents an unbounded teardown path; not run in default suite"]
fn fetch_thread_teardown_is_not_bounded_by_drop() {}

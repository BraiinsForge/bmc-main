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

//! Integration tests for the params/lifecycle wire (Stage E of BDK-432).
//!
//! These tests construct a real `WasmWidgetRuntime` with a hand-rolled WAT probe widget
//! and assert the params/lifecycle contract:
//!
//! * `on_params_update` does NOT fire for the initial delivery via `RuntimeConfig::params`.
//! * `on_params_update` DOES fire for every subsequent `deliver_params_update` call.
//! * The host bumps the version counter on every install; consecutive bumps produce distinct
//!   values (the "different = changed" contract — wrapping is OK, but distinct values must hold
//!   for tests that perform a small number of pushes).
//! * Inside `on_params_update`, the snapshot the guest fetches via `host_params_snapshot`
//!   reflects the just-pushed table (not the previous one).
//! * The lifecycle guard for `host_submit_tree` traps when called from `on_params_update`.
//!
//! ## Headless GL
//!
//! Tests that construct a FemtoVG renderer need a current EGL/GL context.
//! The `ci` build profile (`nix/profiles.nix`) supplies Mesa, llvmpipe
//! and surfaceless EGL inside the Nix sandbox.
//! Locally without Nix the EGL init will fail, and renderer-backed tests
//! then skip with a clear log line rather than spuriously failing.
//! Renderer-free tests do not enter that skip path, so runtime-only
//! contracts still execute without EGL.
//!
//! The expectation is that CI (which runs through the `ci`
//! profile) is the authoritative environment for renderer-backed tests.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::{AssetTagState, Renderer};
use bmc_wasm_protocol::system::{
    DateFormat, NumberFormat, TemperatureUnit, TimeFormat, UnitSystem, Weekday,
};
use bmc_wasm_protocol::{PackageAssetId, PackageAssetKind, PackageAssetRef, SvgId};
use bmc_wasm_runtime::{
    CredentialView, DiskCache, NextAlarm, PackageAssetStore, RenderStatus, RuntimeConfig,
    SystemSettings, SystemSnapshot, WasmWidgetRuntime,
};
use bmc_widget_manifest::{ParamKey, ParamValue};
use bmc_widget_protocol::CredentialSecrets;

#[path = "common/asset_fixtures.rs"]
mod asset_fixtures;
mod common;
use asset_fixtures::{compiled_empty_svg, one_px_png, renderer_ptr, wat_string_literal};
use common::headless_egl;

fn poll_renderer_deliveries(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
) -> bool {
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(renderer))
        .expect("BUG: lifecycle delivery must not trap")
}

fn poll_renderer_deliveries_expect_trap(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    expected_hook: &str,
) {
    let error = runtime
        .poll_deliveries_with_renderer(renderer_ptr(renderer))
        .expect_err("the fixture lifecycle hook must trap after recording its observations");
    assert!(
        error.to_string().contains(expected_hook),
        "the propagated trap must name {expected_hook}, got: {error}"
    );
}

// ── Probe widget fixtures (hand-rolled WAT) ─────────────────────────

/// Test widget that counts lifecycle invocations and records observations
/// on each `on_params_update` call. Exports observation getters used by the tests.
///
/// The reported SDK version is derived from `bmc_wasm_protocol::SDK_VERSION`,
/// so a version bump needs no edit here — only the pinned literal in
/// `sdk_version_constant_matches_fixture_assumption`.
fn probe_widget_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_params_version" (func $host_params_version (result i64)))
      (import "env" "host_params_snapshot"
        (func $host_params_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (global $update_count (mut i32) (i32.const 0))
      (global $render_count (mut i32) (i32.const 0))
      (global $init_count (mut i32) (i32.const 0))
      (global $last_version_in_update (mut i64) (i64.const 0))
      (global $last_snapshot_len_in_update (mut i32) (i32.const 0))

      ;; Required by the host. Returns the packed `bmc_wasm_protocol::SDK_VERSION`.
      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      ;; Required by the host. Body is intentionally trivial — these tests don't render.
      (func (export "render") (param i32))

      ;; Optional. Counts calls so the test can assert init runs exactly once.
      (func (export "init")
        global.get $init_count
        i32.const 1
        i32.add
        global.set $init_count)

      ;; Optional. Counts calls; records the version + snapshot length observed at call time.
      ;; The probe fetches the snapshot into the very start of guest memory (offset 0); these
      ;; tests don't read the snapshot bytes, only the length the host reports.
      (func (export "on_params_update")
        global.get $update_count
        i32.const 1
        i32.add
        global.set $update_count

        call $host_params_version
        global.set $last_version_in_update

        i32.const 0      ;; out_ptr
        i32.const 4096   ;; out_cap — plenty for the small test snapshots
        call $host_params_snapshot
        global.set $last_snapshot_len_in_update)

      (func (export "init_count") (result i32) global.get $init_count)
      (func (export "render_count") (result i32) global.get $render_count)
      (func (export "update_count") (result i32) global.get $update_count)
      (func (export "last_version_in_update") (result i64)
        global.get $last_version_in_update)
      (func (export "last_snapshot_len_in_update") (result i32)
        global.get $last_snapshot_len_in_update))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Misbehaving widget: calls `host_params_snapshot` with an out-of-bounds `out_ptr`.
/// Used to assert the host traps the ABI violation rather than fail-quiet returning 0.
fn oob_snapshot_traps_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_params_snapshot"
        (func $host_params_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      ;; out_ptr = u32::MAX (well past the guest's single 64 KiB page).
      ;; out_cap large enough that the host's `out_cap < needed` early-return doesn't fire
      ;; and the function falls into the memory-write branch.
      (func (export "on_params_update")
        i32.const -1     ;; out_ptr = u32::MAX in two's complement
        i32.const 4096   ;; out_cap
        call $host_params_snapshot
        drop))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Probe widget for the system snapshot channel.
/// Imports `host_system_version` + `host_system_snapshot`
/// and records the version and snapshot length it observes
/// inside `on_params_update` (the unified hook — system updates
/// fire the same export as params updates).
fn system_probe_widget_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_system_version" (func $host_system_version (result i64)))
      (import "env" "host_system_snapshot"
        (func $host_system_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (global $update_count (mut i32) (i32.const 0))
      (global $last_system_version (mut i64) (i64.const 0))
      (global $last_system_snapshot_len (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      (func (export "on_system_update")
        global.get $update_count
        i32.const 1
        i32.add
        global.set $update_count

        call $host_system_version
        global.set $last_system_version

        i32.const 0      ;; out_ptr
        i32.const 4096   ;; out_cap — plenty for the system snapshot
        call $host_system_snapshot
        global.set $last_system_snapshot_len)

      (func (export "update_count") (result i32) global.get $update_count)
      (func (export "last_system_version") (result i64)
        global.get $last_system_version)
      (func (export "last_system_snapshot_len") (result i32)
        global.get $last_system_snapshot_len)

      ;; Lets the test read the initial version/snapshot length directly,
      ;; even when the widget didn't export `on_system_update` for the
      ;; initial delivery (initial deliveries don't fire the hook).
      (func (export "probe_system_version") (result i64)
        call $host_system_version)
      (func (export "probe_system_snapshot_len") (result i32)
        i32.const 0
        i32.const 4096
        call $host_system_snapshot))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Widget exporting both `on_params_update` and `on_system_update`
/// with independent counters, for asserting channel isolation.
fn dual_probe_widget_wat() -> String {
    format!(
        r#"
    (module
      (memory (export "memory") 1)

      (global $params_count (mut i32) (i32.const 0))
      (global $system_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      (func (export "on_params_update")
        global.get $params_count
        i32.const 1
        i32.add
        global.set $params_count)

      (func (export "on_system_update")
        global.get $system_count
        i32.const 1
        i32.add
        global.set $system_count)

      (func (export "params_count") (result i32) global.get $params_count)
      (func (export "system_count") (result i32) global.get $system_count))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Misbehaving widget calling `host_system_snapshot`
/// with an out-of-bounds `out_ptr`.
/// Mirrors `oob_snapshot_traps_wat` for the system channel.
fn oob_system_snapshot_traps_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_system_snapshot"
        (func $host_system_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      (func (export "on_system_update")
        i32.const -1     ;; out_ptr = u32::MAX in two's complement
        i32.const 4096   ;; out_cap
        call $host_system_snapshot
        drop))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Misbehaving widget: calls `host_submit_tree` from `on_params_update`.
/// Used to assert the lifecycle guard traps the call.
fn misbehaving_submit_tree_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_submit_tree"
        (func $host_submit_tree (param i32 i32 i32 i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      ;; Calls host_submit_tree with a zero-length tree. The guard should trap before the
      ;; runtime parses any bytes, so the call never actually mutates renderer state.
      (func (export "on_params_update")
        i32.const 0
        i32.const 0
        i32.const 0
        i32.const 0
        call $host_submit_tree))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Probe widget for the touch channel. Exports `on_touch`, which bumps a
/// counter and calls `host_request_frame` — the canonical "re-render in
/// response to touch" response. The counter getter lets the test assert the
/// hook fired exactly once.
fn touch_probe_widget_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_request_frame" (func $host_request_frame))

      (memory (export "memory") 1)

      (global $touch_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "render") (param i32))

      (func (export "on_touch")
        global.get $touch_count
        i32.const 1
        i32.add
        global.set $touch_count
        call $host_request_frame)

      (func (export "touch_count") (result i32) global.get $touch_count))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

/// Probe widget for the network channel.
/// Exports `on_network_update`, which bumps a counter and calls
/// `host_request_frame` — the "visible on my screen, repaint" response.
/// The counter getter lets the test assert the hook fired exactly once.
fn network_probe_widget_wat() -> String {
    include_str!("fixtures/network_probe.wat").replace(
        "__SDK_VERSION__",
        &bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION).to_string(),
    )
}

const LIFECYCLE_ASSET_TAG: &str = "static";
const DORMANT_IMAGE_DECODE_TAG: &str = "late-image";
const IMAGE_DECODE_COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

fn sleep_asset_probe_wat(on_sleep_export: &str) -> String {
    let svg = compiled_empty_svg();
    let svg_ptr = LIFECYCLE_ASSET_TAG.len();
    let mut data = LIFECYCLE_ASSET_TAG.as_bytes().to_vec();
    data.extend_from_slice(&svg);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_svg"
            (func $register_svg (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $asset_id (mut i32) (i32.const 0))
          (global $sleep_count (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $register_svg
            global.set $asset_id)
          {on_sleep_export}
          (func (export "asset_id") (result i32) global.get $asset_id)
          (func (export "sleep_count") (result i32) global.get $sleep_count))
        "#,
        tag_len = LIFECYCLE_ASSET_TAG.len(),
        svg_len = svg.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn deferred_sleep_then_wake_probe_wat() -> String {
    let svg = compiled_empty_svg();
    let svg_ptr = LIFECYCLE_ASSET_TAG.len();
    let mut data = LIFECYCLE_ASSET_TAG.as_bytes().to_vec();
    data.extend_from_slice(&svg);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_svg"
            (func $register_svg (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $asset_id (mut i32) (i32.const 0))
          (global $event_order (mut i32) (i32.const 0))
          (global $sleep_registration_id (mut i32) (i32.const 0))
          (global $wake_probe_id (mut i32) (i32.const 0))
          (global $wake_restore_id (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $register_svg
            global.set $asset_id)
          (func (export "on_sleep")
            global.get $event_order
            i32.const 10
            i32.mul
            i32.const 1
            i32.add
            global.set $event_order
            i32.const 0
            i32.const {tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $register_svg
            global.set $sleep_registration_id)
          (func (export "on_wake")
            global.get $event_order
            i32.const 10
            i32.mul
            i32.const 2
            i32.add
            global.set $event_order
            i32.const 0
            i32.const {tag_len}
            i32.const -1
            i32.const 1
            call $register_svg
            global.set $wake_probe_id
            i32.const 0
            i32.const {tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $register_svg
            global.set $wake_restore_id)
          (func (export "asset_id") (result i32) global.get $asset_id)
          (func (export "event_order") (result i32) global.get $event_order)
          (func (export "reset_event_order") (result i32)
            i32.const 0
            global.set $event_order
            i32.const 0)
          (func (export "sleep_registration_id") (result i32)
            global.get $sleep_registration_id)
          (func (export "wake_probe_id") (result i32) global.get $wake_probe_id)
          (func (export "wake_restore_id") (result i32) global.get $wake_restore_id))
        "#,
        tag_len = LIFECYCLE_ASSET_TAG.len(),
        svg_len = svg.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn coalesced_package_registration_wat(id: &[u8; 32]) -> String {
    let reference_ptr = LIFECYCLE_ASSET_TAG.len();
    let mut data = LIFECYCLE_ASSET_TAG.as_bytes().to_vec();
    let package_ref = PackageAssetRef::new(PackageAssetKind::Svg, PackageAssetId::from_bytes(*id));
    data.extend_from_slice(package_ref.as_bytes());
    let tree = active_asset_tree_bytes();
    let tree_ptr = data.len();
    data.extend_from_slice(&tree);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_svg_package"
            (func $register_svg (param i32 i32 i32) (result i32)))
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $sleep_registration_id (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const {tree_ptr}
            i32.const {tree_len}
            i32.const 320
            i32.const 240
            call $submit_tree)
          (func (export "on_sleep")
            i32.const 0
            i32.const {tag_len}
            i32.const {reference_ptr}
            call $register_svg
            global.set $sleep_registration_id)
          (func (export "on_wake"))
          (func (export "sleep_registration_id") (result i32)
            global.get $sleep_registration_id))
        "#,
        tag_len = LIFECYCLE_ASSET_TAG.len(),
        tree_len = tree.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn dormant_image_decode_probe_wat() -> String {
    let png = one_px_png([0, 255, 0, 255]);
    let png_ptr = DORMANT_IMAGE_DECODE_TAG.len();
    let mut data = DORMANT_IMAGE_DECODE_TAG.as_bytes().to_vec();
    data.extend_from_slice(&png);
    let data = wat_string_literal(&data);

    format!(
        r#"
        (module
          (import "env" "host_register_bitmap_fit"
            (func $register_fit
              (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
              (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $ready_count (mut i32) (i32.const 0))
          (global $dropped_count (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32))
          (func (export "start_decode") (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {png_ptr}
            i32.const {png_len}
            i32.const 1
            i32.const 1
            i32.const 0
            i32.const 0
            i32.const 0
            call $register_fit)
          (func (export "__on_image_ready") (param i32 i32)
            global.get $ready_count
            i32.const 1
            i32.add
            global.set $ready_count)
          (func (export "__on_image_dropped") (param i32)
            global.get $dropped_count
            i32.const 1
            i32.add
            global.set $dropped_count)
          (func (export "ready_count") (result i32) global.get $ready_count)
          (func (export "dropped_count") (result i32) global.get $dropped_count))
        "#,
        tag_len = DORMANT_IMAGE_DECODE_TAG.len(),
        png_len = png.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn cache_lifecycle_probe_wat() -> String {
    format!(
        r#"
        (module
          (import "env" "host_register_bitmap_from_cache"
            (func $register_cache (param i32 i32) (result i32)))
          (import "env" "host_evict_prefix"
            (func $evict_prefix (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "image")
          (global $initial_bitmap_id (mut i32) (i32.const 0))
          (global $wake_bitmap_id (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const 0
            i32.const 5
            call $register_cache
            global.set $initial_bitmap_id)
          (func (export "on_sleep")
            i32.const 0
            i32.const 5
            call $evict_prefix
            drop)
          (func (export "on_wake")
            i32.const 0
            i32.const 5
            call $register_cache
            global.set $wake_bitmap_id)
          (func (export "initial_bitmap_id") (result i32)
            global.get $initial_bitmap_id)
          (func (export "wake_bitmap_id") (result i32)
            global.get $wake_bitmap_id))
        "#,
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn active_asset_tree_bytes() -> Vec<u8> {
    let svg_id = SvgId::from_ffi(1).expect("BUG: first SVG registry ID must be valid");
    let draw = bmc_wasm_sdk::Draw::svg_builtin(
        0.0,
        0.0,
        32.0,
        32.0,
        svg_id,
        bmc_wasm_protocol::colors::Color::from_rgba(0xFF, 0xFF, 0xFF, 0xFF),
    )
    .animate(
        bmc_wasm_sdk::AnimProperty::Alpha,
        1.0,
        0.5,
        10_000,
        bmc_wasm_sdk::Easing::Linear,
        bmc_wasm_sdk::LoopMode::Forever,
    );
    let node = bmc_wasm_sdk::canvas(bmc_wasm_sdk::PropsData::default(), [draw]);
    bmc_wasm_sdk::serialize_node_to_bytes(&node)
}

fn active_asset_probe_wat(burn_after_first_render: bool) -> String {
    let svg = compiled_empty_svg();
    let tree = active_asset_tree_bytes();
    let svg_ptr = LIFECYCLE_ASSET_TAG.len();
    let tree_ptr = svg_ptr + svg.len();
    let mut data = LIFECYCLE_ASSET_TAG.as_bytes().to_vec();
    data.extend_from_slice(&svg);
    data.extend_from_slice(&tree);
    let data = wat_string_literal(&data);
    let burn = if burn_after_first_render {
        r"
            global.get $render_count
            i32.const 1
            i32.gt_u
            global.get $render_count
            i32.const 6
            i32.le_u
            i32.and
            if
              loop $burn
                br $burn
              end
            end
        "
    } else {
        ""
    };
    let (request_frame_import, request_frame_call) = if burn_after_first_render {
        (
            r#"(import "env" "host_request_frame" (func $request_frame))"#,
            "call $request_frame",
        )
    } else {
        ("", "")
    };
    format!(
        r#"
        (module
          (import "env" "host_register_svg"
            (func $register_svg (param i32 i32 i32 i32) (result i32)))
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          {request_frame_import}
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $asset_id (mut i32) (i32.const 0))
          (global $render_count (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            global.get $render_count
            i32.const 1
            i32.add
            global.set $render_count
            {burn}
            i32.const 0
            i32.const {tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $register_svg
            global.set $asset_id
            i32.const {tree_ptr}
            i32.const {tree_len}
            i32.const 320
            i32.const 240
            call $submit_tree
            {request_frame_call})
          (func (export "asset_id") (result i32) global.get $asset_id)
          (func (export "render_count") (result i32) global.get $render_count))
        "#,
        tag_len = LIFECYCLE_ASSET_TAG.len(),
        svg_len = svg.len(),
        tree_len = tree.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn dormant_callback_probe_wat() -> String {
    let svg = compiled_empty_svg();
    let svg_ptr = LIFECYCLE_ASSET_TAG.len();
    let mut data = LIFECYCLE_ASSET_TAG.as_bytes().to_vec();
    data.extend_from_slice(&svg);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_svg"
            (func $register_svg (param i32 i32 i32 i32) (result i32)))
          (import "env" "host_evict_prefix"
            (func $evict_prefix (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $asset_id (mut i32) (i32.const 0))
          (global $callback_id (mut i32) (i32.const -1))
          (global $callback_evicted (mut i32) (i32.const -1))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func $register (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $register_svg)
          (func (export "render") (param i32)
            call $register
            global.set $asset_id)
          (func (export "on_network_update")
            i32.const 0
            i32.const {tag_len}
            call $evict_prefix
            global.set $callback_evicted
            call $register
            global.set $callback_id)
          (func (export "asset_id") (result i32) global.get $asset_id)
          (func (export "callback_evicted") (result i32)
            global.get $callback_evicted)
          (func (export "callback_id") (result i32) global.get $callback_id))
        "#,
        tag_len = LIFECYCLE_ASSET_TAG.len(),
        svg_len = svg.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

// ── Helpers ─────────────────────────────────────────────────────────

fn credential_update_probe_widget_wat() -> String {
    include_str!("fixtures/update_probe.wat")
        .replace("__UPDATE_HOOK__", "on_credentials_update")
        .replace(
            "__SDK_VERSION__",
            &bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION).to_string(),
        )
        .replace("__REQUEST_FRAME__", "call $host_request_frame")
}

fn key(s: &str) -> ParamKey {
    ParamKey::try_new(s.to_owned()).expect("BUG: test key must be valid")
}

fn build_runtime_without_renderer(wat: impl AsRef<str>) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(wat).expect("BUG: probe WAT must parse");
    WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct")
}

fn fuel_exhausting_update_wat() -> String {
    format!(
        r#"
    (module
      (memory (export "memory") 1)
      (func (export "__bmc_sdk_init") (result i64) i64.const {})
      (func (export "render") (param i32))
      (func (export "on_params_update")
        (loop $forever br $forever)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

fn build_runtime(
    wat: impl AsRef<str>,
    gl: &headless_egl::HeadlessGl,
    initial_params: BTreeMap<ParamKey, ParamValue>,
) -> (WasmWidgetRuntime, bmc_render::gpu::FemtoVgRenderer) {
    build_runtime_with_system(wat, gl, initial_params, SystemSnapshot::default())
}

fn build_runtime_with_system(
    wat: impl AsRef<str>,
    gl: &headless_egl::HeadlessGl,
    initial_params: BTreeMap<ParamKey, ParamValue>,
    initial_system: SystemSnapshot,
) -> (WasmWidgetRuntime, bmc_render::gpu::FemtoVgRenderer) {
    let wasm = wat::parse_str(wat).expect("BUG: probe WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let renderer =
        unsafe { bmc_render::gpu::FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
            .expect("BUG: probe renderer must construct");
    let config = RuntimeConfig {
        params: initial_params,
        system: initial_system,
        ..RuntimeConfig::default()
    };
    let runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: probe runtime must construct");
    (runtime, renderer)
}

fn build_runtime_with_config(
    wat: impl AsRef<str>,
    gl: &headless_egl::HeadlessGl,
    config: RuntimeConfig,
) -> (WasmWidgetRuntime, FemtoVgRenderer) {
    let wasm = wat::parse_str(wat).expect("BUG: probe WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let renderer = unsafe { FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
        .expect("BUG: probe renderer must construct");
    let runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: probe runtime must construct");
    (runtime, renderer)
}

fn render_frame(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    delta_ms: u32,
) -> RenderStatus {
    renderer.begin_frame(320, 240, 1.0);
    let status = runtime
        .with_renderer(renderer_ptr(renderer), |runtime| runtime.render(delta_ms))
        .expect("BUG: probe render must not trap");
    renderer.flush();
    status
}

fn lifecycle_svg_state(
    runtime: &WasmWidgetRuntime,
    renderer: &FemtoVgRenderer,
) -> AssetTagState<SvgId> {
    renderer.svg_tag_state(&format!(
        "{}:{LIFECYCLE_ASSET_TAG}",
        runtime.asset_namespace()
    ))
}

// ── Tests ───────────────────────────────────────────────────────────

#[test]
fn deliver_credentials_update_fires_hook_and_preserves_frame_request() {
    let mut runtime = build_runtime_without_renderer(credential_update_probe_widget_wat());

    let hook_ran =
        runtime.deliver_credentials_update(CredentialView::default(), CredentialSecrets::default());

    assert!(
        hook_ran,
        "credential delivery must invoke on_credentials_update"
    );
    assert_eq!(runtime.call_export_i32("update_count"), Some(1));
    assert!(
        runtime.wants_next_frame(),
        "request_frame from on_credentials_update must reach the scheduler"
    );
}

#[test]
fn sdk_version_constant_matches_fixture_assumption() {
    // The single intentional version pin. The WAT fixtures derive their
    // `__bmc_sdk_init` value from `bmc_wasm_protocol::SDK_VERSION` at build
    // time, so they never need touching; this literal is the one place a
    // `SDK_VERSION` bump must be reflected by hand.
    let (major, minor, patch) = WasmWidgetRuntime::host_sdk_version();
    let packed = u64::from(major) | (u64::from(minor) << 16) | (u64::from(patch) << 32);
    assert_eq!(
        packed, 262_144,
        "host SDK version drifted to ({major}, {minor}, {patch}); \
         bumping `SDK_VERSION` means updating this pinned literal."
    );
}

#[test]
fn initial_params_via_config_do_not_fire_on_params_update() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let mut initial = BTreeMap::new();
    initial.insert(key("foo"), ParamValue::String("bar".into()));
    let (mut runtime, _renderer) = build_runtime(probe_widget_wat(), &gl, initial);

    assert_eq!(
        runtime.call_export_i32("init_count"),
        Some(1),
        "init must run exactly once on construction"
    );
    assert_eq!(
        runtime.call_export_i32("update_count"),
        Some(0),
        "RuntimeConfig::params is the initial delivery — on_params_update must NOT fire for it"
    );
}

#[test]
fn deliver_params_update_fires_hook_and_advances_version() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) = build_runtime(probe_widget_wat(), &gl, BTreeMap::new());

    let mut delivery = BTreeMap::new();
    delivery.insert(key("foo"), ParamValue::String("hello".into()));
    let hook_ran = runtime.deliver_params_update(delivery);

    assert!(
        hook_ran,
        "deliver_params_update must invoke the exported hook"
    );
    assert_eq!(runtime.call_export_i32("update_count"), Some(1));

    let first_version = runtime
        .call_export_i64("last_version_in_update")
        .expect("BUG: probe widget wat declares last_version_in_update — see probe_widget_wat()");
    assert!(
        first_version > 0,
        "version counter must have advanced past initial 0; got {first_version}"
    );

    let snapshot_len = runtime
        .call_export_i32("last_snapshot_len_in_update")
        .expect(
            "BUG: probe widget wat declares last_snapshot_len_in_update — see probe_widget_wat()",
        );
    assert!(
        snapshot_len > 4,
        "snapshot inside on_params_update must reflect the just-pushed table \
         (count header + at least one entry); got {snapshot_len} bytes"
    );

    // A second delivery must fire the hook again with a distinct version.
    let mut delivery2 = BTreeMap::new();
    delivery2.insert(key("foo"), ParamValue::String("world".into()));
    let hook_ran2 = runtime.deliver_params_update(delivery2);
    assert!(hook_ran2);
    assert_eq!(runtime.call_export_i32("update_count"), Some(2));

    let second_version = runtime
        .call_export_i64("last_version_in_update")
        .expect("BUG: probe widget wat declares last_version_in_update — see probe_widget_wat()");
    assert_ne!(
        second_version, first_version,
        "consecutive deliveries must produce distinct version values \
         (different = changed); got {first_version} both times"
    );
}

#[test]
fn host_params_snapshot_traps_on_oob_out_ptr() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) = build_runtime(oob_snapshot_traps_wat(), &gl, BTreeMap::new());

    // Any non-empty delivery so on_params_update fires.
    let mut delivery = BTreeMap::new();
    delivery.insert(key("foo"), ParamValue::String("bar".into()));
    let hook_ran = runtime.deliver_params_update(delivery);
    assert!(
        !hook_ran,
        "host_params_snapshot with an OOB out_ptr must trap on the ABI violation, \
         which surfaces as `deliver_params_update` returning false — silently returning 0 \
         (the prior behaviour) makes a misbehaving guest see 'no snapshot' instead of \
         finding out it bugged itself"
    );
}

#[test]
fn poll_deliveries_rejects_a_runtime_after_a_lifecycle_hook_trap() {
    let wasm = wat::parse_str(fuel_exhausting_update_wat()).expect("BUG: probe WAT must parse");
    let config = RuntimeConfig {
        fuel_per_frame: 1_000,
        ..RuntimeConfig::default()
    };
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: probe runtime must construct");

    assert!(!runtime.deliver_params_update(BTreeMap::new()));
    let trap = runtime
        .poll_deliveries()
        .expect_err("a lifecycle hook trap leaves the guest stack unsafe to reuse");
    assert!(trap.to_string().contains("all fuel consumed"));
}

#[cfg(feature = "capture")]
#[test]
fn capture_observes_fuel_exhaustion_from_lifecycle_hook() {
    let wasm = wat::parse_str(fuel_exhausting_update_wat()).expect("BUG: probe WAT must parse");
    let config = RuntimeConfig {
        fuel_per_frame: 1_000,
        ..RuntimeConfig::default()
    };
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: probe runtime must construct");

    assert!(!runtime.deliver_params_update(BTreeMap::new()));
    let trap = runtime
        .take_lifecycle_trap()
        .expect_err("capture must reject a profile after a lifecycle hook exhausts fuel");
    assert!(trap.to_string().contains("all fuel consumed"));
}

#[test]
fn host_submit_tree_traps_outside_render() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) =
        build_runtime(misbehaving_submit_tree_wat(), &gl, BTreeMap::new());

    let hook_ran = runtime.deliver_params_update(BTreeMap::new());
    assert!(
        !hook_ran,
        "calling host_submit_tree from on_params_update must trap the guard, \
         which surfaces as `deliver_params_update` returning false"
    );
}

// ── System snapshot channel tests ───────────────────────────────────

fn sample_system_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        settings: SystemSettings {
            timezone: "Europe/Bratislava".into(),
            time_format: TimeFormat::Hour12,
            date_format: DateFormat::YyyyMmDdDot,
            number_format: NumberFormat::CommaGroupDotDecimal,
            first_day_of_week: Weekday::Sunday,
            temperature_unit: TemperatureUnit::Fahrenheit,
            unit_system: UnitSystem::Imperial,
        },
        next_alarm: Some(NextAlarm {
            fire_at_utc_ms: 1_700_000_000_000,
            name: "Wake up".into(),
        }),
        night_mode: false,
    }
}

#[test]
fn host_system_snapshot_returns_initial_settings() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let snapshot = sample_system_snapshot();
    let (mut runtime, _renderer) =
        build_runtime_with_system(system_probe_widget_wat(), &gl, BTreeMap::new(), snapshot);

    // Initial delivery via RuntimeConfig::system must NOT fire on_system_update —
    // it's the staged state for the first frame, not an update event.
    assert_eq!(
        runtime.call_export_i32("update_count"),
        Some(0),
        "RuntimeConfig::system is the initial delivery — on_system_update must NOT fire for it"
    );

    let version = runtime
        .call_export_i64("probe_system_version")
        .expect("BUG: system probe widget must export probe_system_version");
    assert!(
        version > 0,
        "version counter must advance past initial 0 once the snapshot is staged; got {version}"
    );

    let snapshot_len = runtime
        .call_export_i32("probe_system_snapshot_len")
        .expect("BUG: system probe widget must export probe_system_snapshot_len");
    assert!(
        snapshot_len > 4,
        "encoded system snapshot must include the entry-count header plus at least one entry; got {snapshot_len} bytes"
    );
}

#[test]
fn deliver_system_update_fires_on_system_update_hook_and_advances_version() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) = build_runtime(system_probe_widget_wat(), &gl, BTreeMap::new());

    let first_delivery = sample_system_snapshot();
    let hook_ran = runtime.deliver_system_update(first_delivery);
    assert!(
        hook_ran,
        "deliver_system_update must invoke the on_system_update hook"
    );
    assert_eq!(runtime.call_export_i32("update_count"), Some(1));

    let first_version = runtime
        .call_export_i64("last_system_version")
        .expect("BUG: system probe widget exports last_system_version");
    assert!(
        first_version > 0,
        "version counter must advance past initial 0; got {first_version}"
    );

    let snapshot_len = runtime
        .call_export_i32("last_system_snapshot_len")
        .expect("BUG: system probe widget exports last_system_snapshot_len");
    assert!(
        snapshot_len > 4,
        "snapshot observed inside on_params_update must reflect the just-pushed system state; got {snapshot_len} bytes"
    );

    // A second delivery must fire the hook again with a distinct version.
    let mut second_delivery = sample_system_snapshot();
    second_delivery.settings.timezone = "America/Los_Angeles".into();
    let hook_ran2 = runtime.deliver_system_update(second_delivery);
    assert!(hook_ran2);
    assert_eq!(runtime.call_export_i32("update_count"), Some(2));

    let second_version = runtime
        .call_export_i64("last_system_version")
        .expect("BUG: system probe widget exports last_system_version");
    assert_ne!(
        second_version, first_version,
        "consecutive deliveries must produce distinct version values; got {first_version} both times"
    );
}

#[test]
fn host_system_snapshot_traps_on_oob_out_ptr() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) =
        build_runtime(oob_system_snapshot_traps_wat(), &gl, BTreeMap::new());

    let hook_ran = runtime.deliver_system_update(sample_system_snapshot());
    assert!(
        !hook_ran,
        "host_system_snapshot with an OOB out_ptr must trap the ABI violation, \
         which surfaces as `deliver_system_update` returning false"
    );
}

/// Channel isolation: a params-only delivery must NOT fire the system hook.
#[test]
fn deliver_params_update_does_not_fire_on_system_update_hook() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) = build_runtime(dual_probe_widget_wat(), &gl, BTreeMap::new());

    let mut delivery = BTreeMap::new();
    delivery.insert(key("foo"), ParamValue::String("hello".into()));
    let hook_ran = runtime.deliver_params_update(delivery);

    assert!(hook_ran, "on_params_update must fire on a params delivery");
    assert_eq!(runtime.call_export_i32("params_count"), Some(1));
    assert_eq!(
        runtime.call_export_i32("system_count"),
        Some(0),
        "params-only delivery must NOT fire on_system_update"
    );
}

/// Channel isolation: a system-only delivery must NOT fire the params hook.
#[test]
fn deliver_system_update_does_not_fire_on_params_update_hook() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) = build_runtime(dual_probe_widget_wat(), &gl, BTreeMap::new());

    let hook_ran = runtime.deliver_system_update(sample_system_snapshot());

    assert!(hook_ran, "on_system_update must fire on a system delivery");
    assert_eq!(runtime.call_export_i32("system_count"), Some(1));
    assert_eq!(
        runtime.call_export_i32("params_count"),
        Some(0),
        "system-only delivery must NOT fire on_params_update"
    );
}

#[test]
fn widget_without_hook_is_silently_fine() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    // Probe widget without an `on_params_update` export.
    let wat = format!(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_init") (result i64) i64.const {})
          (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    );
    let (mut runtime, _renderer) = build_runtime(&wat, &gl, BTreeMap::new());

    let hook_ran = runtime.deliver_params_update(BTreeMap::new());
    assert!(
        !hook_ran,
        "absent `on_params_update` export must be silently fine — return value `false` \
         signals 'no hook', not a trap"
    );
}

// ── Touch channel tests ─────────────────────────────────────────────

#[test]
fn deliver_touch_fires_on_touch_hook_and_requests_frame() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let (mut runtime, _renderer) = build_runtime(touch_probe_widget_wat(), &gl, BTreeMap::new());

    assert!(
        runtime.exports_on_touch(),
        "touch probe widget exports on_touch"
    );
    assert!(
        !runtime.wants_next_frame(),
        "no frame should be pending before any touch is delivered"
    );

    let hook_ran = runtime.deliver_touch();
    assert!(hook_ran, "deliver_touch must invoke the on_touch hook");
    assert_eq!(runtime.call_export_i32("touch_count"), Some(1));
    assert!(
        runtime.wants_next_frame(),
        "request_frame() called from on_touch must leave the runtime wanting a frame — \
         this is the mechanism that re-renders an otherwise-idle widget on touch"
    );
}

#[test]
fn widget_without_on_touch_drops_touch_without_requesting_a_frame() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    // A widget that exports no `on_touch` is non-interactive.
    let wat = format!(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_init") (result i64) i64.const {})
          (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    );
    let (mut runtime, _renderer) = build_runtime(&wat, &gl, BTreeMap::new());

    assert!(
        !runtime.exports_on_touch(),
        "widget without the export must report exports_on_touch() == false so the host \
         drops its touch events instead of queuing them for a render that never comes"
    );

    let hook_ran = runtime.deliver_touch();
    assert!(
        !hook_ran,
        "absent on_touch export — deliver_touch returns false, not a trap"
    );
    assert!(
        !runtime.wants_next_frame(),
        "no on_touch hook means no request_frame, so no frame is scheduled"
    );
}

// ── Network channel tests ───────────────────────────────────────────

#[test]
fn deliver_network_update_fires_hook_and_requests_frame() {
    let mut runtime = build_runtime_without_renderer(network_probe_widget_wat());

    assert!(
        !runtime.wants_next_frame(),
        "no frame should be pending before any network change is delivered"
    );

    runtime.set_network_info(bmc_wasm_runtime::NetworkInfo {
        ssid: "deck-net".to_owned(),
        ip: "10.0.0.7".to_owned(),
    });
    let hook_ran = runtime.deliver_network_update();
    assert!(
        hook_ran,
        "deliver_network_update must invoke the on_network_update hook"
    );
    assert_eq!(runtime.call_export_i32("network_count"), Some(1));
    assert!(
        runtime.wants_next_frame(),
        "request_frame() called from on_network_update must leave the runtime wanting a \
         frame — the host never force-renders on network changes, so this is the only \
         way a network change reaches the screen"
    );
}

#[test]
fn widget_without_on_network_update_is_silently_skipped() {
    let wat = format!(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_init") (result i64) i64.const {})
          (func (export "render") (param i32)))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    );
    let mut runtime = build_runtime_without_renderer(&wat);

    let hook_ran = runtime.deliver_network_update();
    assert!(
        !hook_ran,
        "absent on_network_update export — deliver_network_update returns false, not a trap"
    );
    assert!(
        !runtime.wants_next_frame(),
        "no hook means no request_frame; the widget sees the new info on its next natural render"
    );
}

#[test]
fn dormant_widget_without_sleep_hook_retains_guest_memory_assets() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) =
        build_runtime_with_config(sleep_asset_probe_wat(""), &gl, RuntimeConfig::default());

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let asset_id = SvgId::from_ffi(
        u32::try_from(
            runtime
                .call_export_i32("asset_id")
                .expect("BUG: sleep probe must export asset_id"),
        )
        .expect("BUG: asset ID must fit u32"),
    )
    .expect("BUG: initial SVG registration must return an ID");
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );

    assert!(!runtime.notify_dormant());
    poll_renderer_deliveries(&mut runtime, &mut renderer);

    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id),
        "guest-memory assets must stay drawable after wake"
    );
}

#[test]
fn trapping_sleep_hook_still_retains_assets() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = build_runtime_with_config(
        sleep_asset_probe_wat(r#"(func (export "on_sleep") unreachable)"#),
        &gl,
        RuntimeConfig::default(),
    );

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let asset_id = SvgId::from_ffi(
        u32::try_from(
            runtime
                .call_export_i32("asset_id")
                .expect("BUG: sleep probe must export asset_id"),
        )
        .expect("BUG: asset ID must fit u32"),
    )
    .expect("BUG: initial SVG registration must return an ID");

    assert!(runtime.notify_dormant());
    poll_renderer_deliveries_expect_trap(&mut runtime, &mut renderer, "on_sleep");

    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );
}

#[test]
fn rendererless_delivery_retains_pending_sleep_edge() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let on_sleep = r#"
        (func (export "on_sleep")
          global.get $sleep_count
          i32.const 1
          i32.add
          global.set $sleep_count)
    "#;
    let (mut runtime, mut renderer) = build_runtime_with_config(
        sleep_asset_probe_wat(on_sleep),
        &gl,
        RuntimeConfig::default(),
    );

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let AssetTagState::Resident(asset_id) = lifecycle_svg_state(&runtime, &renderer) else {
        panic!("BUG: initial SVG must be resident");
    };
    assert!(
        !runtime.has_pending_lifecycle(),
        "an idle runtime must not require a GPU-locked lifecycle flush"
    );

    assert!(runtime.notify_dormant());
    assert!(runtime.has_pending_lifecycle());
    runtime
        .poll_deliveries()
        .expect("BUG: rendererless lifecycle delivery must not trap");
    assert_eq!(runtime.call_export_i32("sleep_count"), Some(0));
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id),
        "rendererless delivery must not consume the sleep edge"
    );

    poll_renderer_deliveries(&mut runtime, &mut renderer);
    assert!(!runtime.has_pending_lifecycle());
    assert_eq!(runtime.call_export_i32("sleep_count"), Some(1));
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );
}

#[test]
fn wake_after_rendererless_delivery_runs_hooks_without_asset_churn() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = build_runtime_with_config(
        deferred_sleep_then_wake_probe_wat(),
        &gl,
        RuntimeConfig::default(),
    );

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let AssetTagState::Resident(asset_id) = lifecycle_svg_state(&runtime, &renderer) else {
        panic!("BUG: initial SVG must be resident");
    };

    assert!(runtime.notify_dormant());
    runtime
        .poll_deliveries()
        .expect("BUG: rendererless lifecycle delivery must not trap");
    assert_eq!(runtime.call_export_i32("event_order"), Some(0));
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id),
        "rendererless delivery must retain the sleep edge"
    );

    assert!(runtime.notify_wake());
    poll_renderer_deliveries(&mut runtime, &mut renderer);

    assert_eq!(
        runtime.call_export_i32("event_order"),
        Some(12),
        "sleep must run before wake when both edges await a renderer"
    );
    assert_eq!(
        runtime.call_export_i32("sleep_registration_id"),
        Some(i32::try_from(asset_id.to_ffi()).expect("BUG: SVG ID must fit i32")),
        "a volatile pointer asset must remain usable during the dormant hook"
    );
    assert_eq!(
        runtime.call_export_i32("wake_probe_id"),
        Some(i32::try_from(asset_id.to_ffi()).expect("BUG: SVG ID must fit i32")),
        "coalesced sleep and wake must preserve the resident asset"
    );
    assert_eq!(
        runtime.call_export_i32("wake_restore_id"),
        Some(i32::try_from(asset_id.to_ffi()).expect("BUG: SVG ID must fit i32"))
    );
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );
}

#[test]
fn first_render_restores_asset_registered_by_coalesced_sleep_hook() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let package_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let payload = compiled_empty_svg();
    let id = bmc_wasm_assets::package_asset_id(PackageAssetKind::Svg, &payload);
    let asset_dir = package_dir.path().join("v1").join("svg");
    std::fs::create_dir_all(&asset_dir).expect("BUG: package fixture directory must construct");
    std::fs::write(asset_dir.join(format!("{id}.asset")), payload)
        .expect("BUG: package fixture must be writable");
    let config = RuntimeConfig {
        package_assets: Some(PackageAssetStore::new(package_dir.path())),
        ..RuntimeConfig::default()
    };
    let (mut runtime, mut renderer) = build_runtime_with_config(
        coalesced_package_registration_wat(id.as_bytes()),
        &gl,
        config,
    );

    runtime.notify_dormant();
    runtime.notify_wake();
    poll_renderer_deliveries(&mut runtime, &mut renderer);

    let registered = runtime
        .call_export_i32("sleep_registration_id")
        .and_then(|id| u32::try_from(id).ok())
        .and_then(SvgId::from_ffi)
        .expect("BUG: sleep hook must reserve a valid SVG ID");
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Suspended(registered),
        "wake must leave an unused package asset suspended"
    );

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(registered),
        "the first tree referencing the package ID must restore its reservation"
    );
}

#[test]
fn sleep_after_deferred_wake_delivers_both_hooks_in_final_dormant_state() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = build_runtime_with_config(
        deferred_sleep_then_wake_probe_wat(),
        &gl,
        RuntimeConfig::default(),
    );

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let AssetTagState::Resident(asset_id) = lifecycle_svg_state(&runtime, &renderer) else {
        panic!("BUG: initial SVG must be resident");
    };
    runtime.notify_dormant();
    poll_renderer_deliveries(&mut runtime, &mut renderer);
    assert_eq!(runtime.call_export_i32("reset_event_order"), Some(0));

    runtime.notify_wake();
    runtime
        .poll_deliveries()
        .expect("BUG: rendererless lifecycle delivery must not trap");
    runtime.notify_dormant();
    poll_renderer_deliveries(&mut runtime, &mut renderer);

    assert_eq!(
        runtime.call_export_i32("event_order"),
        Some(21),
        "the queued wake must run before sleep without restoring or suspending assets"
    );
    assert_eq!(
        runtime.call_export_i32("wake_restore_id"),
        Some(i32::try_from(asset_id.to_ffi()).expect("BUG: SVG ID must fit i32")),
        "the resident volatile asset must remain usable during both hooks"
    );
    assert!(
        runtime.renderer_assets_are_dormant_for_test(),
        "the final committed state must remain dormant"
    );
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );
}

#[test]
fn rendererless_dormant_poll_defers_uncached_image_decode_without_upload() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = build_runtime_with_config(
        dormant_image_decode_probe_wat(),
        &gl,
        RuntimeConfig::default(),
    );
    let job_id = runtime
        .with_renderer(renderer_ptr(&mut renderer), |runtime| {
            runtime.call_export_i32("start_decode")
        })
        .expect("BUG: decode probe must export start_decode");
    assert_ne!(job_id, 0, "active decode registration must start a job");

    runtime.notify_dormant();
    let deadline = Instant::now() + IMAGE_DECODE_COMPLETION_TIMEOUT;
    while !runtime.has_completed_image_decodes_for_test() && Instant::now() < deadline {
        runtime
            .poll_deliveries()
            .expect("BUG: rendererless image delivery must not trap");
        std::thread::yield_now();
    }
    assert!(
        runtime.has_completed_image_decodes_for_test(),
        "rendererless dormant delivery must retain a completion that cannot be cached"
    );
    let mut renderer_accessed = false;
    while runtime.has_pending_image_decodes() && Instant::now() < deadline {
        renderer_accessed |= poll_renderer_deliveries(&mut runtime, &mut renderer);
        std::thread::yield_now();
    }
    assert!(!runtime.has_pending_image_decodes());
    assert!(
        !renderer_accessed,
        "discarding an uncached dormant decode must not request a GPU fence"
    );
    assert_eq!(
        runtime.call_export_i32("dropped_count"),
        Some(1),
        "the dormant completion must reclaim the guest's pending job"
    );
    assert_eq!(
        runtime.call_export_i32("ready_count"),
        Some(0),
        "the dormant completion must not drive the active callback"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&format!(
            "{}:{DORMANT_IMAGE_DECODE_TAG}",
            runtime.asset_namespace()
        )),
        AssetTagState::Unknown,
        "a dormant decode without cache backing must not upload an unreachable texture"
    );
}

#[test]
fn sleep_eviction_and_wake_cache_registration_reuse_the_released_bitmap_id() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let cache_dir = tempfile::tempdir().expect("BUG: cache tempdir must construct");
    let cache = DiskCache::new(cache_dir.path().to_path_buf(), 1_048_576);
    let mut metadata = Vec::from(1_u32.to_le_bytes());
    metadata.extend_from_slice(&1_u32.to_le_bytes());
    metadata.extend_from_slice(b"arbitrary identity");
    cache
        .put("image", 1, &metadata, &[0x11, 0x22, 0x33, 0xFF])
        .expect("BUG: cache fixture must be writable");
    let config = RuntimeConfig {
        asset_cache: Some(cache),
        ..RuntimeConfig::default()
    };
    let (mut runtime, mut renderer) =
        build_runtime_with_config(cache_lifecycle_probe_wat(), &gl, config);
    let renderer_tag = format!("{}:image", runtime.asset_namespace());

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let first_raw = u32::try_from(
        runtime
            .call_export_i32("initial_bitmap_id")
            .expect("BUG: cache probe must export initial_bitmap_id"),
    )
    .expect("BUG: bitmap ID must fit u32");
    let first_id = bmc_wasm_protocol::BitmapId::from_ffi(first_raw)
        .expect("BUG: initial cache restore must return an ID");
    assert_eq!(
        renderer.bitmap_tag_state(&renderer_tag),
        AssetTagState::Suspended(first_id),
        "cache registration must reserve without uploading before a draw uses the ID"
    );

    assert!(runtime.notify_dormant());
    poll_renderer_deliveries(&mut runtime, &mut renderer);
    assert_eq!(
        renderer.bitmap_tag_state(&renderer_tag),
        AssetTagState::Unknown
    );

    assert!(runtime.notify_wake());
    poll_renderer_deliveries(&mut runtime, &mut renderer);
    let wake_raw = u32::try_from(
        runtime
            .call_export_i32("wake_bitmap_id")
            .expect("BUG: cache probe must export wake_bitmap_id"),
    )
    .expect("BUG: bitmap ID must fit u32");
    let wake_id = bmc_wasm_protocol::BitmapId::from_ffi(wake_raw)
        .expect("BUG: wake cache restore must return a nonzero ID");
    assert_eq!(
        wake_id, first_id,
        "explicit on_sleep eviction must make its ID available to the next registration"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&renderer_tag),
        AssetTagState::Suspended(wake_id),
        "wake registration must leave the reused reservation lazy until a draw uses it"
    );
}

#[test]
fn first_wake_frame_runs_guest_with_resident_asset() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) =
        build_runtime_with_config(active_asset_probe_wat(false), &gl, RuntimeConfig::default());

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    assert_eq!(runtime.call_export_i32("render_count"), Some(1));
    let AssetTagState::Resident(asset_id) = lifecycle_svg_state(&runtime, &renderer) else {
        panic!("BUG: initial SVG must be resident");
    };
    assert_eq!(
        asset_id.to_ffi(),
        1,
        "cached tree fixture uses the first SVG ID"
    );
    assert!(
        runtime.wants_next_frame(),
        "looping animation must make cached-tree replay eligible"
    );

    runtime.notify_dormant();
    poll_renderer_deliveries(&mut runtime, &mut renderer);
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );

    runtime.notify_wake();
    poll_renderer_deliveries(&mut runtime, &mut renderer);
    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );

    assert_eq!(
        runtime.call_export_i32("render_count"),
        Some(2),
        "wake must force exactly one full guest frame instead of animation-only replay"
    );
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id),
        "the first wake render must preserve the asset ID"
    );
}

#[test]
fn dormant_callback_can_destructively_evict_and_recreate_a_volatile_asset() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) =
        build_runtime_with_config(dormant_callback_probe_wat(), &gl, RuntimeConfig::default());

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let AssetTagState::Resident(asset_id) = lifecycle_svg_state(&runtime, &renderer) else {
        panic!("BUG: initial SVG must be resident");
    };
    runtime.notify_dormant();
    poll_renderer_deliveries(&mut runtime, &mut renderer);

    let callback_ran = runtime.with_renderer(renderer_ptr(&mut renderer), |runtime| {
        runtime.deliver_network_update()
    });

    assert!(callback_ran, "network callback must reach the guest");
    assert_eq!(
        runtime.call_export_i32("callback_evicted"),
        Some(1),
        "explicit eviction must remain destructive while dormant"
    );
    let callback_id = SvgId::from_ffi(
        u32::try_from(
            runtime
                .call_export_i32("callback_id")
                .expect("BUG: callback probe must export callback_id"),
        )
        .expect("BUG: callback SVG ID must fit u32"),
    )
    .expect("BUG: dormant pointer registration must return an ID");
    assert_eq!(callback_id, asset_id);
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(callback_id)
    );
}

#[test]
fn fuel_dead_frames_replay_cached_assets_without_scheduling_and_reset_forces_guest_execution() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let config = RuntimeConfig {
        fuel_per_frame: 5_000,
        ..RuntimeConfig::default()
    };
    let (mut runtime, mut renderer) =
        build_runtime_with_config(active_asset_probe_wat(true), &gl, config);

    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    let AssetTagState::Resident(asset_id) = lifecycle_svg_state(&runtime, &renderer) else {
        panic!("BUG: initial SVG must be resident");
    };
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );

    for strike in 1..5 {
        assert_eq!(
            render_frame(&mut runtime, &mut renderer, 16),
            RenderStatus::FuelExhausted,
            "strike {strike} must remain transient"
        );
    }
    let frame_counter_before_death = runtime.frame_counter_for_test();
    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Dead
    );
    assert_eq!(
        runtime.frame_counter_for_test(),
        frame_counter_before_death + 1,
        "the first permanent-death frame must replay the cached tree behind its overlay"
    );
    assert!(
        !runtime.wants_next_frame(),
        "first permanent-death frame must not schedule the animated cached tree"
    );
    let frame_counter_before_dead_replay = runtime.frame_counter_for_test();
    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Dead
    );
    assert_eq!(
        runtime.frame_counter_for_test(),
        frame_counter_before_dead_replay + 1,
        "an already-dead frame must replay the cached tree behind its overlay"
    );
    assert!(
        !runtime.wants_next_frame(),
        "already-dead frames must not schedule the animated cached tree"
    );
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );

    runtime.reset_fuel_state();
    assert!(
        runtime.wants_next_frame(),
        "reset must request an immediate frame"
    );
    assert_eq!(
        render_frame(&mut runtime, &mut renderer, 16),
        RenderStatus::Ok
    );
    assert_eq!(
        runtime.call_export_i32("render_count"),
        Some(7),
        "reset must force guest execution instead of cached-tree replay"
    );
    assert_eq!(
        lifecycle_svg_state(&runtime, &renderer),
        AssetTagState::Resident(asset_id)
    );
}

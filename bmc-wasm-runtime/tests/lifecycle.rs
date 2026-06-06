// Copyright (C) 2026  Braiins Systems s.r.o.

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
//! `WasmWidgetRuntime::new` needs a current EGL/GL context for FemtoVG.
//! The `ci` build profile (`nix/profiles.nix`) supplies Mesa, llvmpipe
//! and surfaceless EGL inside the Nix sandbox.
//! Locally without Nix the EGL init will fail, and each test then skips
//! with a clear log line rather than spuriously failing.
//!
//! The expectation is that CI (which runs through the `ci`
//! profile) is the authoritative environment for these tests.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;

use bmc_wasm_protocol::system::{
    DateFormat, NumberFormat, TemperatureUnit, TimeFormat, UnitSystem, Weekday,
};
use bmc_wasm_runtime::{
    NextAlarm, RuntimeConfig, SystemSettings, SystemSnapshot, WasmWidgetRuntime,
};
use bmc_widget_manifest::{ParamKey, ParamValue};

mod common;
use common::headless_egl;

// ── Probe widget fixtures (hand-rolled WAT) ─────────────────────────

/// Test widget that counts lifecycle invocations and records observations
/// on each `on_params_update` call. Exports observation getters used by the tests.
///
/// SDK version reported: (0, 1, 0) — matches `bmc_wasm_protocol::SDK_VERSION`
/// at the time of writing. If the host's major bumps, this fixture needs to bump too.
fn probe_widget_wat() -> &'static str {
    // Packed SDK version: major=0 | minor=1<<16 | patch=0<<32 = 0x10000 = 65536.
    // Update this constant in lockstep with `bmc_wasm_protocol::SDK_VERSION` (asserted below).
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

      ;; Required by the host. Returns packed SDK_VERSION = (0, 1, 0).
      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

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
    "#
}

/// Misbehaving widget: calls `host_params_snapshot` with an out-of-bounds `out_ptr`.
/// Used to assert the host traps the ABI violation rather than fail-quiet returning 0.
fn oob_snapshot_traps_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_params_snapshot"
        (func $host_params_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      (func (export "render") (param i32))

      ;; out_ptr = u32::MAX (well past the guest's single 64 KiB page).
      ;; out_cap large enough that the host's `out_cap < needed` early-return doesn't fire
      ;; and the function falls into the memory-write branch.
      (func (export "on_params_update")
        i32.const -1     ;; out_ptr = u32::MAX in two's complement
        i32.const 4096   ;; out_cap
        call $host_params_snapshot
        drop))
    "#
}

/// Probe widget for the system snapshot channel.
/// Imports `host_system_version` + `host_system_snapshot`
/// and records the version and snapshot length it observes
/// inside `on_params_update` (the unified hook — system updates
/// fire the same export as params updates).
fn system_probe_widget_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_system_version" (func $host_system_version (result i64)))
      (import "env" "host_system_snapshot"
        (func $host_system_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (global $update_count (mut i32) (i32.const 0))
      (global $last_system_version (mut i64) (i64.const 0))
      (global $last_system_snapshot_len (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

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
    "#
}

/// Widget exporting both `on_params_update` and `on_system_update`
/// with independent counters, for asserting channel isolation.
fn dual_probe_widget_wat() -> &'static str {
    r#"
    (module
      (memory (export "memory") 1)

      (global $params_count (mut i32) (i32.const 0))
      (global $system_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

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
    "#
}

/// Misbehaving widget calling `host_system_snapshot`
/// with an out-of-bounds `out_ptr`.
/// Mirrors `oob_snapshot_traps_wat` for the system channel.
fn oob_system_snapshot_traps_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_system_snapshot"
        (func $host_system_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      (func (export "render") (param i32))

      (func (export "on_system_update")
        i32.const -1     ;; out_ptr = u32::MAX in two's complement
        i32.const 4096   ;; out_cap
        call $host_system_snapshot
        drop))
    "#
}

/// Misbehaving widget: calls `host_submit_tree` from `on_params_update`.
/// Used to assert the lifecycle guard traps the call.
fn misbehaving_submit_tree_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_submit_tree"
        (func $host_submit_tree (param i32 i32 i32 i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      (func (export "render") (param i32))

      ;; Calls host_submit_tree with a zero-length tree. The guard should trap before the
      ;; runtime parses any bytes, so the call never actually mutates renderer state.
      (func (export "on_params_update")
        i32.const 0
        i32.const 0
        i32.const 0
        i32.const 0
        call $host_submit_tree))
    "#
}

/// Probe widget for the touch channel. Exports `on_touch`, which bumps a
/// counter and calls `host_request_frame` — the canonical "re-render in
/// response to touch" response. The counter getter lets the test assert the
/// hook fired exactly once.
fn touch_probe_widget_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_request_frame" (func $host_request_frame))

      (memory (export "memory") 1)

      (global $touch_count (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      (func (export "render") (param i32))

      (func (export "on_touch")
        global.get $touch_count
        i32.const 1
        i32.add
        global.set $touch_count
        call $host_request_frame)

      (func (export "touch_count") (result i32) global.get $touch_count))
    "#
}

// ── Helpers ─────────────────────────────────────────────────────────

fn key(s: &str) -> ParamKey {
    ParamKey::try_new(s.to_owned()).expect("BUG: test key must be valid")
}

fn build_runtime(
    wat: &str,
    gl: &headless_egl::HeadlessGl,
    initial_params: BTreeMap<ParamKey, ParamValue>,
) -> (WasmWidgetRuntime, bmc_render::gpu::FemtoVgRenderer) {
    build_runtime_with_system(wat, gl, initial_params, SystemSnapshot::default())
}

fn build_runtime_with_system(
    wat: &str,
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
        config,
    )
    .expect("BUG: probe runtime must construct");
    (runtime, renderer)
}

// ── Tests ───────────────────────────────────────────────────────────

#[test]
fn sdk_version_constant_matches_fixture_assumption() {
    // Hardwires the WAT fixture's packed version literal to the host's `SDK_VERSION`.
    // If this assertion fires, the fixture's `i64.const` needs updating.
    let (major, minor, patch) = WasmWidgetRuntime::host_sdk_version();
    let packed = u64::from(major) | (u64::from(minor) << 16) | (u64::from(patch) << 32);
    assert_eq!(
        packed, 65_536,
        "probe WAT fixtures hardcode `i64.const 65536` for `__bmc_sdk_version`; \
         host SDK version drifted to ({major}, {minor}, {patch}) — update fixtures."
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
    let wat = r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_version") (result i64) i64.const 65536)
          (func (export "render") (param i32)))
    "#;
    let (mut runtime, _renderer) = build_runtime(wat, &gl, BTreeMap::new());

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
    let wat = r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_version") (result i64) i64.const 65536)
          (func (export "render") (param i32)))
    "#;
    let (mut runtime, _renderer) = build_runtime(wat, &gl, BTreeMap::new());

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

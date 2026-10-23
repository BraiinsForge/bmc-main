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

//! Periodic timers tick off the host's monotonic clock, with no render involved.

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::cell::RefCell;
use std::io::Write;
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Local;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RuntimeConfig, RuntimeResourceLimits, WasmWidgetRuntime};

mod common;
use common::headless_egl;

/// A guest whose `init` runs `init_body`,
/// counting ticks per timer and calls to its own sleep and wake hooks.
///
/// `$first` and `$second` hold the ids `init_body` stores there.
/// With `$stop_second_on_tick` set, the first timer's tick stops the second;
/// with `$slow_second_on_tick` set, its next tick moves the second to a 3 s period once.
/// With `$slow_first_on_wake` set, `on_wake` moves the first to a 5 s period.
fn timer_wat(init_body: &str, with_on_timer_export: bool) -> String {
    let on_timer = if with_on_timer_export {
        r#"
      (func (export "__on_timer") (param $id i32) (param $elapsed i32)
        (global.set $last_elapsed (local.get $elapsed))
        (if (i32.eq (local.get $id) (global.get $first))
          (then
            (global.set $first_ticks (i32.add (global.get $first_ticks) (i32.const 1)))
            (if (global.get $stop_second_on_tick)
              (then (call $stop (global.get $second))))
            (if (global.get $slow_second_on_tick)
              (then
                (drop (call $set_period (global.get $second) (i32.const 3000)))
                (global.set $slow_second_on_tick (i32.const 0))))))
        (if (i32.eq (local.get $id) (global.get $second))
          (then
            (global.set $second_ticks (i32.add (global.get $second_ticks) (i32.const 1))))))
        "#
    } else {
        ""
    };
    format!(
        r#"
    (module
      (import "env" "host_timer_start" (func $start (param i32) (result i32)))
      (import "env" "host_timer_set_period" (func $set_period (param i32 i32) (result i32)))
      (import "env" "host_timer_stop" (func $stop (param i32)))

      (memory (export "memory") 1)

      (global $first (mut i32) (i32.const 0))
      (global $second (mut i32) (i32.const 0))
      (global $first_ticks (mut i32) (i32.const 0))
      (global $second_ticks (mut i32) (i32.const 0))
      (global $last_elapsed (mut i32) (i32.const 0))
      (global $stop_second_on_tick (mut i32) (i32.const 0))
      (global $slow_second_on_tick (mut i32) (i32.const 0))
      (global $slow_first_on_wake (mut i32) (i32.const 0))
      (global $sleeps (mut i32) (i32.const 0))
      (global $wakes (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {sdk_version})

      (func (export "__alloc") (param $len i32) (result i32)
        (i32.const 1024))

      (func (export "init")
        {init_body})

      {on_timer}

      (func (export "render") (param i32))

      (func (export "on_sleep")
        (global.set $sleeps (i32.add (global.get $sleeps) (i32.const 1))))
      (func (export "on_wake")
        (global.set $wakes (i32.add (global.get $wakes) (i32.const 1)))
        (if (global.get $slow_first_on_wake)
          (then (drop (call $set_period (global.get $first) (i32.const 5000))))))

      (func (export "slow_first_to_3s") (result i32)
        (call $set_period (global.get $first) (i32.const 3000)))
      (func (export "speed_first_past_the_minimum") (result i32)
        (call $set_period (global.get $first) (i32.const 999)))

      (func (export "first") (result i32) (global.get $first))
      (func (export "second") (result i32) (global.get $second))
      (func (export "sleeps") (result i32) (global.get $sleeps))
      (func (export "wakes") (result i32) (global.get $wakes))
      (func (export "first_ticks") (result i32) (global.get $first_ticks))
      (func (export "second_ticks") (result i32) (global.get $second_ticks))
      (func (export "last_elapsed") (result i32) (global.get $last_elapsed)))
    "#,
        sdk_version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

const ONE_SECOND_TIMER: &str = "(global.set $first (call $start (i32.const 1000)))";

/// A runtime whose clock has started at 0, as a host slot's does.
fn runtime(wat: &str) -> WasmWidgetRuntime {
    started(unstarted_runtime(wat, RuntimeConfig::default()))
}

fn runtime_with_cap(wat: &str, max_timers: usize) -> WasmWidgetRuntime {
    runtime_with_limits(
        wat,
        RuntimeResourceLimits {
            max_timers,
            ..RuntimeResourceLimits::default()
        },
    )
}

fn runtime_with_limits(wat: &str, resource_limits: RuntimeResourceLimits) -> WasmWidgetRuntime {
    let config = RuntimeConfig {
        resource_limits,
        ..RuntimeConfig::default()
    };
    started(unstarted_runtime(wat, config))
}

fn started(mut runtime: WasmWidgetRuntime) -> WasmWidgetRuntime {
    advance_to(&mut runtime, 0);
    runtime
}

fn unstarted_runtime(wat: &str, config: RuntimeConfig) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(wat).expect("BUG: timer WAT must parse");
    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_ui::ViewportShape::Rectangular,
        common::test_display(64, 64),
        Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: runtime construct")
}

fn advance_to(runtime: &mut WasmWidgetRuntime, monotonic_ms: u64) {
    runtime.set_time(Local::now().fixed_offset(), monotonic_ms);
    runtime
        .poll_deliveries()
        .expect("BUG: a timer tick must not trap");
}

/// Like [`advance_to`], with a renderer installed so queued sleep and wake hooks run.
fn advance_with_renderer(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    monotonic_ms: u64,
) {
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    let renderer = NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    runtime.set_time(Local::now().fixed_offset(), monotonic_ms);
    runtime
        .poll_deliveries_with_renderer(renderer)
        .expect("BUG: a timer tick must not trap");
}

#[test]
fn a_timer_ticks_at_its_period_without_rendering() {
    let mut runtime = runtime(&timer_wat(ONE_SECOND_TIMER, true));

    advance_to(&mut runtime, 999);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(0));

    advance_to(&mut runtime, 1_000);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(1));
    assert_eq!(runtime.call_export_i32("last_elapsed"), Some(1));
}

/// The testbed's clock runs from app start, so it is far along when a widget loads.
#[test]
fn a_late_first_clock_reading_brings_no_burst_of_missed_periods() {
    let mut runtime =
        unstarted_runtime(&timer_wat(ONE_SECOND_TIMER, true), RuntimeConfig::default());

    advance_to(&mut runtime, 600_000);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(0));

    advance_to(&mut runtime, 601_000);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(1));
    assert_eq!(runtime.call_export_i32("last_elapsed"), Some(1));
}

#[test]
fn ticks_keep_arriving_while_dormant_and_after_wake() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer construct");
    let mut runtime = runtime(&timer_wat(ONE_SECOND_TIMER, true));

    runtime.notify_dormant();
    advance_with_renderer(&mut runtime, &mut renderer, 1_000);
    assert_eq!(
        runtime.call_export_i32("sleeps"),
        Some(1),
        "the widget must be asleep when its tick arrives"
    );
    assert_eq!(
        runtime.call_export_i32("first_ticks"),
        Some(1),
        "a dormant widget missed its tick"
    );

    runtime.notify_wake();
    advance_with_renderer(&mut runtime, &mut renderer, 2_000);
    assert_eq!(runtime.call_export_i32("wakes"), Some(1));
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(2));
}

#[test]
fn missed_periods_arrive_as_one_tick_with_the_elapsed_count() {
    let mut runtime = runtime(&timer_wat(ONE_SECOND_TIMER, true));
    advance_to(&mut runtime, 1_000);

    advance_to(&mut runtime, 4_500);

    assert_eq!(runtime.call_export_i32("first_ticks"), Some(2));
    assert_eq!(
        runtime.call_export_i32("last_elapsed"),
        Some(3),
        "ticks were due at 2 s, 3 s and 4 s"
    );
}

/// A guest whose `init` starts `timers` 1 s timers
/// and whose every tick counts to `iterations` before it records that it completed.
fn burning_timer_wat(timers: usize, iterations: u32) -> String {
    let init_body = "(drop (call $start (i32.const 1000)))".repeat(timers);
    format!(
        r#"
    (module
      (import "env" "host_timer_start" (func $start (param i32) (result i32)))

      (memory (export "memory") 1)

      (global $completed (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {sdk_version})

      (func (export "init")
        {init_body})

      (func (export "__on_timer") (param $id i32) (param $elapsed i32) (local $i i32)
        (block $done
          (loop $again
            (br_if $done (i32.ge_u (local.get $i) (i32.const {iterations})))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $again)))
        (global.set $completed (i32.add (global.get $completed) (i32.const 1))))

      (func (export "render") (param i32))

      (func (export "completed") (result i32) (global.get $completed)))
    "#,
        sdk_version = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

/// One tick at this budget affords about 11 000 iterations of `burning_timer_wat` at wasmi's current costs.
const TICK_FUEL: u64 = 100_000;
/// Fits one budget, while two ticks' worth does not.
const MORE_THAN_HALF_A_BUDGET: u32 = 8_000;

fn fuel_runtime(timers: usize, iterations: u32) -> WasmWidgetRuntime {
    let config = RuntimeConfig {
        fuel_per_frame: TICK_FUEL,
        ..RuntimeConfig::default()
    };
    let mut runtime = started(unstarted_runtime(
        &burning_timer_wat(timers, iterations),
        config,
    ));
    runtime.set_time(Local::now().fixed_offset(), 1_000);
    runtime
}

#[test]
fn each_tick_runs_on_a_fresh_fuel_budget() {
    let mut one_tick = fuel_runtime(1, MORE_THAN_HALF_A_BUDGET);
    one_tick
        .poll_deliveries()
        .expect("premise: one tick fits a budget; re-tune MORE_THAN_HALF_A_BUDGET");
    let mut double_work = fuel_runtime(1, 2 * MORE_THAN_HALF_A_BUDGET);
    let error = double_work
        .poll_deliveries()
        .expect_err("premise: two ticks' worth exceeds a budget; re-tune MORE_THAN_HALF_A_BUDGET");
    assert!(error.to_string().contains("all fuel consumed"), "{error}");

    let mut runtime = fuel_runtime(2, MORE_THAN_HALF_A_BUDGET);
    runtime
        .poll_deliveries()
        .expect("each tick must get its own budget");

    assert_eq!(runtime.call_export_i32("completed"), Some(2));
}

#[test]
fn stopping_a_timer_drops_its_tick_already_due_in_the_same_pass() {
    let init = "(global.set $first (call $start (i32.const 1000)))
        (global.set $second (call $start (i32.const 1000)))
        (global.set $stop_second_on_tick (i32.const 1))";
    let mut runtime = runtime(&timer_wat(init, true));

    advance_to(&mut runtime, 1_000);
    advance_to(&mut runtime, 2_000);

    assert_eq!(runtime.call_export_i32("first_ticks"), Some(2));
    assert_eq!(runtime.call_export_i32("second_ticks"), Some(0));
}

#[test]
fn a_new_period_keeps_the_timer_and_counts_from_the_change() {
    let mut runtime = runtime(&timer_wat(ONE_SECOND_TIMER, true));
    advance_to(&mut runtime, 500);

    assert_eq!(runtime.call_export_i32("slow_first_to_3s"), Some(1));

    advance_to(&mut runtime, 3_499);
    assert_eq!(
        runtime.call_export_i32("first_ticks"),
        Some(0),
        "the old 1 s period no longer applies"
    );
    advance_to(&mut runtime, 3_500);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(1));
}

#[test]
fn a_period_change_drops_the_tick_already_due_in_the_same_pass() {
    let init = "(global.set $first (call $start (i32.const 1000)))
        (global.set $second (call $start (i32.const 1000)))
        (global.set $slow_second_on_tick (i32.const 1))";
    let mut runtime = runtime(&timer_wat(init, true));

    advance_to(&mut runtime, 1_000);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(1));
    assert_eq!(
        runtime.call_export_i32("second_ticks"),
        Some(0),
        "the tick taken before the change belongs to the old period"
    );

    advance_to(&mut runtime, 3_999);
    assert_eq!(runtime.call_export_i32("second_ticks"), Some(0));
    advance_to(&mut runtime, 4_000);
    assert_eq!(runtime.call_export_i32("second_ticks"), Some(1));
}

#[test]
fn a_period_change_from_on_wake_drops_the_tick_due_at_wake() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer construct");
    let init = "(global.set $first (call $start (i32.const 1000)))
        (global.set $slow_first_on_wake (i32.const 1))";
    let mut runtime = runtime(&timer_wat(init, true));
    runtime.notify_dormant();
    advance_with_renderer(&mut runtime, &mut renderer, 500);

    runtime.notify_wake();
    advance_with_renderer(&mut runtime, &mut renderer, 1_000);
    assert_eq!(runtime.call_export_i32("wakes"), Some(1));
    assert_eq!(
        runtime.call_export_i32("first_ticks"),
        Some(0),
        "the tick due at wake belongs to the old period"
    );

    advance_with_renderer(&mut runtime, &mut renderer, 5_999);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(0));
    advance_with_renderer(&mut runtime, &mut renderer, 6_000);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(1));
}

#[test]
fn a_refused_period_change_leaves_the_timer_running() {
    let mut runtime = runtime(&timer_wat(ONE_SECOND_TIMER, true));

    assert_eq!(
        runtime.call_export_i32("speed_first_past_the_minimum"),
        Some(0)
    );

    advance_to(&mut runtime, 1_000);
    assert_eq!(runtime.call_export_i32("first_ticks"), Some(1));
}

#[test]
fn a_timer_past_the_cap_is_refused() {
    let init = "(global.set $first (call $start (i32.const 1000)))
        (global.set $second (call $start (i32.const 1000)))";
    let mut runtime = runtime_with_cap(&timer_wat(init, true), 1);

    assert_ne!(
        runtime.call_export_i32("first"),
        Some(0),
        "the first timer fits the cap"
    );
    assert_eq!(
        runtime.call_export_i32("second"),
        Some(0),
        "the second timer is refused"
    );
}

#[test]
fn a_period_below_the_configured_minimum_is_refused() {
    let limits = RuntimeResourceLimits {
        min_timer_period_ms: NonZeroU32::new(5_000).expect("BUG: 5000 is non-zero"),
        ..RuntimeResourceLimits::default()
    };
    let mut runtime = runtime_with_limits(&timer_wat(ONE_SECOND_TIMER, true), limits);

    assert_eq!(runtime.call_export_i32("first"), Some(0));
}

#[test]
fn a_widget_without_the_on_timer_export_cannot_start_a_timer() {
    let logs = capture_logs(|| {
        let mut runtime = runtime(&timer_wat(ONE_SECOND_TIMER, false));
        assert_eq!(runtime.call_export_i32("first"), Some(0));
    });

    assert_eq!(
        count_lines(&logs, "WARN", "exports no `__on_timer(u32, u32)`"),
        1,
        "the refusal must say why:\n{logs}"
    );
}

#[test]
fn each_refusal_reason_warns_once_per_widget() {
    let init = "(global.set $first (call $start (i32.const 999)))
        (drop (call $start (i32.const 999)))
        (global.set $second (call $start (i32.const 1000)))
        (drop (call $start (i32.const 1000)))
        (drop (call $start (i32.const 1000)))
        (drop (call $set_period (global.get $second) (i32.const 999)))
        (drop (call $set_period (global.get $second) (i32.const 999)))";
    let wat = timer_wat(init, true);

    let logs = capture_logs(|| {
        let mut first_widget = runtime_with_cap(&wat, 1);
        let mut second_widget = runtime_with_cap(&wat, 1);
        assert_eq!(first_widget.call_export_i32("first"), Some(0));
        assert_eq!(second_widget.call_export_i32("first"), Some(0));
    });

    for reason in [
        "host_timer_start refused: period below the minimum",
        "host_timer_set_period refused: period below the minimum",
        "runtime timer limit reached",
    ] {
        assert_eq!(
            count_lines(&logs, "WARN", reason),
            2,
            "one warning per widget for `{reason}`, even after another reason warned:\n{logs}"
        );
        assert_eq!(
            count_lines(&logs, "DEBUG", reason),
            2,
            "repeats of `{reason}` drop to debug:\n{logs}"
        );
    }
}

fn count_lines(logs: &str, level: &str, message: &str) -> usize {
    logs.lines()
        .filter(|line| line.contains(level) && line.contains(message))
        .count()
}

/// Run `body` with the logs this thread emits during it collected into a string.
///
/// The subscriber is global and installed once.
/// tracing caches a callsite's interest process-wide on its first hit,
/// so a test running beside this one with no subscriber
/// would otherwise cache `never` for a line this one reads back.
fn capture_logs(body: impl FnOnce()) -> String {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(|| CaptureWriter(CAPTURE.with_borrow(Clone::clone)))
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("BUG: capture_logs installs the only global subscriber");
    });
    let buffer = Arc::new(Mutex::new(Vec::new()));
    CAPTURE.set(Some(Arc::clone(&buffer)));
    body();
    CAPTURE.set(None);
    let bytes = buffer
        .lock()
        .expect("BUG: the capture buffer is never held across a panic");
    String::from_utf8_lossy(&bytes).into_owned()
}

thread_local! {
    static CAPTURE: RefCell<Option<Arc<Mutex<Vec<u8>>>>> = const { RefCell::new(None) };
}

/// Appends to the emitting thread's capture buffer, dropping lines from threads without one.
struct CaptureWriter(Option<Arc<Mutex<Vec<u8>>>>);

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(buffer) = &self.0 {
            buffer
                .lock()
                .expect("BUG: the capture buffer is never held across a panic")
                .extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

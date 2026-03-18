// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime wrapper using wasmi.

#![expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Instant;

use anyhow::{Result, bail};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    BLACK, FormatPreferences, ICON_METER, RED_60, SDK_VERSION, SDK_VERSION_EXPORT, version_unpack,
};
use chrono::{DateTime, FixedOffset};
use wasmi::{Caller, Extern, Linker};

use crate::gpu::FemtoVgRenderer;
use crate::host_api::{FixtureEvent, FrameTimings, HostState};
use crate::renderer::Renderer;
use crate::tree::{self, TouchHit};

/// Write a `TouchHit` (4×f32 LE = 16 bytes) to WASM memory at `out_ptr`.
pub(super) fn write_touch_hit(caller: &mut Caller<'_, HostState>, out_ptr: u32, hit: &TouchHit) {
    let memory = caller.get_export("memory").and_then(Extern::into_memory);
    if let Some(memory) = memory {
        let data = memory.data_mut(caller);
        let start = out_ptr as usize;
        if start + 16 <= data.len() {
            data[start..start + 4].copy_from_slice(&hit.x.to_le_bytes());
            data[start + 4..start + 8].copy_from_slice(&hit.y.to_le_bytes());
            data[start + 8..start + 12].copy_from_slice(&hit.width.to_le_bytes());
            data[start + 12..start + 16].copy_from_slice(&hit.height.to_le_bytes());
        }
    }
}

/// Call the widget's `__bmc_sdk_version` export and validate against the host.
///
/// Returns the widget's `(major, minor, patch)` version on success.
/// Rejects on missing export or major version mismatch.
fn check_sdk_version(
    instance: wasmi::Instance,
    store: &mut wasmi::Store<HostState>,
) -> Result<(u16, u16, u16)> {
    let (major, minor, patch) = SDK_VERSION;

    let version_func = instance
        .get_typed_func::<(), u64>(&*store, SDK_VERSION_EXPORT)
        .map_err(|_| {
            anyhow::anyhow!(
                "widget missing '{SDK_VERSION_EXPORT}' export — \
             if using Rust SDK, update bmc-wasm-sdk; \
             otherwise export a `{SDK_VERSION_EXPORT}() -> u64` function \
             (packed major|minor<<16|patch<<32, host expects {major}.{minor}.{patch})"
            )
        })?;

    let packed = version_func.call(store, ())?;
    let widget_version = version_unpack(packed);
    let (w_major, w_minor, w_patch) = widget_version;

    if w_major != major {
        bail!(
            "SDK major version mismatch: widget is {w_major}.{w_minor}.{w_patch}, \
             host expects {major}.{minor}.{patch}"
        );
    }

    tracing::info!(
        "widget SDK version {w_major}.{w_minor}.{w_patch} \
         (host {major}.{minor}.{patch})"
    );
    Ok(widget_version)
}

/// Result of a single `render()` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    /// Frame rendered successfully within fuel budget.
    Ok,
    /// Widget exceeded its fuel budget this frame.
    /// The last good frame is shown with a warning indicator.
    FuelExhausted,
    /// Widget exceeded its budget too many times and has been killed.
    /// An error overlay is shown; WASM will not be called again
    /// until [`WasmWidgetRuntime::reset_fuel_state`] is called.
    Dead,
}

/// A callback that can intercept fetch requests before they hit the network.
/// Return `Some((status, body))` to short-circuit, `None` to proceed normally.
pub type FetchInterceptor = Box<dyn Fn(&str, &str) -> Option<(u32, Vec<u8>)>>;

/// A callback invoked when a fetch response is delivered.
/// Called with `(method_and_url, status, body)`.
pub type FetchObserver = Box<dyn Fn(&str, u32, &[u8])>;

/// Host-side limits for resources spawned on behalf of a widget.
pub use crate::runtime_limits::RuntimeResourceLimits;

/// Configuration for creating a [`WasmWidgetRuntime`].
///
/// All optional fields are applied **before** the WASM `init()` export runs,
/// so interceptors and KV are available from the widget's first instruction.
#[expect(missing_debug_implementations)]
pub struct RuntimeConfig {
    /// Instruction budget per frame (default: [`WasmWidgetRuntime::FUEL_PER_FRAME`]).
    pub fuel_per_frame: u64,
    /// Format preferences (12h/24h, date format, etc.).
    pub prefs: FormatPreferences,
    /// Key-value storage directory for this widget.
    pub kv_store_path: Option<std::path::PathBuf>,
    /// Intercept fetch requests before they hit the network.
    /// Return `Some((status, body))` to short-circuit, `None` to proceed normally.
    pub fetch_interceptor: Option<FetchInterceptor>,
    /// Called when a fetch response is delivered. Use for recording/logging.
    pub fetch_observer: Option<FetchObserver>,
    /// Enable recording of network events (SSDP, mDNS, WebSocket, etc.).
    /// Recorded events are drained via [`WasmWidgetRuntime::take_recorded_events`].
    pub record_events: bool,
    /// Pre-recorded event timeline for deterministic replay.
    pub event_fixtures: Vec<FixtureEvent>,
    /// Per-runtime caps for host-side resources such as fetches and sockets.
    pub resource_limits: RuntimeResourceLimits,
    /// MSAA samples used by the mesh atlas renderer. `0` disables mesh MSAA.
    pub mesh_msaa_samples: u32,
    /// Seed for the host RNG.
    ///
    /// - `None` keeps the default time-derived auto-seeding (the host picks a
    ///   non-zero seed from `monotonic_ms` on first use).
    /// - `Some(s)` honours the seed verbatim, including `Some(0)`. Note that
    ///   `Some(0)` makes the xorshift state stuck at zero (the RNG returns
    ///   `0` indefinitely); pick any non-zero seed for varied deterministic
    ///   output.
    pub rng_seed: Option<u64>,
    /// Sender for LED commands. Widgets call `led::set_effect()` etc., the host
    /// forwards commands through this channel. `None` = LED control unavailable.
    pub led_command_sender: Option<std::sync::mpsc::Sender<bmc_shared_led_data::LedCommand>>,
    /// Frame poll cadence (ms) used to clamp `frame_delay_ms` while host-side
    /// animations are active, so a widget's `request_frame_after(longer)`
    /// (e.g. a 1Hz clock tick) does not starve cached-tree animation replays.
    /// Defaults to [`Self::DEFAULT_ANIMATION_FRAME_DELAY_MS`] (~30 fps), which
    /// matches the Deck's hardware ceiling for 3D content. Hosts on faster
    /// targets can lower this toward 16 ms (60 fps) per the BDK-266 NFR.
    pub animation_frame_delay_ms: u32,
}

impl RuntimeConfig {
    /// Default animation cadence: ~30 fps (33 ms). Matches the BDK-355 mesh
    /// budget and the observed compositor rate on the Vivante GC400.
    pub const DEFAULT_ANIMATION_FRAME_DELAY_MS: u32 = 33;
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            fuel_per_frame: WasmWidgetRuntime::FUEL_PER_FRAME,
            prefs: FormatPreferences::default(),
            kv_store_path: None,
            fetch_interceptor: None,
            fetch_observer: None,
            record_events: false,
            event_fixtures: Vec::new(),
            resource_limits: RuntimeResourceLimits::default(),
            mesh_msaa_samples: 0,
            rng_seed: None,
            led_command_sender: None,
            animation_frame_delay_ms: Self::DEFAULT_ANIMATION_FRAME_DELAY_MS,
        }
    }
}

/// WebAssembly widget runtime.
///
/// Executes WASM modules in a sandboxed environment with fuel metering.
/// Owns the GPU renderer inside `HostState`.
#[expect(missing_debug_implementations)]
pub struct WasmWidgetRuntime {
    pub(super) store: wasmi::Store<HostState>,
    pub(super) instance: wasmi::Instance,
    render_func: wasmi::TypedFunc<u32, ()>,
    sdk_version: (u16, u16, u16),
    /// Instruction budget reset before each WASM frame execution.
    pub(super) fuel_per_frame: u64,
    /// Consecutive frames that exceeded the fuel budget.
    fuel_strikes: u32,
    /// Widget permanently stopped after exceeding [`Self::max_fuel_strikes`].
    fuel_dead: bool,
    /// How many consecutive fuel-outs before the widget is killed.
    max_fuel_strikes: u32,
    #[cfg(feature = "profiling")]
    wasm_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    wasm_every: ii_stopwatch::Every,
}

impl WasmWidgetRuntime {
    /// Maximum fuel (instructions) per frame.
    pub const FUEL_PER_FRAME: u64 = 10_000_000;

    /// Create a new runtime from WASM bytes and a GL function loader.
    ///
    /// All configuration from [`RuntimeConfig`] is applied **before** the WASM
    /// `init()` export runs, so interceptors, KV, and event fixtures are
    /// available from the widget's first instruction.
    ///
    /// # Safety
    /// `load_fn` must return valid OpenGL function pointers for the current GL context.
    pub unsafe fn new<F>(
        wasm_bytes: &[u8],
        load_fn: F,
        width: u32,
        height: u32,
        fbo_id: u32,
        config: RuntimeConfig,
    ) -> Result<Self>
    where
        F: FnMut(&str) -> *const c_void,
    {
        let RuntimeConfig {
            fuel_per_frame,
            prefs,
            kv_store_path,
            fetch_interceptor,
            fetch_observer,
            record_events,
            event_fixtures,
            resource_limits,
            mesh_msaa_samples,
            rng_seed,
            led_command_sender,
            animation_frame_delay_ms,
        } = config;

        let mut engine_config = wasmi::Config::default();
        engine_config.consume_fuel(true);
        engine_config.set_max_cached_stacks(4);
        // Disable Wasm proposals not used by our Rust-compiled widgets.
        // Saves validation/translation overhead.
        engine_config.wasm_tail_call(false);
        engine_config.wasm_multi_memory(false);
        engine_config.wasm_memory64(false);
        engine_config.wasm_extended_const(false);
        engine_config.wasm_custom_page_sizes(false);
        engine_config.wasm_wide_arithmetic(false);
        let engine = wasmi::Engine::new(&engine_config);
        let module = wasmi::Module::new(&engine, wasm_bytes)?;

        let renderer =
            unsafe { FemtoVgRenderer::new(load_fn, width, height, fbo_id, mesh_msaa_samples) }?;
        let host_state = HostState::new(renderer, prefs, resource_limits);

        let mut store = wasmi::Store::new(&engine, host_state);
        store.set_fuel(fuel_per_frame)?;

        let mut linker = Linker::new(&engine);
        Self::register_host_functions(&mut linker)?;

        let instance = linker.instantiate_and_start(&mut store, &module)?;
        let sdk_version = check_sdk_version(instance, &mut store)?;
        let render_func = instance.get_typed_func::<u32, ()>(&store, "render")?;

        // Apply config before init() so interceptors/KV are available immediately.
        let state = store.data_mut();
        state.kv_store_path = kv_store_path;
        state.fetch_interceptor = fetch_interceptor;
        state.fetch_observer = fetch_observer;
        state.record_events = record_events;
        state.rng_state = rng_seed;
        state.led_command_sender = led_command_sender;
        state.frame_schedule.animation_frame_delay_ms = animation_frame_delay_ms;
        if !event_fixtures.is_empty() {
            state.event_fixtures = Some(crate::host_api::FixtureEventState {
                events: event_fixtures,
                cursor: 0,
                ws_event_txs: HashMap::new(),
                socket_event_txs: HashMap::new(),
                mdns_event_txs: HashMap::new(),
                ssdp_event_txs: HashMap::new(),
                udp_event_txs: HashMap::new(),
            });
        }

        // Call init — all host config is in place
        if let Ok(init_func) = instance.get_typed_func::<(u32, u32), ()>(&store, "init") {
            init_func.call(&mut store, (width, height))?;
        }

        Ok(Self {
            store,
            instance,
            render_func,
            sdk_version,
            fuel_per_frame,
            fuel_strikes: 0,
            fuel_dead: false,
            max_fuel_strikes: 5,
            #[cfg(feature = "profiling")]
            wasm_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            wasm_every: ii_stopwatch::Every::new(std::time::Duration::from_secs(5)),
        })
    }

    fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
        super::imports::register_host_functions(linker)
    }

    /// Render a frame. Call `renderer().begin_frame()` before and `renderer().flush()` after.
    ///
    /// On animation-only frames (no pending input, host auto-requested),
    /// skips WASM execution and re-renders from cached tree data.
    ///
    /// Returns [`RenderStatus::FuelExhausted`] if the widget blew its budget
    /// (last good frame is shown with a warning bar). After
    /// [`Self::max_fuel_strikes`] consecutive fuel-outs the widget is killed
    /// and [`RenderStatus::Dead`] is returned on every subsequent call.
    pub fn render(&mut self, delta_ms: u32) -> Result<RenderStatus> {
        let state = self.store.data_mut();

        // Dead widget — show overlay on every frame.
        // Use `reset_fuel_state()` to revive (e.g. from a testbed button).
        if self.fuel_dead {
            state.interaction.begin_frame();
            state.begin_render_frame();
            Self::render_cached_tree(state, delta_ms);
            Self::draw_dead_overlay(state);
            return Ok(RenderStatus::Dead);
        }

        // Decide frame type BEFORE begin_frame consumes events
        let mut animation_only = state.frame_schedule.animation_only_frame
            && !state.interaction.has_pending_events()
            && state.cached_tree.is_some();

        // Check monotonic deadline for deferred WASM render (request_frame_after).
        // Uses monotonic_ms instead of delta_ms countdown because sub-millisecond
        // frames truncate delta_ms to 0 and stall countdown-based timers.
        if let Some(deadline_ms) = state.frame_schedule.deferred_wasm_render_at_ms
            && state.monotonic_ms >= deadline_ms
        {
            state.frame_schedule.deferred_wasm_render_at_ms = None;
            animation_only = false;
        }

        state.interaction.begin_frame();
        state.begin_render_frame();
        state.delta_ms = delta_ms;

        if animation_only {
            Self::render_cached_tree(state, delta_ms);
            return Ok(RenderStatus::Ok);
        }

        // Full WASM frame: compute real elapsed time since last WASM render
        // (not just the animation frame's ~0-16ms delta).
        let wasm_delta = (state.monotonic_ms - state.frame_schedule.last_wasm_render_at_ms) as u32;
        state.frame_schedule.last_wasm_render_at_ms = state.monotonic_ms;

        // Full frame: run WASM with per-frame fuel budget.
        self.store.set_fuel(self.fuel_per_frame)?;
        let wasm_t0 = Instant::now();
        ii_stopwatch::stopwatch_start!(self.wasm_w);
        let call_result = self.render_func.call(&mut self.store, wasm_delta);
        ii_stopwatch::stopwatch_stop!(self.wasm_w);

        #[cfg(feature = "profiling")]
        if ii_stopwatch::every_expired!(self.wasm_every) {
            let rss = crate::proc_mem::read_self_rss();
            let vm_rss_kb = rss.map_or(0, |s| s.vm_rss_kb);
            let rss_shmem_kb = rss.map_or(0, |s| s.rss_shmem_kb);
            tracing::info!(
                target: crate::profile::TARGET,
                "wasm_tick {wasm} vm_rss_kb={vm_rss_kb} rss_shmem_kb={rss_shmem_kb}",
                wasm = self.wasm_w,
            );
            ii_stopwatch::stopwatch_reset!(self.wasm_w);
        }

        match call_result {
            Ok(()) => {
                self.store.data_mut().last_timings.wasm_us = wasm_t0.elapsed().as_micros() as u32;
                self.fuel_strikes = 0;
                Ok(RenderStatus::Ok)
            }
            Err(e) if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) => {
                self.fuel_strikes += 1;
                tracing::warn!(
                    "widget exceeded fuel budget (strike {}/{})",
                    self.fuel_strikes,
                    self.max_fuel_strikes,
                );
                if self.fuel_strikes >= self.max_fuel_strikes {
                    self.fuel_dead = true;
                    let state = self.store.data_mut();
                    Self::render_cached_tree(state, delta_ms);
                    Self::draw_dead_overlay(state);
                    return Ok(RenderStatus::Dead);
                }
                // Show last good frame + warning bar, and request a
                // retry so the widget can run again with any state
                // changes that happened before the fuel trap.
                let state = self.store.data_mut();
                Self::render_cached_tree(state, delta_ms);
                Self::draw_fuel_warning(state, self.fuel_strikes, self.max_fuel_strikes);
                state.frame_schedule.frame_requested = true;
                Ok(RenderStatus::FuelExhausted)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Re-render the last successfully submitted tree (no WASM execution).
    ///
    /// Calls `layout_and_render` directly on the cached `TreeNode`, skipping
    /// deserialization entirely.
    fn render_cached_tree(state: &mut HostState, delta_ms: u32) {
        let Some((ref tree_node, width, height)) = state.cached_tree else {
            return;
        };
        let frame_counter = state.frame_counter;
        state.frame_counter += 1;
        let mut timings = FrameTimings::default();
        match tree::layout_and_render(
            tree_node,
            width,
            height,
            &mut state.renderer,
            &mut state.interaction,
            &mut state.modal_states,
            &mut state.scroll_states,
            &mut state.animation_states,
            &mut state.transition_states,
            frame_counter,
            delta_ms,
            &mut timings,
            &mut state.taffy,
        ) {
            Ok((result, has_active)) => {
                state.last_timings = timings;
                let had_interaction = !result.clicks.is_empty() || !result.drags.is_empty();
                // No WASM execution, no deserialization on cached frames
                state.tree_clicks = result.clicks;
                state.tree_drags = result.drags;
                if has_active || had_interaction {
                    state.frame_schedule.frame_requested = true;
                    state.frame_schedule.animation_only_frame = !had_interaction;
                }
                state.frame_schedule.has_active_animations = has_active;
                if has_active {
                    state.frame_schedule.frame_delay_ms = Some(
                        state
                            .frame_schedule
                            .frame_delay_ms
                            .map_or(state.frame_schedule.animation_frame_delay_ms, |delay_ms| {
                                delay_ms.min(state.frame_schedule.animation_frame_delay_ms)
                            }),
                    );
                }
            }
            Err(e) => {
                tracing::error!("cached tree render failed: {e}");
            }
        }
    }

    /// Subtle red bar at the top edge indicating fuel exhaustion.
    fn draw_fuel_warning(state: &mut HostState, strikes: u32, max_strikes: u32) {
        let w = state.renderer.width();
        let fraction = strikes as f32 / max_strikes as f32;
        let bar_w = w * fraction;
        // Red bar, increasingly opaque as strikes accumulate
        #[expect(clippy::cast_sign_loss)] // fraction is always 0..=1
        let alpha = (100.0 + 155.0 * fraction) as u8;
        let color = Color::from_rgba(0xFF, 0x00, 0x00, alpha);
        state.renderer.fill_rect(0.0, 0.0, bar_w, 3.0, color);
    }

    /// Full error overlay for a dead widget — CDS notification banner.
    fn draw_dead_overlay(state: &mut HostState) {
        let canvas_w = state.renderer.width();
        let canvas_h = state.renderer.height();

        let title = "This widget has been stopped";
        let subtitle = "It used too many resources and was suspended.";
        let banner_w = f32::clamp(canvas_w * 0.6, 250.0, 400.0);
        let banner_h =
            tree::measure_notification_banner(title, subtitle, banner_w, &mut state.renderer);

        // Semi-transparent dark scrim
        state
            .renderer
            .fill_rect(0.0, 0.0, canvas_w, canvas_h, BLACK.with_alpha(0.69));

        tree::render_notification_banner(
            title,
            subtitle,
            RED_60,
            ICON_METER,
            (canvas_w - banner_w) / 2.0,
            (canvas_h - banner_h) / 2.0,
            banner_w,
            banner_h,
            &mut state.renderer,
        );
    }

    /// Reset the fuel strike counter and dead state.
    ///
    /// Call this after hot-reloading a widget or when the host wants to
    /// give the widget another chance.
    pub fn reset_fuel_state(&mut self) {
        self.fuel_strikes = 0;
        self.fuel_dead = false;
    }

    /// Set the wall-clock time and monotonic clock for the next render.
    ///
    /// Must be called before each `render()`. The testbed sets these from real
    /// clocks; the capture binary increments by fixed 16ms steps.
    pub fn set_time(&mut self, system_time: DateTime<FixedOffset>, monotonic_ms: u64) {
        let state = self.store.data_mut();
        state.system_time = system_time;
        state.monotonic_ms = monotonic_ms;
        // Clamp so that wasm_delta doesn't underflow when time rewinds
        // (e.g. after a capture span resets the timeline cursor).
        state.frame_schedule.last_wasm_render_at_ms = state
            .frame_schedule
            .last_wasm_render_at_ms
            .min(monotonic_ms);
    }

    /// Access the GPU renderer (for begin_frame, flush, and testbed drawing).
    pub fn renderer(&mut self) -> &mut FemtoVgRenderer {
        &mut self.store.data_mut().renderer
    }

    /// Per-component timing breakdown from the last rendered frame.
    #[must_use]
    pub fn last_timings(&self) -> FrameTimings {
        self.store.data().last_timings
    }

    /// Whether the widget needs another frame rendered.
    ///
    /// Returns `true` after the widget calls `request_frame()` or
    /// `request_frame_after(ms)`, and also while cached-tree animations still
    /// require host-side replay frames. The host **must not** call
    /// [`Self::render`] when this returns `false` — doing so wastes CPU and GPU
    /// for an identical frame.
    ///
    /// When this returns `true`, check [`Self::next_frame_delay`] to see if
    /// the frame should be rendered immediately or after a delay.
    #[must_use]
    pub fn wants_next_frame(&self) -> bool {
        self.store.data().frame_schedule.frame_requested
    }

    /// Delay before the next host wake, if another frame was requested.
    ///
    /// Returns `None` for immediate frames (`request_frame()`), or `Some(ms)`
    /// for delayed wakes. This may be shorter than the widget's original
    /// `request_frame_after(ms)` delay while cached-tree animations are active;
    /// the widget's semantic full-WASM deadline remains tracked separately.
    ///
    /// The host should **sleep or schedule one timer** for the delay — not
    /// busy-wait or render immediately.
    #[must_use]
    pub fn next_frame_delay(&self) -> Option<u32> {
        self.store.data().frame_schedule.frame_delay_ms
    }

    /// Push a touch event to be processed next frame.
    pub fn push_touch_event(&mut self, event: crate::interaction::TouchEvent) {
        self.store.data_mut().interaction.push_event(event);
    }

    /// The SDK version the widget was compiled with (major, minor, patch).
    #[must_use]
    pub fn sdk_version(&self) -> (u16, u16, u16) {
        self.sdk_version
    }

    /// The SDK version the host expects (major, minor, patch).
    #[must_use]
    pub fn host_sdk_version() -> (u16, u16, u16) {
        SDK_VERSION
    }

    /// Look up the screen-space bounds of a registered UI element by string ID.
    ///
    /// Delegates to [`InteractionState::element_bounds`]. Must be called after
    /// a render pass (hit regions are rebuilt each frame).
    #[must_use]
    pub fn element_bounds(&self, id: &str) -> Option<crate::interaction::Rect> {
        self.store.data().interaction.element_bounds(id)
    }

    /// Return all registered hit region element IDs (sorted).
    #[must_use]
    pub fn element_ids(&self) -> Vec<&str> {
        self.store.data().interaction.element_ids()
    }

    /// Hit test against registered UI regions.
    ///
    /// Delegates to [`InteractionState::hit_test`]. Must be called after
    /// a render pass (hit regions are rebuilt each frame).
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
        self.store.data().interaction.hit_test(x, y)
    }

    /// Whether a deferred render is pending (widget called `request_frame_after`
    /// and the deadline hasn't been reached yet).
    #[must_use]
    pub fn has_deferred_render(&self) -> bool {
        let state = self.store.data();
        state
            .frame_schedule
            .deferred_wasm_render_at_ms
            .is_some_and(|deadline| state.monotonic_ms < deadline)
    }

    /// Get the instance for additional exports.
    #[must_use]
    pub fn instance(&self) -> &wasmi::Instance {
        &self.instance
    }
}

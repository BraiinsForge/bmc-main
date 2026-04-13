// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM runtime wrapper using wasmi.

#![expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use bmc_wasm_protocol::{FormatPreferences, SDK_VERSION, SDK_VERSION_EXPORT, version_unpack};
use chrono::{DateTime, FixedOffset};
use wasmi::{Caller, Extern, Linker};

use crate::gpu::FemtoVgRenderer;
use crate::host_api::{
    ActiveSocket, FixtureEvent, FrameTimings, HostState, HttpInboundRequest, HttpListenerResponse,
    MdnsEvent, SocketEvent, SocketOutbound, SsdpEvent, UdpBroadcastEvent, WsEvent, WsOutbound,
};
use crate::renderer::Renderer;
use crate::tree::{self, TouchHit};
use crate::xml::XmlDocumentIndex;

use super::memory::read_string;

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

        let renderer = unsafe { FemtoVgRenderer::new(load_fn, width, height, fbo_id) }?;
        let host_state = HostState::new(renderer, config.prefs, config.resource_limits);

        let mut store = wasmi::Store::new(&engine, host_state);
        store.set_fuel(config.fuel_per_frame)?;

        let mut linker = Linker::new(&engine);
        Self::register_host_functions(&mut linker)?;

        let instance = linker.instantiate_and_start(&mut store, &module)?;
        let sdk_version = check_sdk_version(instance, &mut store)?;
        let render_func = instance.get_typed_func::<u32, ()>(&store, "render")?;

        // Apply config before init() so interceptors/KV are available immediately.
        let state = store.data_mut();
        state.kv_store_path = config.kv_store_path;
        state.fetch_interceptor = config.fetch_interceptor;
        state.fetch_observer = config.fetch_observer;
        state.record_events = config.record_events;
        if !config.event_fixtures.is_empty() {
            state.event_fixtures = Some(crate::host_api::FixtureEventState {
                events: config.event_fixtures,
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
            fuel_per_frame: config.fuel_per_frame,
            fuel_strikes: 0,
            fuel_dead: false,
            max_fuel_strikes: 5,
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
        let mut animation_only = state.animation_only_frame
            && !state.interaction.has_pending_events()
            && state.cached_tree.is_some();

        // Check monotonic deadline for deferred WASM render (request_frame_after).
        // Uses monotonic_ms instead of delta_ms countdown because sub-millisecond
        // frames truncate delta_ms to 0 and stall countdown-based timers.
        if let Some(deadline_ms) = state.deferred_wasm_render_at_ms
            && state.monotonic_ms >= deadline_ms
        {
            state.deferred_wasm_render_at_ms = None;
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
        let wasm_delta = (state.monotonic_ms - state.last_wasm_render_at_ms) as u32;
        state.last_wasm_render_at_ms = state.monotonic_ms;

        // Full frame: run WASM with per-frame fuel budget.
        self.store.set_fuel(self.fuel_per_frame)?;
        let wasm_t0 = Instant::now();
        match self.render_func.call(&mut self.store, wasm_delta) {
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
                state.frame_requested = true;
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
                    state.frame_requested = true;
                    state.animation_only_frame = !had_interaction;
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
        let alpha = (100.0 + 155.0 * fraction) as u32;
        let color = 0xFF_00_00_00 | (alpha & 0xFF);
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
            .fill_rect(0.0, 0.0, canvas_w, canvas_h, 0x00_00_00_B0);

        tree::render_notification_banner(
            title,
            subtitle,
            bmc_wasm_protocol::RED_60,
            bmc_wasm_protocol::ICON_METER,
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
        state.last_wasm_render_at_ms = state.last_wasm_render_at_ms.min(monotonic_ms);
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
    /// `request_frame_after(ms)`. The host **must not** call [`Self::render`]
    /// when this returns `false` — doing so wastes CPU and GPU for an
    /// identical frame.
    ///
    /// When this returns `true`, check [`Self::next_frame_delay`] to see if
    /// the frame should be rendered immediately or after a delay.
    #[must_use]
    pub fn wants_next_frame(&self) -> bool {
        self.store.data().frame_requested
    }

    /// Delay before the next frame, if the widget used `request_frame_after(ms)`.
    ///
    /// Returns `None` for immediate frames (`request_frame()`), or `Some(ms)`
    /// for delayed frames. The host should **sleep or schedule a timer** for the
    /// delay — not busy-wait or render immediately.
    #[must_use]
    pub fn next_frame_delay(&self) -> Option<u32> {
        self.store.data().frame_delay_ms
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
    pub fn hit_test(&self, x: i32, y: i32) -> Option<String> {
        self.store.data().interaction.hit_test(x, y)
    }

    /// Whether a deferred render is pending (widget called `request_frame_after`
    /// and the deadline hasn't been reached yet).
    #[must_use]
    pub fn has_deferred_render(&self) -> bool {
        let state = self.store.data();
        state
            .deferred_wasm_render_at_ms
            .is_some_and(|deadline| state.monotonic_ms < deadline)
    }

    /// Get the instance for additional exports.
    #[must_use]
    pub fn instance(&self) -> &wasmi::Instance {
        &self.instance
    }
}

pub(super) fn xml_lookup_text(
    xml_docs: &HashMap<u32, XmlDocumentIndex>,
    doc_id: u32,
    path: &str,
) -> Option<String> {
    xml_docs.get(&doc_id)?.get_str(path).map(str::to_owned)
}

/// Maximum decoded image size accepted by `host_decode_image` (RGBA pixels).
const MAX_DECODE_IMAGE_PIXELS: u64 = 4_194_304;
/// Maximum decoder allocation budget accepted by `host_decode_image`.
///
/// This is intentionally slightly above the 8-bit RGBA output budget so common
/// decoders can keep modest working buffers, while still rejecting high
/// bit-depth images before they allocate substantially larger intermediates.
const MAX_DECODE_IMAGE_ALLOC_BYTES: u64 = 24 * 1024 * 1024;

pub(super) fn validate_kv_key(key: &str) -> Result<()> {
    if key.is_empty() || key.contains('/') || key.contains('\\') || key.contains("..") {
        bail!("invalid KV key");
    }
    Ok(())
}

pub(super) fn kv_disk_path(base: &std::path::Path, key: &str) -> Result<std::path::PathBuf> {
    validate_kv_key(key)?;
    Ok(base.join(key))
}

fn rgba_byte_len_limited(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    anyhow::ensure!(
        pixels <= MAX_DECODE_IMAGE_PIXELS,
        "decoded image exceeds pixel budget ({pixels} > {MAX_DECODE_IMAGE_PIXELS})"
    );
    let bytes = pixels
        .checked_mul(4)
        .expect("BUG: RGBA byte count overflow after pixel budget check");
    usize::try_from(bytes).map_err(Into::into)
}

fn probe_image_dimensions(data: &[u8]) -> Result<(u32, u32)> {
    match std::panic::catch_unwind(|| {
        image::ImageReader::new(std::io::Cursor::new(data))
            .with_guessed_format()
            .map_err(image::ImageError::IoError)
            .and_then(image::ImageReader::into_dimensions)
    }) {
        Ok(Ok(dimensions)) => Ok(dimensions),
        Ok(Err(e)) => Err(anyhow::anyhow!("{e}")),
        Err(_) => bail!("decoder panicked while probing dimensions"),
    }
}

pub(super) fn decode_image_rgba_limited(data: &[u8]) -> Result<image::RgbaImage> {
    let (width, height) = probe_image_dimensions(data)?;
    let _ = rgba_byte_len_limited(width, height)?;
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(MAX_DECODE_IMAGE_ALLOC_BYTES);

    match std::panic::catch_unwind(|| {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(data));
        reader.limits(limits);
        reader
            .with_guessed_format()
            .map_err(image::ImageError::IoError)
            .and_then(image::ImageReader::decode)
    }) {
        Ok(Ok(img)) => Ok(img.to_rgba8()),
        Ok(Err(e)) => Err(anyhow::anyhow!("{e}")),
        Err(_) => bail!("decoder panicked"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TlsVerificationMode {
    Full,
    Insecure,
}

pub(super) fn host_tls_connect_impl(
    caller: &mut Caller<'_, HostState>,
    host_ptr: u32,
    host_len: u32,
    port: u32,
    verification_mode: TlsVerificationMode,
) -> u32 {
    let host = read_string(caller, host_ptr, host_len);
    let Some(host) = host else { return 0 };

    let state = caller.data_mut();
    if state.sockets.len() >= state.resource_limits.max_sockets {
        tracing::warn!(
            max_sockets = state.resource_limits.max_sockets,
            "TLS connect rejected: runtime socket limit reached"
        );
        return 0;
    }
    let socket_id = state.next_socket_id;
    state.next_socket_id += 1;

    let (event_tx, event_rx) = std::sync::mpsc::channel::<SocketEvent>();

    if let Some(fixtures) = &mut state.event_fixtures {
        // Stub mode: no background thread, fixture replay injects events
        let (write_tx, _write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
        state
            .sockets
            .insert(socket_id, ActiveSocket { write_tx, event_rx });
        fixtures.socket_event_txs.insert(socket_id, event_tx);
    } else {
        let (write_tx, write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
        state
            .sockets
            .insert(socket_id, ActiveSocket { write_tx, event_rx });
        let port = port as u16;
        std::thread::spawn(move || {
            tls_background_thread(
                socket_id,
                &host,
                port,
                verification_mode,
                event_tx,
                write_rx,
            );
        });
    }

    socket_id
}

fn build_tls_client_config(verification_mode: TlsVerificationMode) -> Result<rustls::ClientConfig> {
    use std::sync::Arc;

    let crypto_provider = rustls::crypto::ring::default_provider();
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(crypto_provider))
        .with_safe_default_protocol_versions()?;

    let config = match verification_mode {
        TlsVerificationMode::Full => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        TlsVerificationMode::Insecure => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_no_client_auth(),
    };

    Ok(config)
}

/// Background thread for a single WebSocket connection.
///
/// Connects to `url` with optional extra headers, then runs a loop that
/// interleaves reading inbound messages (with a 50 ms read timeout to avoid
/// blocking forever) and draining outbound messages from `msg_rx`.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
pub(super) fn ws_background_thread(
    ws_id: u32,
    url: &str,
    headers: &[(String, String)],
    event_tx: std::sync::mpsc::Sender<WsEvent>,
    msg_rx: std::sync::mpsc::Receiver<WsOutbound>,
) {
    use tungstenite::http::Request;
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, connect};

    // Connect — use the plain URL when there are no custom headers so tungstenite
    // generates the required WebSocket handshake headers automatically. When extra
    // headers are needed, build a Request from the ClientRequestUri which adds them.
    let connect_result = if headers.is_empty() {
        connect(url)
    } else {
        let uri: tungstenite::http::Uri = match url.parse() {
            Ok(u) => u,
            Err(e) => {
                tracing::error!(ws_id, "WS bad URL: {e}");
                let _ = event_tx.send(WsEvent::Close(1002));
                return;
            }
        };
        let mut request = Request::builder()
            .uri(&uri)
            .header(
                "Host",
                uri.authority()
                    .map_or_else(|| "localhost".to_owned(), ToString::to_string),
            )
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            );
        for (k, v) in headers {
            request = request.header(k.as_str(), v.as_str());
        }
        let request = match request.body(()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(ws_id, "WS bad request: {e}");
                let _ = event_tx.send(WsEvent::Close(1002));
                return;
            }
        };
        connect(request)
    };

    let (mut socket, _response) = match connect_result {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(ws_id, "WS connect failed: {e}");
            let _ = event_tx.send(WsEvent::Close(1006));
            return;
        }
    };

    // Set a short read timeout so we can periodically check for outbound messages
    // instead of blocking forever on reads.
    if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
        let _ = tcp.set_read_timeout(Some(Duration::from_millis(50)));
    }

    let _ = event_tx.send(WsEvent::Open);
    tracing::info!(ws_id, %url, "WS connected");

    loop {
        // Drain all pending outbound messages
        loop {
            match msg_rx.try_recv() {
                Ok(WsOutbound::Text(text)) => {
                    if let Err(e) = socket.send(Message::Text(text)) {
                        tracing::warn!(ws_id, "WS send error: {e}");
                        let _ = event_tx.send(WsEvent::Close(1006));
                        return;
                    }
                }
                Ok(WsOutbound::Close) => {
                    let _ = socket.close(None);
                    let _ = event_tx.send(WsEvent::Close(1000));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(WsEvent::Close(1006));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        // Read one inbound message (blocks up to 50 ms due to read timeout)
        match socket.read() {
            Ok(Message::Text(text)) => {
                if event_tx.send(WsEvent::Message(text.into_bytes())).is_err() {
                    return;
                }
            }
            Ok(Message::Binary(data)) => {
                if event_tx.send(WsEvent::Message(data.clone())).is_err() {
                    return;
                }
            }
            Ok(Message::Close(frame)) => {
                let code = frame.map_or(1000, |f| f.code.into());
                let _ = event_tx.send(WsEvent::Close(code));
                tracing::info!(ws_id, code, "WS closed by server");
                return;
            }
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Read timeout expired — no data available, loop back to check outbound
            }
            Err(e) => {
                tracing::warn!(ws_id, "WS read error: {e}");
                break;
            }
        }
    }

    let _ = event_tx.send(WsEvent::Close(1006));
    tracing::info!(ws_id, "WS background thread exiting");
}

/// Background thread for a plain TCP socket connection.
///
/// Connects to `host:port`, then loops: drain outbound writes from
/// `write_rx`, read inbound data with a 50 ms timeout.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
pub(super) fn tcp_background_thread(
    socket_id: u32,
    host: &str,
    port: u16,
    event_tx: std::sync::mpsc::Sender<SocketEvent>,
    write_rx: std::sync::mpsc::Receiver<SocketOutbound>,
) {
    use std::io::{Read as _, Write as _};

    let addr = format!("{host}:{port}");
    let mut tcp = match std::net::TcpStream::connect(&addr) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::error!(socket_id, %addr, "TCP connect failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    if let Err(e) = tcp.set_read_timeout(Some(Duration::from_millis(50))) {
        tracing::warn!(socket_id, "failed to set read timeout: {e}");
    }

    let _ = event_tx.send(SocketEvent::Connected);
    tracing::info!(socket_id, %addr, "TCP connected");

    let mut read_buf = vec![0_u8; 16_384];

    loop {
        // Drain outbound writes
        loop {
            match write_rx.try_recv() {
                Ok(SocketOutbound::Data(data)) => {
                    if let Err(e) = tcp.write_all(&data) {
                        tracing::warn!(socket_id, "TCP write error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                    if let Err(e) = tcp.flush() {
                        tracing::warn!(socket_id, "TCP flush error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                }
                Ok(SocketOutbound::Close) => {
                    let _ = event_tx.send(SocketEvent::Closed(0));
                    tracing::info!(socket_id, "TCP socket closed by widget");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(SocketEvent::Closed(1));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        // Read inbound data (blocks up to 50 ms due to read timeout)
        match tcp.read(&mut read_buf) {
            Ok(0) => {
                let _ = event_tx.send(SocketEvent::Closed(0));
                tracing::info!(socket_id, "TCP EOF");
                return;
            }
            Ok(n) => {
                if event_tx
                    .send(SocketEvent::Data(read_buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(socket_id, "TCP read error: {e}");
                let _ = event_tx.send(SocketEvent::Closed(1));
                return;
            }
        }
    }
}

/// Background thread for a single TLS socket connection.
///
/// Connects to `host:port` with TLS, then loops: drain outbound writes from
/// `write_rx`, read inbound data with a 50 ms timeout.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
fn tls_background_thread(
    socket_id: u32,
    host: &str,
    port: u16,
    verification_mode: TlsVerificationMode,
    event_tx: std::sync::mpsc::Sender<SocketEvent>,
    write_rx: std::sync::mpsc::Receiver<SocketOutbound>,
) {
    use std::io::{Read as _, Write as _};
    use std::sync::Arc;

    let config = match build_tls_client_config(verification_mode) {
        Ok(config) => config,
        Err(e) => {
            tracing::error!(socket_id, "TLS config error: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    // TCP connect
    let addr = format!("{host}:{port}");
    let tcp = match std::net::TcpStream::connect(&addr) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::error!(socket_id, %addr, "TCP connect failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let server_name = match rustls::pki_types::ServerName::try_from(host.to_owned()) {
        Ok(name) => name,
        Err(e) => {
            tracing::error!(socket_id, "invalid server name '{host}': {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    // TLS handshake
    let conn = match rustls::ClientConnection::new(Arc::new(config), server_name) {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(socket_id, "TLS handshake setup failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let mut tls = rustls::StreamOwned::new(conn, tcp);

    // Set a short read timeout for the underlying TCP stream so we can
    // periodically check for outbound writes.
    if let Err(e) = tls.sock.set_read_timeout(Some(Duration::from_millis(50))) {
        tracing::warn!(socket_id, "failed to set read timeout: {e}");
    }

    let _ = event_tx.send(SocketEvent::Connected);
    tracing::info!(socket_id, %addr, "TLS connected");

    let mut read_buf = vec![0_u8; 16_384];

    loop {
        // Drain outbound writes
        loop {
            match write_rx.try_recv() {
                Ok(SocketOutbound::Data(data)) => {
                    if let Err(e) = tls.write_all(&data) {
                        tracing::warn!(socket_id, "TLS write error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                    if let Err(e) = tls.flush() {
                        tracing::warn!(socket_id, "TLS flush error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                }
                Ok(SocketOutbound::Close) => {
                    let _ = event_tx.send(SocketEvent::Closed(0));
                    tracing::info!(socket_id, "TLS socket closed by widget");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(SocketEvent::Closed(1));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        // Read inbound data (blocks up to 50 ms due to read timeout)
        match tls.read(&mut read_buf) {
            Ok(0) => {
                // EOF — remote closed
                let _ = event_tx.send(SocketEvent::Closed(0));
                tracing::info!(socket_id, "TLS EOF");
                return;
            }
            Ok(n) => {
                if event_tx
                    .send(SocketEvent::Data(read_buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(socket_id, "TLS read error: {e}");
                let _ = event_tx.send(SocketEvent::Closed(1));
                return;
            }
        }
    }
}

/// Certificate verifier that accepts all certificates.
///
/// Only used by the explicit insecure TLS socket path for trusted LAN devices.
#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Background thread for mDNS browse sessions.
///
/// Polls all registered service type receivers and forwards resolved
/// service events as JSON to the host state.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(super) fn mdns_browse_thread(
    service_types: Vec<String>,
    event_tx: std::sync::mpsc::Sender<MdnsEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("mDNS daemon creation failed: {e}");
            return;
        }
    };

    let receivers: Vec<_> = service_types
        .iter()
        .filter_map(|st| match daemon.browse(st) {
            Ok(rx) => Some((st.clone(), rx)),
            Err(e) => {
                tracing::error!("mDNS browse({st}) failed: {e}");
                None
            }
        })
        .collect();

    if receivers.is_empty() {
        let _ = daemon.shutdown();
        return;
    }

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        for (_, rx) in &receivers {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        // Build JSON with service details
                        let svc_type = info.ty_domain.clone();
                        let name = info.get_fullname().to_owned();
                        let port = info.get_port();
                        // Get first address (prefer IPv4)
                        let host = info
                            .get_addresses_v4()
                            .iter()
                            .next()
                            .map(ToString::to_string)
                            .unwrap_or_default();

                        // Build TXT as JSON object
                        let txt_pairs: Vec<String> = info
                            .get_properties()
                            .iter()
                            .map(|p| {
                                let k = p.key();
                                let v = p.val_str();
                                format!("\"{}\":\"{}\"", escape_json(k), escape_json(v))
                            })
                            .collect();
                        let txt_json = format!("{{{}}}", txt_pairs.join(","));

                        let json = format!(
                            "{{\"service_type\":\"{}\",\"name\":\"{}\",\"host\":\"{}\",\"port\":{},\"txt\":{}}}",
                            escape_json(&svc_type),
                            escape_json(&name),
                            escape_json(&host),
                            port,
                            txt_json,
                        );
                        if event_tx.send(MdnsEvent::Found(json)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        if event_tx.send(MdnsEvent::Removed(fullname)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::SearchStarted(_)
                    | ServiceEvent::ServiceFound(_, _)
                    | ServiceEvent::SearchStopped(_)
                    | _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = daemon.shutdown();
}

/// Escape a string for JSON output (quotes and backslashes).
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Background thread for SSDP M-SEARCH discovery.
///
/// Sends M-SEARCH multicast requests and listens for UPnP device responses.
/// For each responding device, fetches and parses the device description XML
/// to extract control URLs, then delivers pre-parsed JSON events.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(super) fn ssdp_search_thread(
    search_target: String,
    timeout_secs: u32,
    event_tx: std::sync::mpsc::Sender<SsdpEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::collections::HashSet;
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    let multicast_group = Ipv4Addr::new(239, 255, 255, 250);
    let multicast_addr = SocketAddrV4::new(multicast_group, 1900);

    // Socket for M-SEARCH (ephemeral port, receives unicast responses)
    let search_socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SSDP: failed to bind search socket: {e}");
            return;
        }
    };
    if let Err(e) = search_socket.set_read_timeout(Some(Duration::from_millis(250))) {
        tracing::error!("SSDP: failed to set search socket timeout: {e}");
        return;
    }

    // Socket for NOTIFY listener (multicast group on port 1900, receives byebye/alive).
    // Port 1900 may already be in use — that's fine, NOTIFY listener is best-effort.
    let notify_socket: Option<UdpSocket> =
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 1900))
            .ok()
            .and_then(|sock| {
                if let Err(e) = sock.join_multicast_v4(&multicast_group, &Ipv4Addr::UNSPECIFIED) {
                    tracing::warn!("SSDP: failed to join multicast group: {e}");
                    return None;
                }
                let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
                Some(sock)
            });

    let mut seen_usns: HashSet<String> = HashSet::new();
    let overall_timeout = Duration::from_secs(u64::from(timeout_secs).max(3));
    let resend_interval = Duration::from_secs(30);
    let mut last_send = Instant::now()
        .checked_sub(resend_interval)
        .expect("BUG: system clock too close to epoch for SSDP interval");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Send M-SEARCH periodically
        if last_send.elapsed() >= resend_interval {
            let request = format!(
                "M-SEARCH * HTTP/1.1\r\n\
                 HOST: 239.255.255.250:1900\r\n\
                 MAN: \"ssdp:discover\"\r\n\
                 MX: {timeout_secs}\r\n\
                 ST: {search_target}\r\n\r\n"
            );
            if let Err(e) = search_socket.send_to(request.as_bytes(), multicast_addr) {
                tracing::warn!("SSDP: M-SEARCH send failed: {e}");
            } else {
                tracing::debug!("SSDP: sent M-SEARCH for {search_target}");
            }
            last_send = Instant::now();
        }

        // Listen for M-SEARCH responses within the search window
        let listen_deadline = Instant::now() + overall_timeout;
        let mut buf = [0_u8; 4096];
        while Instant::now() < listen_deadline {
            if stop_rx.try_recv().is_ok() {
                return;
            }

            // Poll search socket for M-SEARCH responses
            if let Ok((n, _addr)) = search_socket.recv_from(&mut buf) {
                let response = String::from_utf8_lossy(&buf[..n]);
                if let Some(event) = ssdp_handle_response(&response, &search_target, &mut seen_usns)
                    && event_tx.send(event).is_err()
                {
                    return;
                }
            }

            // Poll notify socket for NOTIFY messages (byebye / alive)
            if let Some(ref sock) = notify_socket
                && let Ok((n, _addr)) = sock.recv_from(&mut buf)
            {
                let msg = String::from_utf8_lossy(&buf[..n]);
                if let Some(event) = ssdp_handle_notify(&msg, &search_target, &mut seen_usns)
                    && event_tx.send(event).is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Background thread for UDP broadcast: sends a broadcast message and collects responses.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(super) fn udp_broadcast_thread(
    port: u32,
    message: String,
    timeout_secs: u32,
    event_tx: std::sync::mpsc::Sender<UdpBroadcastEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, port as u16);

    let socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("UDP broadcast: failed to bind socket: {e}");
            return;
        }
    };
    if let Err(e) = socket.set_broadcast(true) {
        tracing::error!("UDP broadcast: failed to set broadcast: {e}");
        return;
    }
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(250))) {
        tracing::error!("UDP broadcast: failed to set read timeout: {e}");
        return;
    }

    let resend_interval = Duration::from_secs(30);
    let listen_window = Duration::from_secs(u64::from(timeout_secs).max(3));
    let mut last_send = Instant::now()
        .checked_sub(resend_interval)
        .expect("BUG: system clock too close to epoch for UDP broadcast interval");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Send broadcast periodically
        if last_send.elapsed() >= resend_interval {
            if let Err(e) = socket.send_to(message.as_bytes(), broadcast_addr) {
                tracing::warn!("UDP broadcast: send failed: {e}");
            } else {
                tracing::debug!("UDP broadcast: sent to port {port}");
            }
            last_send = Instant::now();
        }

        // Listen for responses
        let deadline = Instant::now() + listen_window;
        let mut buf = [0_u8; 4096];
        while Instant::now() < deadline {
            if stop_rx.try_recv().is_ok() {
                return;
            }
            if let Ok((n, addr)) = socket.recv_from(&mut buf)
                && let Ok(data) = std::str::from_utf8(&buf[..n])
            {
                let source = addr.to_string();
                if event_tx
                    .send(UdpBroadcastEvent::Response(data.to_owned(), source))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Handle an M-SEARCH response: extract LOCATION + USN, fetch description, return Found event.
fn ssdp_handle_response(
    response: &str,
    search_target: &str,
    seen_usns: &mut std::collections::HashSet<String>,
) -> Option<SsdpEvent> {
    // Verify the ST header matches our search target — devices may respond
    // with unrelated service types (e.g. upnp:rootdevice) to any M-SEARCH.
    let st = ssdp_extract_header(response, "ST")?;
    if st != search_target {
        return None;
    }

    let location = ssdp_extract_header(response, "LOCATION")?;
    let usn = ssdp_extract_header(response, "USN")?;

    if seen_usns.contains(&usn) {
        return None;
    }
    seen_usns.insert(usn.clone());

    tracing::debug!("SSDP: discovered USN={usn} at {location}");

    if let Some(json) = ssdp_fetch_description(&location) {
        return Some(SsdpEvent::Found(json));
    }
    tracing::warn!("SSDP: failed to parse description from {location}");
    None
}

/// Handle an SSDP NOTIFY message: detect `ssdp:byebye` for removal, `ssdp:alive` for discovery.
fn ssdp_handle_notify(
    msg: &str,
    search_target: &str,
    seen_usns: &mut std::collections::HashSet<String>,
) -> Option<SsdpEvent> {
    // Only process NOTIFY messages
    if !msg.starts_with("NOTIFY") {
        return None;
    }

    let nts = ssdp_extract_header(msg, "NTS")?;
    let usn = ssdp_extract_header(msg, "USN")?;
    let nt = ssdp_extract_header(msg, "NT").unwrap_or_default();

    // Only process events matching our search target
    if !nt.contains(search_target) && !usn.contains(search_target) {
        return None;
    }

    if nts == "ssdp:byebye" {
        tracing::debug!("SSDP: byebye USN={usn}");
        seen_usns.remove(&usn);
        Some(SsdpEvent::Removed(usn))
    } else if nts == "ssdp:alive" {
        // Treat as discovery if not already seen
        let location = ssdp_extract_header(msg, "LOCATION")?;
        if seen_usns.contains(&usn) {
            return None;
        }
        seen_usns.insert(usn.clone());
        tracing::debug!("SSDP: alive USN={usn} at {location}");
        if let Some(json) = ssdp_fetch_description(&location) {
            return Some(SsdpEvent::Found(json));
        }
        tracing::warn!("SSDP: failed to parse description from {location}");
        None
    } else {
        None
    }
}

/// Extract a header value from an SSDP HTTP-like response (case-insensitive).
fn ssdp_extract_header(response: &str, header_name: &str) -> Option<String> {
    let header_lower = header_name.to_ascii_lowercase();
    for line in response.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.trim().to_ascii_lowercase() == header_lower
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// Fetch a UPnP device description XML and extract relevant fields as JSON.
fn ssdp_fetch_description(location: &str) -> Option<String> {
    let response = ureq::get(location).call().ok()?;
    let body = response.into_body().read_to_string().ok()?;
    let doc = roxmltree::Document::parse(&body).ok()?;
    let root = doc.root_element();

    // Extract friendlyName from <device>
    let device_elem = root.descendants().find(|n| n.has_tag_name("device"))?;
    let friendly_name = device_elem
        .descendants()
        .find(|n| n.has_tag_name("friendlyName"))
        .and_then(|n| n.text())
        .unwrap_or("Unknown");

    // Extract control URLs from <serviceList>
    let mut av_transport_path = String::new();
    let mut rendering_control_path = String::new();

    for service in device_elem
        .descendants()
        .filter(|n| n.has_tag_name("service"))
    {
        let svc_type = service
            .descendants()
            .find(|n| n.has_tag_name("serviceType"))
            .and_then(|n| n.text())
            .unwrap_or("");
        let control_url = service
            .descendants()
            .find(|n| n.has_tag_name("controlURL"))
            .and_then(|n| n.text())
            .unwrap_or("");

        if svc_type.contains("AVTransport") {
            control_url.clone_into(&mut av_transport_path);
        } else if svc_type.contains("RenderingControl") {
            control_url.clone_into(&mut rendering_control_path);
        }
    }

    // Extract host and port from the LOCATION URL
    // Format: http://host:port/path
    let url_body = location.strip_prefix("http://")?;
    let host_port = url_body.split('/').next()?;
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse::<u16>().ok()?)
    } else {
        (host_port, 80)
    };

    let json = format!(
        "{{\"usn\":\"\",\"location\":\"{}\",\"name\":\"{}\",\"host\":\"{}\",\"port\":{},\"av_transport_path\":\"{}\",\"rendering_control_path\":\"{}\"}}",
        escape_json(location),
        escape_json(friendly_name),
        escape_json(host),
        port,
        escape_json(&av_transport_path),
        escape_json(&rendering_control_path),
    );

    Some(json)
}

/// Background thread for an HTTP listener.
///
/// Accepts connections, parses simple HTTP/1.1 requests, and sends them
/// to the WASM runtime for processing. Responses come back via a per-request
/// channel stored in `HostState::http_response_txs`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(super) fn http_listener_thread(
    port: u16,
    request_tx: std::sync::mpsc::Sender<HttpInboundRequest>,
    stop_rx: std::sync::mpsc::Receiver<()>,
    port_report_tx: std::sync::mpsc::Sender<u16>,
) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("HTTP listener bind failed on port {port}: {e}");
            let _ = port_report_tx.send(0);
            return;
        }
    };
    listener
        .set_nonblocking(true)
        .expect("BUG: set_nonblocking failed");

    let actual_port = listener.local_addr().map_or(port, |a| a.port());
    let _ = port_report_tx.send(actual_port);
    tracing::info!("HTTP listener started on port {actual_port}");

    let mut next_req_id: u32 = 1;

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((mut stream, addr)) => {
                tracing::debug!("HTTP connection from {addr}");
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                // Parse HTTP/1.1 request (simple line-based)
                let mut reader = BufReader::new(&stream);

                // Request line: METHOD PATH HTTP/1.1
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
                if parts.len() < 2 {
                    continue;
                }
                let method = parts[0].to_owned();
                let path = parts[1].to_owned();

                // Headers
                let mut headers = String::new();
                let mut content_length: usize = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                        break;
                    }
                    if let Some(val) = line
                        .to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().to_owned())
                    {
                        content_length = val.parse().unwrap_or(0);
                    }
                    headers.push_str(&line);
                }

                // Body
                let mut body = vec![0_u8; content_length];
                if content_length > 0 {
                    let _ = reader.read_exact(&mut body);
                }

                let request_id = next_req_id;
                next_req_id += 1;

                // Create a response channel for this request. The sender goes
                // with the request to the WASM runtime; the receiver stays here
                // so we can block until WASM responds.
                let (resp_tx, resp_rx) = std::sync::mpsc::channel::<HttpListenerResponse>();

                let req = HttpInboundRequest {
                    request_id,
                    method,
                    path,
                    headers,
                    body,
                    response_tx: resp_tx,
                };

                if request_tx.send(req).is_err() {
                    break; // Listener was shut down
                }

                // Wait for WASM to send a response (with timeout)
                if let Ok(resp) = resp_rx.recv_timeout(Duration::from_secs(10)) {
                    let status_text = match resp.status {
                        204 => "No Content",
                        400 => "Bad Request",
                        404 => "Not Found",
                        500 => "Internal Server Error",
                        _ => "OK",
                    };
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n{}\r\n",
                        resp.status,
                        status_text,
                        resp.body.len(),
                        resp.headers,
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&resp.body);
                    let _ = stream.flush();
                } else {
                    let response = "HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::error!("HTTP listener accept error: {e}");
                break;
            }
        }
    }
    tracing::info!("HTTP listener stopped on port {actual_port}");
}

/// Perform an HTTP request, returning (status_code, body).
/// Returns (0, error_message) on network errors.
pub(super) fn do_fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> (u32, Vec<u8>) {
    // Methods that accept a body (POST, PUT, PATCH) vs. bodyless (GET, DELETE, HEAD)
    let result = match method {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method {
                "POST" => ureq::post(url),
                "PUT" => ureq::put(url),
                _ => ureq::patch(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            match body {
                Some(bytes) => req.send(bytes),
                None => req.send_empty(),
            }
        }
        _ => {
            let mut req = match method {
                "DELETE" => ureq::delete(url),
                "HEAD" => ureq::head(url),
                _ => ureq::get(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.call()
        }
    };
    match result {
        Ok(response) => {
            let status = u32::from(response.status().as_u16());
            match response.into_body().read_to_vec() {
                Ok(body) => (status, body),
                Err(e) => (0, format!("body read error: {e}").into_bytes()),
            }
        }
        Err(ureq::Error::StatusCode(code)) => {
            // ureq 3 returns HTTP 4xx/5xx as Err(StatusCode) — pass through
            (u32::from(code), Vec::new())
        }
        Err(_) => (0, Vec::new()),
    }
}
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::Path;

    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba, RgbaImage};

    use crate::xml::XmlDocumentIndex;

    use super::{
        MAX_DECODE_IMAGE_PIXELS, TlsVerificationMode, build_tls_client_config,
        decode_image_rgba_limited, kv_disk_path, rgba_byte_len_limited, validate_kv_key,
        xml_lookup_text,
    };

    const XML_WIDGET_FEED: &str = r#"
        <rss>
            <channel>
                <item>
                    <title>Launch</title>
                    <pubDate>Sat, 12 Apr 2026 18:00:00 GMT</pubDate>
                    <ttl>15</ttl>
                    <res duration="00:01:02" />
                </item>
            </channel>
        </rss>
    "#;

    #[test]
    fn kv_key_validation_rejects_path_traversal_sequences() {
        assert!(validate_kv_key("../secret").is_err());
        assert!(validate_kv_key("subdir/key").is_err());
        assert!(validate_kv_key(r"subdir\key").is_err());
        assert!(validate_kv_key("").is_err());
        assert!(validate_kv_key("plain_key").is_ok());
    }

    #[test]
    fn kv_path_for_valid_key_stays_under_base_dir() {
        let base = Path::new("/tmp/widget-kv");
        let path = kv_disk_path(base, "pairing_guid").expect("BUG: valid key should resolve");

        assert_eq!(path, base.join("pairing_guid"));
    }

    #[test]
    fn rgba_budget_rejects_images_over_limit() {
        let mut side = 1_u32;
        while u64::from(side) * u64::from(side) <= MAX_DECODE_IMAGE_PIXELS {
            side += 1;
        }

        assert!(rgba_byte_len_limited(side, side).is_err());
    }

    #[test]
    fn decode_image_limited_accepts_small_png() {
        let img = RgbaImage::from_pixel(2, 2, Rgba([0x12, 0x34, 0x56, 0xFF]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("BUG: test PNG encoding should succeed");

        let rgba = decode_image_rgba_limited(&encoded.into_inner())
            .expect("BUG: small PNG should decode within budget");

        assert_eq!((rgba.width(), rgba.height()), (2, 2));
        assert_eq!(rgba.as_raw().len(), 16);
    }

    #[test]
    fn decode_image_limited_rejects_high_bit_depth_png_over_alloc_budget() {
        let img =
            ImageBuffer::from_pixel(2048, 2048, image::Rgba([0x1234, 0x5678, 0x9ABC, 0xFFFF]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba16(img)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("BUG: test PNG encoding should succeed");

        assert!(decode_image_rgba_limited(&encoded.into_inner()).is_err());
    }

    #[test]
    fn tls_client_config_builds_for_both_modes() {
        assert!(build_tls_client_config(TlsVerificationMode::Full).is_ok());
        assert!(build_tls_client_config(TlsVerificationMode::Insecure).is_ok());
    }

    #[test]
    fn xml_lookup_reads_multiple_fields_from_one_indexed_document() {
        let mut xml_docs = HashMap::new();
        let doc_id = 1;
        xml_docs.insert(
            doc_id,
            XmlDocumentIndex::from_xml(XML_WIDGET_FEED)
                .expect("BUG: test XML should build an index"),
        );

        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//title"),
            Some("Launch".to_owned())
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//pubDate"),
            Some("Sat, 12 Apr 2026 18:00:00 GMT".to_owned())
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//ttl").and_then(|value| value.parse::<f64>().ok()),
            Some(15.0)
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//res/@duration"),
            Some("00:01:02".to_owned())
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//title"),
            Some("Launch".to_owned())
        );

        xml_docs.remove(&doc_id);

        assert_eq!(xml_lookup_text(&xml_docs, doc_id, "//title"), None);
    }

    #[test]
    fn xml_lookup_f64_rejects_non_numeric_fields() {
        let mut xml_docs = HashMap::new();
        let doc_id = 1;
        xml_docs.insert(
            doc_id,
            XmlDocumentIndex::from_xml(XML_WIDGET_FEED)
                .expect("BUG: test XML should build an index"),
        );

        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//title")
                .and_then(|value| value.parse::<f64>().ok()),
            None
        );
    }
}

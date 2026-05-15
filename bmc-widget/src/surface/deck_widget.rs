// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fmt;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_surface, wl_touch},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1;

use bmc_widget_protocol::client::{
    deck_widget_manager_v1::DeckWidgetManagerV1,
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use bmc_widget_protocol::{ActionPayload, SettingUpdate, SizeType};
use wayland_client::WEnum;

use crate::egl::DmaBufInfo;
use crate::wayland::{from_protocol, to_protocol};

use super::common::{
    PollOutcome, blocking_dispatch_impl, create_buffer_from_dmabuf, impl_common_dispatch,
    invalidate_cached_wl_buffers, poll_dispatch, submit_buffer_to_surface,
};
use super::{WidgetEvent, WidgetSurface};

/// How long [`DeckWidgetSurfaceClient::connect`] waits for the compositor
/// to finish emitting the initial configure batch before giving up. A
/// working compositor produces `configure_done` synchronously on
/// `get_widget_surface`; this timeout guards against a crashed or
/// misconfigured compositor.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Events from the compositor to a `deck_widget_v1` widget.
#[derive(Debug, Clone)]
pub enum DeckWidgetEvent {
    /// A system setting changed at runtime.
    Setting(SettingUpdate),
    ParamUpdate(serde_json::Map<String, serde_json::Value>),
    /// Compositor requested graceful shutdown.
    Shutdown,
    /// Touch down from standard `wl_touch`.
    TouchDown {
        id: i32,
        x: f64,
        y: f64,
    },
    /// Touch motion from standard `wl_touch`.
    TouchMotion {
        id: i32,
        x: f64,
        y: f64,
    },
    /// Touch up from standard `wl_touch`.
    TouchUp {
        id: i32,
    },
    /// Touch cancelled from standard `wl_touch`.
    TouchCancel,
}

/// Initial configuration collected during the compositor's startup batch.
///
/// Populated between `get_widget_surface` and `configure_done`; returned
/// from [`DeckWidgetSurfaceClient::connect`] so the widget can build its
/// renderer against known geometry and params before entering the main
/// event loop.
///
/// `params` is the raw JSON object the compositor sent — widgets
/// deserialize it into their own `#[derive(Deserialize)]` struct.
#[derive(Debug, Clone)]
pub struct InitialState {
    pub size: SizeType,
    pub width: u32,
    pub height: u32,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub settings: Vec<SettingUpdate>,
}

/// Surface state for a `deck_widget_v1` widget with DMA-BUF support.
///
/// Tracks compositor globals, surface lifecycle, frame scheduling, and
/// pending protocol events. Mirrors [`crate::surface::XdgSurfaceState`] but
/// uses the `deck_widget_v1` protocol instead of XDG shell.
pub struct DeckWidgetSurfaceState {
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Current surface width in pixels.
    pub width: u32,
    /// Current surface height in pixels.
    pub height: u32,
    /// A frame callback fired -- widget should render.
    pub needs_render: bool,
    /// Number of frames rendered (wrapping counter).
    pub frame_count: u32,

    // -- Wayland objects (internal) --
    compositor: Option<wl_compositor::WlCompositor>,
    widget_manager: Option<DeckWidgetManagerV1>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    seat: Option<wl_seat::WlSeat>,
    touch: Option<wl_touch::WlTouch>,
    surface: Option<wl_surface::WlSurface>,
    widget_surface: Option<DeckWidgetSurfaceV1>,

    // -- Initial configure accumulation --
    /// Becomes `true` when the compositor emits `configure_done`. Used by
    /// [`DeckWidgetSurfaceClient::connect`] to stop blocking.
    configure_done: bool,
    /// Accumulated `configure` data (size class + dimensions). `None` until
    /// the compositor emits its `configure` event.
    pending_size: Option<(SizeType, u32, u32)>,
    /// Accumulated `params(json)` event (empty if the compositor sent
    /// `{}`).
    pending_params: serde_json::Map<String, serde_json::Value>,
    /// Accumulated setting events emitted before `configure_done`.
    pending_initial_settings: Vec<SettingUpdate>,

    pending_events: Vec<DeckWidgetEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TouchCapabilityChange {
    None,
    Acquire,
    Release,
}

/// Abstracts the `wl_touch.release` destructor so `sync_touch_capability`
/// can finalize a proxy without importing the real `WlTouch` type (which
/// keeps the helper unit-testable with a mock).
trait ReleasableTouch {
    fn release(&self);
}

impl ReleasableTouch for wl_touch::WlTouch {
    fn release(&self) {
        wl_touch::WlTouch::release(self);
    }
}

fn sync_touch_capability<T: ReleasableTouch>(
    touch: &mut Option<T>,
    has_touch_capability: bool,
) -> TouchCapabilityChange {
    match (has_touch_capability, touch.is_some()) {
        (true, false) => TouchCapabilityChange::Acquire,
        (true, true) | (false, false) => TouchCapabilityChange::None,
        (false, true) => {
            if let Some(t) = touch.take() {
                t.release();
            }
            TouchCapabilityChange::Release
        }
    }
}

impl DeckWidgetSurfaceState {
    /// Drain all pending protocol events.
    pub fn drain_events(&mut self) -> std::vec::Drain<'_, DeckWidgetEvent> {
        self.pending_events.drain(..)
    }
}

impl fmt::Debug for DeckWidgetSurfaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeckWidgetSurfaceState")
            .field("running", &self.running)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("needs_render", &self.needs_render)
            .field("frame_count", &self.frame_count)
            .field("configure_done", &self.configure_done)
            .field("pending_events", &self.pending_events.len())
            .finish_non_exhaustive()
    }
}

/// Single-connection Wayland client for `deck_widget_v1` widgets with DMA-BUF.
///
/// Handles connection, global binding, surface creation, frame callbacks, and
/// slot-based buffer submission using the `deck_widget_v1` protocol.
pub struct DeckWidgetSurfaceClient {
    conn: Connection,
    queue: EventQueue<DeckWidgetSurfaceState>,
    state: DeckWidgetSurfaceState,
    /// Per-slot cached `wl_buffer`s for double-buffered rendering.
    /// Same pattern as [`crate::surface::XdgSurfaceClient::cached_buffers`].
    cached_buffers: Vec<Option<wl_buffer::WlBuffer>>,
}

impl fmt::Debug for DeckWidgetSurfaceClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cached = self.cached_buffers.iter().filter(|b| b.is_some()).count();
        f.debug_struct("DeckWidgetSurfaceClient")
            .field("state", &self.state)
            .field(
                "cached_buffers",
                &format_args!("{cached}/{}", self.cached_buffers.len()),
            )
            .finish_non_exhaustive()
    }
}

impl DeckWidgetSurfaceClient {
    /// Connect to the Wayland display, create a `deck_widget_v1` surface,
    /// and block until the compositor has finished emitting its initial
    /// configure batch.
    ///
    /// Binds `wl_compositor`, `deck_widget_manager_v1`, and
    /// `zwp_linux_dmabuf_v1`, creates a surface, then waits for a
    /// `configure_done` event before returning. The returned
    /// [`InitialState`] carries size class, pixel dimensions, widget
    /// params, and any runtime settings the compositor already knew about
    /// at spawn time.
    pub fn connect() -> Result<(Self, InitialState)> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = DeckWidgetSurfaceState {
            running: true,
            width: 0,
            height: 0,
            needs_render: false,
            frame_count: 0,
            compositor: None,
            widget_manager: None,
            linux_dmabuf: None,
            seat: None,
            touch: None,
            surface: None,
            widget_surface: None,
            configure_done: false,
            pending_size: None,
            pending_params: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
            pending_events: Vec::new(),
        };

        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        let compositor = state
            .compositor
            .as_ref()
            .context("wl_compositor not available")?;
        let widget_manager = state
            .widget_manager
            .as_ref()
            .context("deck_widget_manager_v1 not available")?;
        anyhow::ensure!(
            state.linux_dmabuf.is_some(),
            "zwp_linux_dmabuf_v1 not available"
        );

        let surface = compositor.create_surface(&qh, ());
        let widget_surface = widget_manager.get_widget_surface(&surface, &qh, ());

        surface.commit();

        state.surface = Some(surface);
        state.widget_surface = Some(widget_surface);

        // Block until configure_done arrives. A correctly behaving
        // compositor sends it synchronously after get_widget_surface; the
        // poll-based wait pushes the deadline into `poll(2)` so a silent
        // compositor surfaces as `PollOutcome::Timeout` instead of an
        // indefinite hang in `blocking_dispatch`.
        let deadline = Instant::now() + CONFIGURE_TIMEOUT;
        while !state.configure_done {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let remaining_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            match poll_dispatch(&conn, &mut queue, &mut state, remaining_ms)
                .context("Wayland dispatch while awaiting configure_done")?
            {
                PollOutcome::Events => {}
                PollOutcome::Timeout => anyhow::bail!(
                    "timed out after {:?} waiting for configure_done",
                    CONFIGURE_TIMEOUT
                ),
            }
        }

        let (size, width, height) = state
            .pending_size
            .context("configure_done without prior configure event")?;
        state.width = width;
        state.height = height;

        let initial = InitialState {
            size,
            width,
            height,
            params: std::mem::take(&mut state.pending_params),
            settings: std::mem::take(&mut state.pending_initial_settings),
        };

        tracing::info!(
            "Deck widget surface ready: {}x{} size={:?} params={} settings={}",
            width,
            height,
            size,
            initial.params.len(),
            initial.settings.len(),
        );

        Ok((
            Self {
                conn,
                queue,
                state,
                cached_buffers: Vec::new(),
            },
            initial,
        ))
    }

    /// Get a reference to the surface state.
    #[must_use]
    pub fn state(&self) -> &DeckWidgetSurfaceState {
        &self.state
    }

    /// Submit a DMA-BUF frame for a reusable buffer slot.
    ///
    /// On first call for a given `slot`, creates a `wl_buffer` from the
    /// DMA-BUF info and caches it. Subsequent calls reuse the cached buffer,
    /// avoiding repeated `wl_buffer` creation overhead.
    ///
    /// Call [`invalidate_cached_buffers`](Self::invalidate_cached_buffers)
    /// when the surface is resized or the underlying DMA-BUF changes.
    pub fn submit_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> Result<()> {
        let qh = self.queue.handle();
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 not available")?;

        if slot >= self.cached_buffers.len() {
            self.cached_buffers.resize_with(slot + 1, || None);
        }

        if self.cached_buffers[slot].is_none() {
            let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
            self.cached_buffers[slot] = Some(buffer);
            tracing::debug!("Cached wl_buffer for slot {slot}");
        }

        let buffer = self.cached_buffers[slot]
            .as_ref()
            .expect("BUG: cached buffer should exist after creation above");
        self.submit_wl_buffer(buffer, info, request_frame)
    }

    /// Attach buffer, damage, optionally request frame callback, and commit.
    fn submit_wl_buffer(
        &self,
        buffer: &wl_buffer::WlBuffer,
        info: &DmaBufInfo,
        request_frame: bool,
    ) -> Result<()> {
        let qh = self.queue.handle();
        let surface = self.state.surface.as_ref().context("surface not created")?;
        submit_buffer_to_surface(surface, &qh, buffer, info, request_frame);
        Ok(())
    }

    /// Invalidate all cached buffer slots.
    ///
    /// Destroys any cached `wl_buffer`s. Call this when the surface is
    /// resized or when the underlying DMA-BUF export buffers are recreated.
    ///
    /// Late `wl_buffer::Release` events from previously cached buffers are
    /// harmless and ignored.
    pub fn invalidate_cached_buffers(&mut self) {
        invalidate_cached_wl_buffers(&mut self.cached_buffers);
    }

    /// Request the first frame callback (call once after setup, before the
    /// event loop).
    pub fn request_frame(&self) {
        let qh = self.queue.handle();
        if let Some(ref surface) = self.state.surface {
            surface.frame(&qh, ());
            surface.commit();
        }
    }

    /// Block until a Wayland event arrives, then dispatch all pending events.
    pub fn blocking_dispatch(&mut self) -> Result<()> {
        blocking_dispatch_impl(&mut self.queue, &mut self.state)
    }

    /// Poll for Wayland events with a timeout, then dispatch pending events.
    ///
    /// Returns `Ok(true)` on all normal paths (events read, timeout, or
    /// already-queued events dispatched). Returns `Ok(false)` only if a
    /// non-fatal `EAGAIN`/`EWOULDBLOCK` occurs in `poll(2)` or
    /// `ReadEventsGuard::read()`; pending events are still dispatched. A
    /// timeout of `-1` blocks indefinitely; `0` is non-blocking.
    ///
    /// This follows the `prepare_read -> poll -> read/cancel -> dispatch_pending`
    /// pattern required by `wayland-client`.
    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> Result<PollOutcome> {
        poll_dispatch(&self.conn, &mut self.queue, &mut self.state, timeout_ms)
    }

    /// Forward a typed widget action as a `deck_widget_v1` request.
    /// No-ops if the surface hasn't been created yet.
    pub fn request_action(&self, action: &ActionPayload) -> Result<()> {
        let Some(ref surface) = self.state.widget_surface else {
            return Ok(());
        };
        match action {
            ActionPayload::PlaySound { sound } => surface.play_sound(sound.clone()),
            ActionPayload::StopSound {} => surface.stop_sound(),
            ActionPayload::LedTemporary {
                request_id,
                effect,
                color,
                period_ms,
                duration_ms,
            } => surface.led_temporary(
                *request_id,
                to_protocol::led_effect(*effect),
                u32::from(color.r),
                u32::from(color.g),
                u32::from(color.b),
                *period_ms,
                *duration_ms,
            ),
            ActionPayload::LedEndless {
                request_id,
                effect,
                color,
                period_ms,
            } => surface.led_endless(
                *request_id,
                to_protocol::led_effect(*effect),
                u32::from(color.r),
                u32::from(color.g),
                u32::from(color.b),
                *period_ms,
            ),
            ActionPayload::StopLed { request_id } => surface.stop_led(*request_id),
        }
        self.conn.flush().context("Wayland flush after action")?;
        Ok(())
    }
}

impl WidgetSurface for DeckWidgetSurfaceClient {
    fn running(&self) -> bool {
        self.state.running
    }

    fn request_shutdown(&mut self) {
        self.state.running = false;
    }

    fn width(&self) -> u32 {
        self.state.width
    }

    fn height(&self) -> u32 {
        self.state.height
    }

    fn take_size_changed(&mut self) -> bool {
        false
    }

    fn needs_render(&self) -> bool {
        self.state.needs_render
    }

    fn take_render_requested(&mut self) -> bool {
        let requested = self.state.needs_render;
        self.state.needs_render = false;
        requested
    }

    fn mark_needs_render(&mut self) {
        self.state.needs_render = true;
    }

    fn frame_count(&self) -> u32 {
        self.state.frame_count
    }

    fn blocking_dispatch(&mut self) -> anyhow::Result<()> {
        DeckWidgetSurfaceClient::blocking_dispatch(self)
    }

    fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<PollOutcome> {
        DeckWidgetSurfaceClient::poll_dispatch(self, timeout_ms)
    }

    fn request_frame(&self) {
        DeckWidgetSurfaceClient::request_frame(self);
    }

    fn submit_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> anyhow::Result<()> {
        DeckWidgetSurfaceClient::submit_buffer(self, info, slot, request_frame)
    }

    fn invalidate_cached_buffers(&mut self) {
        DeckWidgetSurfaceClient::invalidate_cached_buffers(self);
    }

    fn drain_events(&mut self) -> Vec<WidgetEvent> {
        self.state
            .pending_events
            .drain(..)
            .map(|event| match event {
                DeckWidgetEvent::Setting(update) => WidgetEvent::Setting(update),
                DeckWidgetEvent::ParamUpdate(params) => WidgetEvent::ParamUpdate(params),
                DeckWidgetEvent::Shutdown => WidgetEvent::Shutdown,
                DeckWidgetEvent::TouchDown { id, x, y } => WidgetEvent::TouchDown { id, x, y },
                DeckWidgetEvent::TouchMotion { id, x, y } => WidgetEvent::TouchMotion { id, x, y },
                DeckWidgetEvent::TouchUp { id } => WidgetEvent::TouchUp { id },
                DeckWidgetEvent::TouchCancel => WidgetEvent::TouchCancel,
            })
            .collect()
    }
}

impl_common_dispatch!(DeckWidgetSurfaceState);

impl Dispatch<wl_registry::WlRegistry, ()> for DeckWidgetSurfaceState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    );
                    tracing::debug!("Bound wl_compositor v{}", version.min(6));
                    state.compositor = Some(compositor);
                }
                "deck_widget_manager_v1" => {
                    let widget_manager =
                        registry.bind::<DeckWidgetManagerV1, _, _>(name, version.min(1), qh, ());
                    tracing::debug!("Bound deck_widget_manager_v1 v{}", version.min(1));
                    state.widget_manager = Some(widget_manager);
                }
                "zwp_linux_dmabuf_v1" => {
                    let dmabuf = registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    tracing::debug!("Bound zwp_linux_dmabuf_v1 v{}", version.min(4));
                    state.linux_dmabuf = Some(dmabuf);
                }
                "wl_seat" if state.seat.is_none() => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(9), qh, ());
                    tracing::debug!("Bound wl_seat v{}", version.min(9));
                    state.seat = Some(seat);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<DeckWidgetManagerV1, ()> for DeckWidgetSurfaceState {
    fn event(
        _: &mut Self,
        _: &DeckWidgetManagerV1,
        _: <DeckWidgetManagerV1 as wayland_client::Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn size_type_from_protocol(w: WEnum<deck_widget_surface_v1::SizeType>) -> Option<SizeType> {
    match w.into_result().ok()? {
        deck_widget_surface_v1::SizeType::Small => Some(SizeType::Small),
        deck_widget_surface_v1::SizeType::Medium => Some(SizeType::Medium),
        deck_widget_surface_v1::SizeType::Large => Some(SizeType::Large),
        deck_widget_surface_v1::SizeType::Full => Some(SizeType::Full),
        _ => None,
    }
}

impl Dispatch<DeckWidgetSurfaceV1, ()> for DeckWidgetSurfaceState {
    fn event(
        state: &mut Self,
        _: &DeckWidgetSurfaceV1,
        event: deck_widget_surface_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            deck_widget_surface_v1::Event::Configure {
                size_type,
                width,
                height,
            } => {
                let size = size_type_from_protocol(size_type).unwrap_or(SizeType::Small);
                tracing::debug!("Configure: size={:?} {}x{}", size, width, height);
                state.pending_size = Some((size, width, height));
            }
            deck_widget_surface_v1::Event::Params { json } => {
                handle_params_json(
                    &mut state.pending_params,
                    &mut state.pending_events,
                    state.configure_done,
                    &json,
                );
            }
            deck_widget_surface_v1::Event::ConfigureDone => {
                tracing::debug!("ConfigureDone");
                state.configure_done = true;
            }
            deck_widget_surface_v1::Event::Timezone { value } => {
                tracing::debug!("Timezone update: {value}");
                let update = SettingUpdate::Timezone(value);
                if state.configure_done {
                    state.pending_events.push(DeckWidgetEvent::Setting(update));
                } else {
                    state.pending_initial_settings.push(update);
                }
            }
            deck_widget_surface_v1::Event::NightMode { value } => {
                if let Some(b) = from_protocol::night_mode(value) {
                    push_setting(state, SettingUpdate::NightMode(b));
                }
            }
            deck_widget_surface_v1::Event::DateFormat { value } => {
                if let Some(v) = from_protocol::date_format(value) {
                    push_setting(state, SettingUpdate::DateFormat(v));
                }
            }
            deck_widget_surface_v1::Event::TimeFormat { value } => {
                if let Some(v) = from_protocol::time_format(value) {
                    push_setting(state, SettingUpdate::TimeFormat(v));
                }
            }
            deck_widget_surface_v1::Event::NumberFormat { value } => {
                if let Some(v) = from_protocol::number_format(value) {
                    push_setting(state, SettingUpdate::NumberFormat(v));
                }
            }
            deck_widget_surface_v1::Event::TemperatureUnit { value } => {
                if let Some(v) = from_protocol::temperature_unit(value) {
                    push_setting(state, SettingUpdate::TemperatureUnit(v));
                }
            }
            deck_widget_surface_v1::Event::FirstDayOfWeek { value } => {
                if let Some(v) = from_protocol::weekday(value) {
                    push_setting(state, SettingUpdate::FirstDayOfWeek(v));
                }
            }
            deck_widget_surface_v1::Event::Shutdown => {
                tracing::info!("Shutdown requested by compositor");
                state.running = false;
                state.pending_events.push(DeckWidgetEvent::Shutdown);
            }
            deck_widget_surface_v1::Event::LedRequestStatus { request_id, status } => {
                tracing::debug!("Received led_request_status: req={request_id} status={status:?}");
            }
            _ => {}
        }
    }
}

/// Route a setting event to the initial batch (before `configure_done`)
/// or the runtime queue (after).
fn push_setting(state: &mut DeckWidgetSurfaceState, update: SettingUpdate) {
    if state.configure_done {
        state.pending_events.push(DeckWidgetEvent::Setting(update));
    } else {
        state.pending_initial_settings.push(update);
    }
}

fn push_params(
    pending_params: &mut serde_json::Map<String, serde_json::Value>,
    pending_events: &mut Vec<DeckWidgetEvent>,
    configure_done: bool,
    params: serde_json::Map<String, serde_json::Value>,
) {
    if configure_done {
        pending_events.push(DeckWidgetEvent::ParamUpdate(params));
    } else {
        *pending_params = params;
    }
}

fn handle_params_json(
    pending_params: &mut serde_json::Map<String, serde_json::Value>,
    pending_events: &mut Vec<DeckWidgetEvent>,
    configure_done: bool,
    json: &str,
) {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(map)) => {
            push_params(pending_params, pending_events, configure_done, map);
        }
        Ok(other) => {
            tracing::warn!("Params JSON is not an object, ignoring: {other}");
        }
        Err(e) => {
            tracing::warn!("Failed to decode params JSON ({}): {}", json, e);
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for DeckWidgetSurfaceState {
    fn event(
        _: &mut Self,
        _buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            // deck_widget surfaces only support slot-based cached wl_buffer
            // submission, so release is intentionally ignored.
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for DeckWidgetSurfaceState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(caps),
        } = event
        {
            match sync_touch_capability(&mut state.touch, caps.contains(wl_seat::Capability::Touch))
            {
                TouchCapabilityChange::Acquire => {
                    let touch = seat.get_touch(qh, ());
                    tracing::debug!("Acquired wl_touch from seat");
                    state.touch = Some(touch);
                }
                TouchCapabilityChange::Release => {
                    tracing::debug!("Dropped wl_touch after seat capability removal");
                }
                TouchCapabilityChange::None => {}
            }
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for DeckWidgetSurfaceState {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down { id, x, y, .. } => {
                state
                    .pending_events
                    .push(DeckWidgetEvent::TouchDown { id, x, y });
                state.needs_render = true;
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state
                    .pending_events
                    .push(DeckWidgetEvent::TouchMotion { id, x, y });
                state.needs_render = true;
            }
            wl_touch::Event::Up { id, .. } => {
                state.pending_events.push(DeckWidgetEvent::TouchUp { id });
                state.needs_render = true;
            }
            wl_touch::Event::Cancel => {
                state.pending_events.push(DeckWidgetEvent::TouchCancel);
                state.needs_render = true;
            }
            wl_touch::Event::Frame => {
                tracing::trace!("wl_touch::Frame");
            }
            wl_touch::Event::Shape { .. } => {
                tracing::trace!("wl_touch::Shape (ignored)");
            }
            wl_touch::Event::Orientation { .. } => {
                tracing::trace!("wl_touch::Orientation (ignored)");
            }
            other => {
                tracing::debug!(?other, "unhandled wl_touch event");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};
    use std::cell::Cell;
    use std::rc::Rc;

    use super::{
        DeckWidgetEvent, ReleasableTouch, TouchCapabilityChange, handle_params_json,
        sync_touch_capability,
    };

    struct MockTouch {
        released: Rc<Cell<bool>>,
    }

    impl ReleasableTouch for MockTouch {
        fn release(&self) {
            self.released.set(true);
        }
    }

    #[test]
    fn touch_capability_drop_releases_and_clears_existing_touch_handle() {
        let released = Rc::new(Cell::new(false));
        let mut touch = Some(MockTouch {
            released: Rc::clone(&released),
        });

        let change = sync_touch_capability(&mut touch, false);

        assert_eq!(change, TouchCapabilityChange::Release);
        assert!(touch.is_none());
        assert!(
            released.get(),
            "wl_touch.release() must be called before dropping"
        );
    }

    #[test]
    fn touch_capability_return_requests_reacquire() {
        let mut touch = None::<MockTouch>;

        let change = sync_touch_capability(&mut touch, true);

        assert_eq!(change, TouchCapabilityChange::Acquire);
        assert!(touch.is_none());
    }

    #[test]
    fn params_null_is_ignored_before_configure_done() {
        let mut pending_params = Map::new();
        pending_params.insert("keep".to_owned(), Value::String("value".to_owned()));
        let mut pending_events: Vec<DeckWidgetEvent> = Vec::new();

        handle_params_json(&mut pending_params, &mut pending_events, false, "null");

        assert_eq!(
            pending_params.get("keep"),
            Some(&Value::String("value".to_owned()))
        );
        assert!(pending_events.is_empty());
    }

    #[test]
    fn params_null_is_ignored_after_configure_done() {
        let mut pending_params = Map::new();
        let mut pending_events: Vec<DeckWidgetEvent> = Vec::new();

        handle_params_json(&mut pending_params, &mut pending_events, true, "null");

        assert!(pending_events.is_empty());
    }
}

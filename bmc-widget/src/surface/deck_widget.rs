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

use std::{
    fmt,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use wayland_backend::client::ObjectId;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_surface, wl_touch},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1;

use bmc_widget_protocol::client::{
    deck_widget_manager_v2::DeckWidgetManagerV2,
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use bmc_widget_protocol::{
    ActionPayload, NextAlarm, SettingUpdate, ViewportShape, WidgetInstanceKey, widget_key_from_env,
};
use wayland_client::WEnum;

use crate::egl::DmaBufInfo;
use crate::protocol::{from_protocol, to_protocol};

use super::common::{
    BufferSlotMap, PollOutcome, ReleasedBuffer, ReleasedBufferSet, blocking_dispatch_impl,
    create_buffer_from_dmabuf, drain_released_buffer_slots, drain_released_buffers,
    impl_common_dispatch, invalidate_cached_wl_buffer_slots, invalidate_cached_wl_buffers,
    poll_dispatch, record_released_buffer, submit_buffer_to_surface, unregister_wl_buffer_slot,
};
use super::{WidgetEvent, WidgetSurface};

/// How long [`DeckWidgetSurfaceClient::connect`] waits for the compositor
/// to finish emitting the initial configure batch before giving up. A
/// working compositor produces `configure_done` synchronously on
/// `get_widget_surface`; this timeout guards against a crashed or
/// misconfigured compositor.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Events from the compositor to a `deck_widget` widget.
#[derive(Debug, Clone)]
pub enum DeckWidgetEvent {
    /// A system setting changed at runtime.
    Setting(SettingUpdate),
    ParamUpdate(serde_json::Map<String, serde_json::Value>),
    CredentialsUpdate(serde_json::Map<String, serde_json::Value>),
    SecretsUpdate(bmc_widget_protocol::CredentialSecrets),
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
    /// Compositor published a new lifecycle state for this widget.
    Lifecycle(bmc_widget_protocol::LifecycleState),
    /// Automatic scene cycling will transition this widget on-screen soon.
    TransitionIncoming,
}

impl From<DeckWidgetEvent> for WidgetEvent {
    fn from(event: DeckWidgetEvent) -> Self {
        match event {
            DeckWidgetEvent::Setting(update) => Self::Setting(update),
            DeckWidgetEvent::ParamUpdate(params) => Self::ParamUpdate(params),
            DeckWidgetEvent::CredentialsUpdate(view) => Self::CredentialsUpdate(view),
            DeckWidgetEvent::SecretsUpdate(secrets) => Self::SecretsUpdate(secrets),
            DeckWidgetEvent::Shutdown => Self::Shutdown,
            DeckWidgetEvent::TouchDown { id, x, y } => Self::TouchDown { id, x, y },
            DeckWidgetEvent::TouchMotion { id, x, y } => Self::TouchMotion { id, x, y },
            DeckWidgetEvent::TouchUp { id } => Self::TouchUp { id },
            DeckWidgetEvent::TouchCancel => Self::TouchCancel,
            DeckWidgetEvent::Lifecycle(s) => Self::Lifecycle(s),
            DeckWidgetEvent::TransitionIncoming => Self::TransitionIncoming,
        }
    }
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
    pub width: u32,
    pub height: u32,
    pub viewport_shape: ViewportShape,
    pub display: bmc_widget_protocol::DisplayInfo,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub credentials: serde_json::Map<String, serde_json::Value>,
    /// For this process to spend on the widget's behalf,
    /// never forwarded to the widget itself.
    pub credential_secrets: bmc_widget_protocol::CredentialSecrets,
    pub settings: Vec<SettingUpdate>,
    /// Opaque, stable per-instance token delivered on the handshake (via
    /// `configure`); keys per-instance resources such as the asset cache.
    pub token: String,
}

/// Surface state for a `deck_widget` widget with DMA-BUF support.
///
/// Tracks compositor globals, surface lifecycle, frame scheduling, and
/// pending protocol events. Mirrors [`crate::surface::XdgSurfaceState`] but
/// uses the `deck_widget` protocol instead of XDG shell.
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
    widget_manager: Option<DeckWidgetManagerV2>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    seat: Option<wl_seat::WlSeat>,
    touch: Option<wl_touch::WlTouch>,
    surface: Option<wl_surface::WlSurface>,
    widget_surface: Option<DeckWidgetSurfaceV1>,

    // -- Initial configure accumulation --
    /// Becomes `true` when the compositor emits `configure_done`. Used by
    /// [`DeckWidgetSurfaceClient::connect`] to stop blocking.
    configure_done: bool,
    /// Accumulated `configure` data (dimensions, viewport shape, instance
    /// token). `None` until the compositor emits its `configure` event.
    pending_size: Option<(ViewportShape, u32, u32, String)>,
    /// Accumulated `display_info` event. `None` until the compositor emits
    /// it; resolved to the BMC100 default for old compositors that do not.
    pending_display: Option<bmc_widget_protocol::DisplayInfo>,
    /// Accumulated `params(json)` event (empty if the compositor sent
    /// `{}`).
    pending_params: serde_json::Map<String, serde_json::Value>,
    /// Accumulated `credentials(json)` event (empty when nothing is bound).
    pending_credentials: serde_json::Map<String, serde_json::Value>,
    /// Accumulated `credential_secrets(json)` event.
    pending_secrets: serde_json::Map<String, serde_json::Value>,
    /// Accumulated setting events emitted before `configure_done`.
    pending_initial_settings: Vec<SettingUpdate>,

    pending_events: Vec<DeckWidgetEvent>,
    buffer_slots: BufferSlotMap,
    released_buffers: ReleasedBufferSet,
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

    /// Drain slot ids released by the compositor.
    pub fn drain_released_slots(&mut self) -> Vec<usize> {
        drain_released_buffer_slots(&self.buffer_slots, &mut self.released_buffers)
    }

    /// Drain buffer ids released by the compositor.
    pub fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        drain_released_buffers(&self.buffer_slots, &mut self.released_buffers)
    }

    fn unregister_wl_buffer_id(&mut self, buffer_id: &ObjectId) -> Option<usize> {
        unregister_wl_buffer_slot(
            &mut self.buffer_slots,
            &mut self.released_buffers,
            buffer_id,
        )
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
            .field("buffer_slots", &self.buffer_slots.len())
            .field("released_buffers", &self.released_buffers.len())
            .finish_non_exhaustive()
    }
}

/// Single-connection Wayland client for `deck_widget` widgets with DMA-BUF.
///
/// Handles connection, global binding, surface creation, frame callbacks, and
/// slot-based buffer submission using the `deck_widget` protocol.
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
    pub fn connect_with_fd_and_key(
        wayland_fd: std::os::fd::OwnedFd,
        widget_key: WidgetInstanceKey,
    ) -> Result<(Self, InitialState)> {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::from(wayland_fd);
        let backend = wayland_backend::client::Backend::connect(stream)
            .map_err(|e| anyhow::anyhow!("wayland_backend::Backend::connect: {e}"))?;
        let conn = Connection::from_backend(backend);

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
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_credentials: serde_json::Map::new(),
            pending_secrets: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
            pending_events: Vec::new(),
            buffer_slots: BufferSlotMap::new(),
            released_buffers: ReleasedBufferSet::new(),
        };

        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        let compositor = state
            .compositor
            .as_ref()
            .context("wl_compositor not available")?;
        anyhow::ensure!(
            state.linux_dmabuf.is_some(),
            "zwp_linux_dmabuf_v1 not available"
        );

        let surface = compositor.create_surface(&qh, ());
        let widget_surface = state
            .widget_manager
            .as_ref()
            .context("deck_widget_manager_v2 not available")?
            .get_widget_surface(widget_key.to_string(), &surface, &qh, ());

        surface.commit();

        state.surface = Some(surface);
        state.widget_surface = Some(widget_surface);

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

        let (viewport_shape, width, height, token) = state
            .pending_size
            .take()
            .context("configure_done without prior configure event")?;
        state.width = width;
        state.height = height;

        let initial = take_initial_state(&mut state, viewport_shape, width, height, token);

        tracing::info!(
            "Deck widget surface ready (fd): {}x{} viewport_shape={:?} params={} settings={}",
            width,
            height,
            viewport_shape,
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

    /// Returns the file descriptor that the Wayland event queue polls on.
    #[must_use]
    pub fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::unix::io::AsFd;
        self.conn.as_fd()
    }

    /// Connect to the Wayland display and return the compositor's initial state.
    ///
    /// Requires the globals `wl_compositor`, `deck_widget_manager_v2`,
    /// and `zwp_linux_dmabuf_v1`; returns after `configure_done`.
    pub fn connect() -> Result<(Self, InitialState)> {
        let widget_key = widget_key_from_env()?;
        Self::connect_inner(widget_key)
    }

    fn connect_inner(widget_key: WidgetInstanceKey) -> Result<(Self, InitialState)> {
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
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_credentials: serde_json::Map::new(),
            pending_secrets: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
            pending_events: Vec::new(),
            buffer_slots: BufferSlotMap::new(),
            released_buffers: ReleasedBufferSet::new(),
        };

        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        let compositor = state
            .compositor
            .as_ref()
            .context("wl_compositor not available")?;
        anyhow::ensure!(
            state.linux_dmabuf.is_some(),
            "zwp_linux_dmabuf_v1 not available"
        );

        let surface = compositor.create_surface(&qh, ());
        let widget_surface = state
            .widget_manager
            .as_ref()
            .context("deck_widget_manager_v2 not available")?
            .get_widget_surface(widget_key.to_string(), &surface, &qh, ());

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

        let (viewport_shape, width, height, token) = state
            .pending_size
            .take()
            .context("configure_done without prior configure event")?;
        state.width = width;
        state.height = height;

        let initial = take_initial_state(&mut state, viewport_shape, width, height, token);

        tracing::info!(
            "Deck widget surface ready: {}x{} viewport_shape={:?} params={} settings={}",
            width,
            height,
            viewport_shape,
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
            self.state.buffer_slots.insert(buffer.id(), slot);
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
        invalidate_cached_wl_buffers(
            &mut self.cached_buffers,
            &mut self.state.buffer_slots,
            &mut self.state.released_buffers,
        );
    }

    pub fn invalidate_cached_buffer_slots(&mut self, slots: &[usize]) {
        invalidate_cached_wl_buffer_slots(
            &mut self.cached_buffers,
            &mut self.state.buffer_slots,
            &mut self.state.released_buffers,
            slots,
        );
    }

    pub fn submit_buffer_with_wl_buffer(
        &self,
        info: &DmaBufInfo,
        buffer: &wl_buffer::WlBuffer,
        request_frame: bool,
    ) -> Result<()> {
        self.submit_wl_buffer(buffer, info, request_frame)
    }

    pub fn flush(&self) -> Result<()> {
        self.conn.flush().context("Wayland flush failed")
    }

    /// Mint a `wl_buffer` directly from DMA-BUF info without release tracking.
    ///
    /// This compatibility path does not register the returned buffer in the
    /// release tracker, so `wl_buffer.release` events for it are not reported
    /// by [`Self::drain_released_slots`]. Callers that need release tracking
    /// must use [`Self::mint_wl_buffer_for_slot`] and destroy the returned
    /// buffer through [`Self::destroy_minted_wl_buffer`].
    pub fn mint_wl_buffer_via_dmabuf(&self, info: &DmaBufInfo) -> Result<wl_buffer::WlBuffer> {
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("linux_dmabuf not bound"))?;
        Ok(crate::surface::common::create_buffer_from_dmabuf(
            linux_dmabuf,
            info,
            &self.queue.handle(),
        ))
    }

    pub fn mint_wl_buffer_for_slot(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
    ) -> Result<wl_buffer::WlBuffer> {
        let buffer = self.mint_wl_buffer_via_dmabuf(info)?;
        self.state.buffer_slots.insert(buffer.id(), slot);
        Ok(buffer)
    }

    /// Destroy a host-owned minted `wl_buffer` and stop tracking its slot.
    ///
    /// Use this for buffers created by [`Self::mint_wl_buffer_for_slot`].
    /// Cached buffers remain owned by [`Self::invalidate_cached_buffers`].
    pub fn destroy_minted_wl_buffer(&mut self, buffer: wl_buffer::WlBuffer) {
        self.state.unregister_wl_buffer_id(&buffer.id());
        buffer.destroy();
        drop(buffer);
    }

    /// Drain slot ids released by the compositor.
    pub fn drain_released_slots(&mut self) -> Vec<usize> {
        self.state.drain_released_slots()
    }

    /// Drain buffer ids released by the compositor.
    pub fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        self.state.drain_released_buffers()
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.state.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.state.height
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

    /// Forward a typed widget action as a `deck_widget` request.
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
                scope,
            } => surface.led_temporary(
                *request_id,
                to_protocol::led_effect(*effect),
                u32::from(color.r),
                u32::from(color.g),
                u32::from(color.b),
                *period_ms,
                *duration_ms,
                to_protocol::led_scope(*scope),
            ),
            ActionPayload::LedEndless {
                request_id,
                effect,
                color,
                period_ms,
                scope,
            } => surface.led_endless(
                *request_id,
                to_protocol::led_effect(*effect),
                u32::from(color.r),
                u32::from(color.g),
                u32::from(color.b),
                *period_ms,
                to_protocol::led_scope(*scope),
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

    fn invalidate_cached_buffer_slots(&mut self, slots: &[usize]) {
        DeckWidgetSurfaceClient::invalidate_cached_buffer_slots(self, slots);
    }

    fn drain_released_slots(&mut self) -> Vec<usize> {
        DeckWidgetSurfaceClient::drain_released_slots(self)
    }

    fn drain_events(&mut self) -> Vec<WidgetEvent> {
        self.state
            .pending_events
            .drain(..)
            .map(Into::into)
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
                "deck_widget_manager_v2" => {
                    let widget_manager =
                        registry.bind::<DeckWidgetManagerV2, _, _>(name, version.min(2), qh, ());
                    tracing::debug!("Bound deck_widget_manager_v2 v{}", version.min(2));
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

impl Dispatch<DeckWidgetManagerV2, ()> for DeckWidgetSurfaceState {
    fn event(
        _: &mut Self,
        _: &DeckWidgetManagerV2,
        _: <DeckWidgetManagerV2 as wayland_client::Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn resolve_display(
    pending: Option<bmc_widget_protocol::DisplayInfo>,
) -> bmc_widget_protocol::DisplayInfo {
    pending.unwrap_or(bmc_widget_protocol::DisplayInfo::BMC100)
}

fn apply_display_info_event(
    state: &mut DeckWidgetSurfaceState,
    width: u32,
    height: u32,
    shape: WEnum<deck_widget_surface_v1::DisplayShape>,
    dpi: u32,
) {
    let Some(shape) = shape.into_result().ok().map(Into::into) else {
        tracing::warn!("display_info event carries unknown display_shape; ignoring event");
        return;
    };
    state.pending_display = Some(bmc_widget_protocol::DisplayInfo {
        width,
        height,
        shape,
        dpi,
    });
}

impl Dispatch<DeckWidgetSurfaceV1, ()> for DeckWidgetSurfaceState {
    #[expect(clippy::too_many_lines)]
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
                width,
                height,
                viewport_shape,
                token,
            } => {
                let Some(shape) = (match viewport_shape {
                    WEnum::Value(v) => Some(v.into()),
                    WEnum::Unknown(value) => {
                        tracing::warn!(
                            value,
                            "configure event carries unknown viewport_shape; ignoring event"
                        );
                        None
                    }
                }) else {
                    return;
                };
                tracing::debug!("Configure: {}x{} viewport_shape={:?}", width, height, shape);
                state.pending_size = Some((shape, width, height, token));
            }
            deck_widget_surface_v1::Event::DisplayInfo {
                width,
                height,
                shape,
                dpi,
            } => apply_display_info_event(state, width, height, shape, dpi),
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
            deck_widget_surface_v1::Event::UnitSystem { value } => {
                if let Some(v) = from_protocol::unit_system(value) {
                    push_setting(state, SettingUpdate::UnitSystem(v));
                }
            }
            deck_widget_surface_v1::Event::NextAlarm {
                present,
                fire_at_utc_ms_hi,
                fire_at_utc_ms_lo,
                name,
            } => {
                if let Some(present) = from_protocol::presence(present) {
                    let next = if present {
                        // i64 reassembly from the wayland-protocol hi/lo split
                        // (presentation-time `tv_sec_hi`/`tv_sec_lo` pattern).
                        let fire_at_utc_ms =
                            (i64::from(fire_at_utc_ms_hi) << 32) | i64::from(fire_at_utc_ms_lo);
                        Some(NextAlarm {
                            fire_at_utc_ms,
                            name,
                        })
                    } else {
                        None
                    };
                    push_setting(state, SettingUpdate::NextAlarm(next));
                }
            }
            deck_widget_surface_v1::Event::Shutdown => {
                tracing::info!("Shutdown requested by compositor");
                state.running = false;
                state.pending_events.push(DeckWidgetEvent::Shutdown);
            }
            deck_widget_surface_v1::Event::Lifecycle { state: value } => {
                if let Some(s) = from_protocol::lifecycle_state(value) {
                    state.pending_events.push(DeckWidgetEvent::Lifecycle(s));
                }
            }
            deck_widget_surface_v1::Event::TransitionIncoming => {
                state
                    .pending_events
                    .push(DeckWidgetEvent::TransitionIncoming);
            }
            deck_widget_surface_v1::Event::Credentials { json } => {
                handle_credential_json(
                    &mut state.pending_credentials,
                    &mut state.pending_events,
                    state.configure_done,
                    &json,
                    DeckWidgetEvent::CredentialsUpdate,
                    "credentials",
                );
            }
            deck_widget_surface_v1::Event::CredentialSecrets { json } => {
                handle_credential_json(
                    &mut state.pending_secrets,
                    &mut state.pending_events,
                    state.configure_done,
                    &json,
                    |map| {
                        DeckWidgetEvent::SecretsUpdate(bmc_widget_protocol::CredentialSecrets::new(
                            map,
                        ))
                    },
                    "credential_secrets",
                );
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

/// Drain the accumulated configure batch into the state handed to the widget.
fn take_initial_state(
    state: &mut DeckWidgetSurfaceState,
    viewport_shape: ViewportShape,
    width: u32,
    height: u32,
    token: String,
) -> InitialState {
    InitialState {
        width,
        height,
        viewport_shape,
        display: resolve_display(state.pending_display.take()),
        params: std::mem::take(&mut state.pending_params),
        credentials: std::mem::take(&mut state.pending_credentials),
        credential_secrets: bmc_widget_protocol::CredentialSecrets::new(std::mem::take(
            &mut state.pending_secrets,
        )),
        settings: std::mem::take(&mut state.pending_initial_settings),
        token,
    }
}

/// Decode one credential event into the initial batch or the runtime queue.
///
/// `what` names the event for the decode-failure log; the payload is never
/// logged, since the secrets variant flows through here too.
fn handle_credential_json(
    pending: &mut serde_json::Map<String, serde_json::Value>,
    pending_events: &mut Vec<DeckWidgetEvent>,
    configure_done: bool,
    json: &str,
    to_event: fn(serde_json::Map<String, serde_json::Value>) -> DeckWidgetEvent,
    what: &str,
) {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(map)) => {
            if configure_done {
                pending_events.push(to_event(map));
            } else {
                *pending = map;
            }
        }
        Ok(_) => tracing::warn!("{what} JSON is not an object, ignoring"),
        Err(e) => tracing::warn!("Failed to decode {what} JSON: {e}"),
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
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            record_released_buffer(
                &state.buffer_slots,
                &mut state.released_buffers,
                buffer.id(),
            );
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
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state
                    .pending_events
                    .push(DeckWidgetEvent::TouchMotion { id, x, y });
            }
            wl_touch::Event::Up { id, .. } => {
                state.pending_events.push(DeckWidgetEvent::TouchUp { id });
            }
            wl_touch::Event::Cancel => {
                state.pending_events.push(DeckWidgetEvent::TouchCancel);
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
    use wayland_client::WEnum;

    use super::{
        DeckWidgetEvent, DeckWidgetSurfaceState, ReleasableTouch, TouchCapabilityChange,
        WidgetEvent, handle_params_json, sync_touch_capability,
    };
    use bmc_widget_protocol::LifecycleState;
    use bmc_widget_protocol::client::deck_widget_surface_v1;

    fn test_surface_state() -> DeckWidgetSurfaceState {
        DeckWidgetSurfaceState {
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
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_credentials: serde_json::Map::new(),
            pending_secrets: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
            pending_events: Vec::new(),
            buffer_slots: super::BufferSlotMap::new(),
            released_buffers: super::ReleasedBufferSet::new(),
        }
    }

    #[test]
    fn pending_display_defaults_to_bmc100_when_unset() {
        let resolved = super::resolve_display(None);
        assert_eq!(resolved, bmc_widget_protocol::DisplayInfo::BMC100);
    }

    #[test]
    fn pending_display_uses_compositor_value_when_set() {
        let round = bmc_widget_protocol::DisplayInfo {
            width: 480,
            height: 480,
            shape: bmc_widget_protocol::DisplayShape::Round,
            dpi: 1,
        };
        assert_eq!(super::resolve_display(Some(round)), round);
    }

    #[test]
    fn display_info_event_updates_pending_display() {
        let mut state = test_surface_state();
        super::apply_display_info_event(
            &mut state,
            480,
            480,
            WEnum::Value(deck_widget_surface_v1::DisplayShape::Round),
            1,
        );
        assert_eq!(
            state.pending_display,
            Some(bmc_widget_protocol::DisplayInfo {
                width: 480,
                height: 480,
                shape: bmc_widget_protocol::DisplayShape::Round,
                dpi: 1,
            }),
        );
    }

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

    #[test]
    fn deck_widget_event_lifecycle_translates_to_widget_event_lifecycle() {
        let translated: WidgetEvent = DeckWidgetEvent::Lifecycle(LifecycleState::Visible).into();
        assert!(matches!(
            translated,
            WidgetEvent::Lifecycle(LifecycleState::Visible)
        ));
    }

    #[test]
    fn deck_widget_event_shutdown_still_translates_to_widget_event_shutdown() {
        let translated: WidgetEvent = DeckWidgetEvent::Shutdown.into();
        assert!(matches!(translated, WidgetEvent::Shutdown));
    }

    #[test]
    fn deck_widget_event_transition_incoming_translates_to_widget_event_transition_incoming() {
        let translated: WidgetEvent = DeckWidgetEvent::TransitionIncoming.into();
        assert!(matches!(translated, WidgetEvent::TransitionIncoming));
    }

    #[test]
    fn drain_released_slots_returns_and_clears_ids() {
        let mut state = DeckWidgetSurfaceState {
            running: true,
            width: 64,
            height: 64,
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
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_credentials: serde_json::Map::new(),
            pending_secrets: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
            pending_events: Vec::new(),
            buffer_slots: super::BufferSlotMap::new(),
            released_buffers: super::ReleasedBufferSet::new(),
        };
        let buffer_id = wayland_backend::client::ObjectId::null();
        state.buffer_slots.insert(buffer_id.clone(), 3);
        state.released_buffers.insert(buffer_id);

        let drained = state.drain_released_slots();

        assert_eq!(drained, vec![3]);
        assert!(state.released_buffers.is_empty());
    }

    #[test]
    fn unregister_wl_buffer_id_removes_mapping_and_pending_release() {
        let mut state = DeckWidgetSurfaceState {
            running: true,
            width: 64,
            height: 64,
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
            pending_display: None,
            pending_params: serde_json::Map::new(),
            pending_credentials: serde_json::Map::new(),
            pending_secrets: serde_json::Map::new(),
            pending_initial_settings: Vec::new(),
            pending_events: Vec::new(),
            buffer_slots: super::BufferSlotMap::new(),
            released_buffers: super::ReleasedBufferSet::new(),
        };
        let buffer_id = wayland_backend::client::ObjectId::null();
        state.buffer_slots.insert(buffer_id.clone(), 7);
        state.released_buffers.insert(buffer_id.clone());

        let removed = state.unregister_wl_buffer_id(&buffer_id);

        assert_eq!(removed, Some(7));
        assert!(!state.buffer_slots.contains_key(&buffer_id));
        assert!(!state.released_buffers.contains(&buffer_id));
    }
}

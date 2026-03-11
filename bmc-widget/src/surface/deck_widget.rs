// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fmt;

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

use crate::egl::DmaBufInfo;
use crate::wayland::setting_from_protocol;

use super::common::{
    PollOutcome, blocking_dispatch_impl, create_buffer_from_dmabuf, impl_common_dispatch,
    invalidate_cached_wl_buffers, poll_dispatch, submit_buffer_to_surface,
};
use super::{WidgetEvent, WidgetSurface};

/// Events from the compositor to a `deck_widget_v1` widget.
#[derive(Debug, Clone)]
pub enum DeckWidgetEvent {
    /// A system setting changed at runtime.
    Setting {
        /// Setting type enum value from the protocol.
        setting_type: u32,
        /// JSON-encoded setting value.
        value: String,
    },
    /// Compositor requested graceful shutdown.
    Shutdown,
    /// Touch down from standard `wl_touch`.
    TouchDown { id: i32, x: f64, y: f64 },
    /// Touch motion from standard `wl_touch`.
    TouchMotion { id: i32, x: f64, y: f64 },
    /// Touch up from standard `wl_touch`.
    TouchUp { id: i32 },
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
    pending_events: Vec<DeckWidgetEvent>,
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
    /// Connect to the Wayland display and create a `deck_widget_v1` surface.
    ///
    /// Binds `wl_compositor`, `deck_widget_manager_v1`, and
    /// `zwp_linux_dmabuf_v1`, then creates a surface and registers it with
    /// the given `instance_id`.
    pub fn connect(instance_id: &str, width: u32, height: u32) -> Result<Self> {
        anyhow::ensure!(
            width > 0 && height > 0,
            "surface dimensions must be non-zero"
        );

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = DeckWidgetSurfaceState {
            running: true,
            width,
            height,
            needs_render: false,
            frame_count: 0,
            compositor: None,
            widget_manager: None,
            linux_dmabuf: None,
            seat: None,
            touch: None,
            surface: None,
            widget_surface: None,
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
        let widget_surface =
            widget_manager.get_widget_surface(&surface, instance_id.to_owned(), &qh, ());

        surface.commit();

        state.surface = Some(surface);
        state.widget_surface = Some(widget_surface);

        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip after widget registration")?;

        tracing::info!(
            "Deck widget surface ready: {}x{} instance_id={}",
            state.width,
            state.height,
            instance_id,
        );

        Ok(Self {
            conn,
            queue,
            state,
            cached_buffers: Vec::new(),
        })
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
            .filter_map(|event| match event {
                DeckWidgetEvent::Setting {
                    setting_type,
                    value,
                } => setting_from_protocol(setting_type, &value).map(WidgetEvent::Setting),
                DeckWidgetEvent::Shutdown => Some(WidgetEvent::Shutdown),
                DeckWidgetEvent::TouchDown { id, x, y } => {
                    Some(WidgetEvent::TouchDown { id, x, y })
                }
                DeckWidgetEvent::TouchMotion { id, x, y } => {
                    Some(WidgetEvent::TouchMotion { id, x, y })
                }
                DeckWidgetEvent::TouchUp { id } => Some(WidgetEvent::TouchUp { id }),
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
            deck_widget_surface_v1::Event::Setting {
                setting_type,
                value,
            } => {
                tracing::debug!("Setting update: type={setting_type:?}, value={value}");
                state.pending_events.push(DeckWidgetEvent::Setting {
                    setting_type: setting_type.into(),
                    value,
                });
            }
            deck_widget_surface_v1::Event::Shutdown => {
                tracing::info!("Shutdown requested by compositor");
                state.running = false;
                state.pending_events.push(DeckWidgetEvent::Shutdown);
            }
            _ => {}
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
            && caps.contains(wl_seat::Capability::Touch)
            && state.touch.is_none()
        {
            let touch = seat.get_touch(qh, ());
            tracing::debug!("Acquired wl_touch from seat");
            state.touch = Some(touch);
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
            wl_touch::Event::Frame
            | wl_touch::Event::Cancel
            | wl_touch::Event::Shape { .. }
            | wl_touch::Event::Orientation { .. }
            | _ => {}
        }
    }
}

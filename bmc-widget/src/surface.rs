// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland surface client with DMA-BUF buffer management.
//!
//! Provides [`XdgSurfaceClient`] — a turnkey Wayland client that connects to
//! the compositor, creates an XDG toplevel surface, and manages DMA-BUF buffer
//! submission via `zwp_linux_dmabuf_v1`. Widgets only need to implement their
//! render loop on top.

use std::fmt;
use std::os::fd::AsFd;

use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use bmc_widget_protocol::client::{
    deck_widget_manager_v1::DeckWidgetManagerV1,
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};

use crate::egl::DmaBufInfo;
use crate::wayland::setting_from_protocol;

/// Re-export so widgets can match on setting variants without depending on
/// `bmc-widget-protocol` directly.
pub use bmc_widget_protocol::SettingUpdate;

// ── Shared helpers for both surface clients ────────────────────────────

/// Block until a Wayland event arrives, then dispatch all pending events.
///
/// Shared implementation used by both [`XdgSurfaceClient`] and
/// [`DeckWidgetSurfaceClient`].
fn blocking_dispatch_impl<S: 'static>(queue: &mut EventQueue<S>, state: &mut S) -> Result<()> {
    queue
        .blocking_dispatch(state)
        .context("Wayland dispatch failed")?;
    Ok(())
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
fn poll_dispatch<S: 'static>(
    conn: &Connection,
    queue: &mut EventQueue<S>,
    state: &mut S,
    timeout_ms: i32,
) -> Result<bool> {
    conn.flush()?;

    let read_guard = queue.prepare_read();
    let mut read_would_block = false;

    match read_guard {
        None => {
            // Events already queued — just dispatch them
            queue
                .dispatch_pending(state)
                .context("Wayland dispatch failed")?;
            return Ok(true);
        }
        Some(guard) => {
            let fd = conn.as_fd();
            let mut pollfd = libc::pollfd {
                fd: std::os::fd::AsRawFd::as_raw_fd(&fd),
                events: libc::POLLIN,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) };

            match poll_ret.cmp(&0) {
                std::cmp::Ordering::Greater => match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(err))
                        if err.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // Non-fatal race: poll reported readability but no
                        // event was available by the time we read.
                        read_would_block = true;
                    }
                    Err(err) => return Err(err).context("Wayland socket read failed"),
                },
                std::cmp::Ordering::Equal => {
                    // Timeout — cancel read
                    drop(guard);
                }
                std::cmp::Ordering::Less => {
                    // Error
                    let err = std::io::Error::last_os_error();
                    drop(guard);
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "all other io::ErrorKind variants are fatal"
                    )]
                    match err.kind() {
                        std::io::ErrorKind::Interrupted => {
                            // EINTR — not fatal, just dispatch pending
                        }
                        std::io::ErrorKind::WouldBlock => {
                            // EAGAIN — non-fatal, dispatch pending and
                            // signal caller via Ok(false)
                            queue
                                .dispatch_pending(state)
                                .context("Wayland dispatch failed")?;
                            return Ok(false);
                        }
                        _ => {
                            return Err(err).context("poll(2) on Wayland fd failed");
                        }
                    }
                }
            }
        }
    }

    queue
        .dispatch_pending(state)
        .context("Wayland dispatch failed")?;

    if read_would_block {
        return Ok(false);
    }

    Ok(true)
}

/// Destroy all cached `wl_buffer`s and return the number destroyed.
///
/// Shared implementation for [`XdgSurfaceClient::invalidate_cached_buffers`]
/// and [`DeckWidgetSurfaceClient::invalidate_cached_buffers`].
fn invalidate_cached_wl_buffers(cached_buffers: &mut [Option<wl_buffer::WlBuffer>]) -> u32 {
    let mut destroyed = 0_u32;
    for cached in cached_buffers {
        if let Some(buf) = cached.take() {
            buf.destroy();
            destroyed += 1;
        }
    }
    if destroyed > 0 {
        tracing::debug!("Destroyed {destroyed} cached wl_buffer(s)");
    }
    destroyed
}

/// Attach buffer, damage, optionally request frame callback, and commit.
///
/// Shared implementation for buffer submission on both surface client types.
#[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
fn submit_buffer_to_surface<S>(
    surface: &wl_surface::WlSurface,
    qh: &QueueHandle<S>,
    buffer: &wl_buffer::WlBuffer,
    info: &DmaBufInfo,
    request_frame: bool,
) where
    S: Dispatch<wl_callback::WlCallback, ()> + 'static,
{
    surface.attach(Some(buffer), 0, 0);
    surface.damage_buffer(0, 0, info.width as i32, info.height as i32);

    if request_frame {
        surface.frame(qh, ());
    }

    surface.commit();
}

/// Typed events from the compositor to a widget.
#[derive(Debug, Clone)]
pub enum WidgetEvent {
    /// A setting was updated at runtime.
    Setting(bmc_widget_protocol::SettingUpdate),
    /// The compositor requests graceful shutdown.
    Shutdown,
}

/// Common interface for widget surface clients.
///
/// Abstracts over XDG toplevel (standalone) and `deck_widget_v1` (production)
/// backends so widget render loops can work with either.
pub trait WidgetSurface {
    /// Whether the event loop should keep running.
    fn running(&self) -> bool;
    /// Request shutdown (sets running to false).
    fn request_shutdown(&mut self);
    /// Current surface width in pixels.
    fn width(&self) -> u32;
    /// Current surface height in pixels.
    fn height(&self) -> u32;
    /// Whether a resize occurred since last acknowledged.
    fn take_size_changed(&mut self) -> bool;
    /// Whether a frame callback or timeout has fired — widget should render.
    /// Unlike [`take_render_requested`](Self::take_render_requested), this
    /// does not clear the flag.
    fn needs_render(&self) -> bool;
    /// Whether a frame callback or timeout has fired — widget should render.
    /// Clears the flag so subsequent calls return `false` until the next event.
    fn take_render_requested(&mut self) -> bool;
    /// Signal that a render is needed (e.g. after poll timeout).
    fn mark_needs_render(&mut self);
    /// Number of per-frame buffers currently held by the compositor.
    ///
    /// This is meaningful for backends that create a fresh `wl_buffer` per
    /// submit and destroy it on `Release`.
    ///
    /// Cached-buffer backends reuse the same small set of `wl_buffer`s across
    /// frames and rely on frame callbacks for backpressure instead, so this may
    /// remain `0` even while frames continue to be presented.
    fn pending_buffer_count(&self) -> u32;
    /// Whether more frames can be submitted without exceeding the limit.
    ///
    /// For per-frame buffer backends this typically compares
    /// [`pending_buffer_count`](Self::pending_buffer_count) with `max_pending`.
    ///
    /// Cached-buffer backends may return `true` unconditionally because the
    /// compositor backpressures them through frame callbacks rather than
    /// per-frame in-flight buffer accumulation.
    fn can_submit_frame(&self, max_pending: u32) -> bool;
    /// Frame counter (wrapping).
    fn frame_count(&self) -> u32;
    /// Block until a Wayland event arrives, then dispatch.
    fn blocking_dispatch(&mut self) -> anyhow::Result<()>;
    /// Poll for events with timeout, then dispatch. -1 blocks, 0 non-blocking.
    fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<bool>;
    /// Request the first frame callback (call once before the event loop).
    fn request_frame(&self);
    /// Submit a DMA-BUF frame. Optionally request a frame callback.
    fn commit_buffer(&mut self, info: &DmaBufInfo, request_frame: bool) -> anyhow::Result<()>;
    /// Submit a DMA-BUF frame using cached `wl_buffer` for a double-buffer slot.
    fn commit_cached_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> anyhow::Result<()>;
    /// Invalidate cached `wl_buffer`s (call on resize).
    fn invalidate_cached_buffers(&mut self);
    /// Drain pending compositor events.
    fn drain_events(&mut self) -> Vec<WidgetEvent>;
}

/// Wayland surface state for an XDG toplevel with DMA-BUF support.
///
/// Tracks compositor globals, surface lifecycle flags, and frame scheduling.
/// Widgets read these fields to decide when and what to render.
#[expect(clippy::struct_excessive_bools, reason = "protocol state flags")]
pub struct XdgSurfaceState {
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Current surface width in pixels.
    pub width: u32,
    /// Current surface height in pixels.
    pub height: u32,
    /// A frame callback fired — widget should render.
    pub needs_render: bool,
    /// Surface was resized — widget should recreate GPU buffers.
    pub size_changed: bool,
    /// Number of frames rendered (wrapping counter).
    pub frame_count: u32,

    // -- Wayland objects (internal) --
    compositor: Option<wl_compositor::WlCompositor>,
    xdg_wm_base: Option<xdg_wm_base::XdgWmBase>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    configured: bool,
}

impl fmt::Debug for XdgSurfaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XdgSurfaceState")
            .field("running", &self.running)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("needs_render", &self.needs_render)
            .field("size_changed", &self.size_changed)
            .field("frame_count", &self.frame_count)
            .field("configured", &self.configured)
            .finish_non_exhaustive()
    }
}

impl XdgSurfaceState {
    /// Get the `zwp_linux_dmabuf_v1` global (for widgets that manage their own
    /// `wl_buffer` lifecycle, e.g. cached buffers).
    #[must_use]
    pub fn linux_dmabuf(&self) -> Option<&zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1> {
        self.linux_dmabuf.as_ref()
    }

    /// Get the `wl_surface` (for widgets that need direct surface access, e.g.
    /// to attach cached buffers or request frame callbacks manually).
    #[must_use]
    pub fn wl_surface(&self) -> Option<&wl_surface::WlSurface> {
        self.surface.as_ref()
    }
}

/// Wayland client for XDG toplevel surfaces with DMA-BUF buffer management.
///
/// Handles connection, global binding, surface creation, frame callbacks, and
/// buffer submission. The widget owns the render loop and calls
/// [`commit_buffer`](Self::commit_buffer) after each frame.
pub struct XdgSurfaceClient {
    conn: Connection,
    queue: EventQueue<XdgSurfaceState>,
    state: XdgSurfaceState,
    /// Per-slot cached `wl_buffer`s for double-buffered rendering.
    /// Widgets that reuse the same DMA-BUF fd/stride across frames for a
    /// given slot can avoid per-frame `wl_buffer` creation overhead.
    cached_buffers: Vec<Option<wl_buffer::WlBuffer>>,
}

impl fmt::Debug for XdgSurfaceClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cached = self.cached_buffers.iter().filter(|b| b.is_some()).count();
        f.debug_struct("XdgSurfaceClient")
            .field("state", &self.state)
            .field(
                "cached_buffers",
                &format_args!("{cached}/{}", self.cached_buffers.len()),
            )
            .finish_non_exhaustive()
    }
}

impl XdgSurfaceClient {
    /// Connect to the Wayland display and create an XDG toplevel surface.
    pub fn connect(width: u32, height: u32, title: &str, app_id: &str) -> Result<Self> {
        anyhow::ensure!(
            width > 0 && height > 0,
            "surface dimensions must be non-zero"
        );

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = XdgSurfaceState {
            running: true,
            width,
            height,
            needs_render: false,
            size_changed: false,
            frame_count: 0,
            compositor: None,
            xdg_wm_base: None,
            linux_dmabuf: None,
            surface: None,
            xdg_surface: None,
            xdg_toplevel: None,
            configured: false,
        };

        // Roundtrip to discover globals
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        // Verify required globals
        let compositor = state
            .compositor
            .as_ref()
            .context("wl_compositor not available")?;
        let xdg_wm_base = state
            .xdg_wm_base
            .as_ref()
            .context("xdg_wm_base not available")?;
        anyhow::ensure!(
            state.linux_dmabuf.is_some(),
            "zwp_linux_dmabuf_v1 not available"
        );

        // Create surface + XDG toplevel
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &qh, ());
        let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());

        xdg_toplevel.set_title(title.to_owned());
        xdg_toplevel.set_app_id(app_id.to_owned());

        // Commit to trigger configure
        surface.commit();

        state.surface = Some(surface);
        state.xdg_surface = Some(xdg_surface);
        state.xdg_toplevel = Some(xdg_toplevel);

        // Wait for configure
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for configure")?;

        tracing::info!("XDG surface configured: {}x{}", state.width, state.height);

        Ok(Self {
            conn,
            queue,
            state,
            cached_buffers: Vec::new(),
        })
    }

    /// Get a reference to the surface state.
    #[must_use]
    pub fn state(&self) -> &XdgSurfaceState {
        &self.state
    }

    /// Get a mutable reference to the surface state.
    pub fn state_mut(&mut self) -> &mut XdgSurfaceState {
        &mut self.state
    }

    /// Get the queue handle for creating protocol objects.
    #[must_use]
    pub fn queue_handle(&self) -> QueueHandle<XdgSurfaceState> {
        self.queue.handle()
    }

    /// Commit a rendered DMA-BUF frame to the compositor.
    ///
    /// Creates a `wl_buffer` from the DMA-BUF info, attaches it to the
    /// surface, damages the full area, optionally requests a frame callback,
    /// and commits.
    pub fn commit_buffer(&mut self, info: &DmaBufInfo, request_frame: bool) -> Result<()> {
        let qh = self.queue.handle();
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 not available")?;

        let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
        self.submit_buffer(&buffer, info, request_frame)
    }

    /// Commit a cached DMA-BUF frame for a double-buffer slot.
    ///
    /// On first call for a given `slot`, creates a `wl_buffer` from the
    /// DMA-BUF info and caches it. Subsequent calls reuse the cached buffer,
    /// avoiding per-frame `wl_buffer` creation overhead.
    ///
    /// Call [`invalidate_cached_buffers`](Self::invalidate_cached_buffers)
    /// when the surface is resized or the underlying DMA-BUF changes.
    pub fn commit_cached_buffer(
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

        // Grow slot storage if needed
        if slot >= self.cached_buffers.len() {
            self.cached_buffers.resize_with(slot + 1, || None);
        }

        // Create and cache the wl_buffer on first use for this slot
        if self.cached_buffers[slot].is_none() {
            let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
            self.cached_buffers[slot] = Some(buffer);
            tracing::debug!("Cached wl_buffer for slot {slot}");
        }

        let buffer = self.cached_buffers[slot]
            .as_ref()
            .expect("BUG: cached buffer should exist after creation above");
        self.submit_buffer(buffer, info, request_frame)
    }

    /// Attach buffer, damage, optionally request frame callback, and commit.
    fn submit_buffer(
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
    /// This follows the `prepare_read → poll → read/cancel → dispatch_pending`
    /// pattern required by `wayland-client`.
    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> Result<bool> {
        poll_dispatch(&self.conn, &mut self.queue, &mut self.state, timeout_ms)
    }
}

impl WidgetSurface for XdgSurfaceClient {
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
        let changed = self.state.size_changed;
        self.state.size_changed = false;
        changed
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

    fn pending_buffer_count(&self) -> u32 {
        // XDG uses cached buffers; no per-frame tracking.
        0
    }

    fn can_submit_frame(&self, _max_pending: u32) -> bool {
        // XDG cached buffers: compositor handles backpressure via frame callbacks.
        true
    }

    fn frame_count(&self) -> u32 {
        self.state.frame_count
    }

    fn blocking_dispatch(&mut self) -> anyhow::Result<()> {
        XdgSurfaceClient::blocking_dispatch(self)
    }

    fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<bool> {
        XdgSurfaceClient::poll_dispatch(self, timeout_ms)
    }

    fn request_frame(&self) {
        XdgSurfaceClient::request_frame(self);
    }

    fn commit_buffer(&mut self, info: &DmaBufInfo, request_frame: bool) -> anyhow::Result<()> {
        XdgSurfaceClient::commit_buffer(self, info, request_frame)
    }

    fn commit_cached_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> anyhow::Result<()> {
        XdgSurfaceClient::commit_cached_buffer(self, info, slot, request_frame)
    }

    fn invalidate_cached_buffers(&mut self) {
        XdgSurfaceClient::invalidate_cached_buffers(self);
    }

    fn drain_events(&mut self) -> Vec<WidgetEvent> {
        // XDG toplevel has no settings events. Close is reflected in running().
        Vec::new()
    }
}

/// Create a `wl_buffer` from DMA-BUF info using the `linux-dmabuf` protocol.
#[must_use]
#[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
pub fn create_buffer_from_dmabuf<S>(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<S>,
) -> wl_buffer::WlBuffer
where
    S: Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()>
        + Dispatch<wl_buffer::WlBuffer, ()>
        + 'static,
{
    let params = linux_dmabuf.create_params(qh, ());

    let modifier: u64 = info.modifier.into();
    let modifier_hi = (modifier >> 32) as u32;
    let modifier_lo = (modifier & 0xFFFF_FFFF) as u32;

    params.add(
        info.fd.as_fd(),
        0, // plane index
        0, // offset
        info.stride,
        modifier_hi,
        modifier_lo,
    );

    let buffer = params.create_immed(
        info.width as i32,
        info.height as i32,
        info.format as u32,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    );
    params.destroy();

    buffer
}

/// Generate common Dispatch impls for a surface state type.
///
/// These impls are identical for both XDG and deck_widget surface clients.
/// The state type must have `frame_count: u32` and `needs_render: bool` fields.
macro_rules! impl_common_dispatch {
    ($state:ty) => {
        impl Dispatch<wl_compositor::WlCompositor, ()> for $state {
            fn event(
                _: &mut Self,
                _: &wl_compositor::WlCompositor,
                _: wl_compositor::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<wl_surface::WlSurface, ()> for $state {
            fn event(
                _: &mut Self,
                _: &wl_surface::WlSurface,
                _: wl_surface::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<wl_callback::WlCallback, ()> for $state {
            fn event(
                state: &mut Self,
                _: &wl_callback::WlCallback,
                event: wl_callback::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if let wl_callback::Event::Done { .. } = event {
                    state.frame_count = state.frame_count.wrapping_add(1);
                    state.needs_render = true;
                }
            }
        }

        impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for $state {
            fn event(
                _: &mut Self,
                _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
                event: zwp_linux_dmabuf_v1::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    zwp_linux_dmabuf_v1::Event::Format { format } => {
                        tracing::trace!("DMA-BUF format: 0x{format:08x}");
                    }
                    zwp_linux_dmabuf_v1::Event::Modifier {
                        format,
                        modifier_hi,
                        modifier_lo,
                    } => {
                        let modifier = (u64::from(modifier_hi) << 32) | u64::from(modifier_lo);
                        tracing::trace!("DMA-BUF format 0x{format:08x} modifier 0x{modifier:016x}");
                    }
                    _ => {}
                }
            }
        }

        impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for $state {
            fn event(
                _: &mut Self,
                _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
                event: zwp_linux_buffer_params_v1::Event,
                (): &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if let zwp_linux_buffer_params_v1::Event::Failed = event {
                    tracing::error!("DMA-BUF buffer creation failed");
                }
            }
        }
    };
}

// ── Wayland protocol dispatch implementations ─────────────────────────

impl_common_dispatch!(XdgSurfaceState);

impl Dispatch<wl_registry::WlRegistry, ()> for XdgSurfaceState {
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
                "xdg_wm_base" => {
                    let wm_base =
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, version.min(6), qh, ());
                    tracing::debug!("Bound xdg_wm_base v{}", version.min(6));
                    state.xdg_wm_base = Some(wm_base);
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
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for XdgSurfaceState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for XdgSurfaceState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for XdgSurfaceState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 && height > 0 {
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "width/height are positive after check"
                    )]
                    {
                        let new_width = width as u32;
                        let new_height = height as u32;
                        if new_width != state.width || new_height != state.height {
                            state.width = new_width;
                            state.height = new_height;
                            state.size_changed = true;
                        }
                    }
                    tracing::debug!("Toplevel configured: {width}x{height}");
                }
            }
            xdg_toplevel::Event::Close => {
                tracing::info!("Close requested");
                state.running = false;
            }
            xdg_toplevel::Event::ConfigureBounds { .. }
            | xdg_toplevel::Event::WmCapabilities { .. }
            | _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for XdgSurfaceState {
    fn event(
        _: &mut Self,
        _buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            // Buffer released by compositor. Current XDG users submit cached
            // buffers and reuse them across frames, so release is
            // intentionally ignored here. If XDG gains per-frame buffer
            // submission in the future, its wl_buffer lifecycle must be
            // tracked in XdgSurfaceClient.
        }
    }
}

// ── deck_widget_v1 surface client ──────────────────────────────────────

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
}

/// Surface state for a `deck_widget_v1` widget with DMA-BUF support.
///
/// Tracks compositor globals, surface lifecycle, frame scheduling, and
/// pending protocol events. Mirrors [`XdgSurfaceState`] but uses the
/// `deck_widget_v1` protocol instead of XDG shell.
pub struct DeckWidgetSurfaceState {
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Current surface width in pixels.
    pub width: u32,
    /// Current surface height in pixels.
    pub height: u32,
    /// A frame callback fired — widget should render.
    pub needs_render: bool,
    /// Number of `wl_buffer`s currently held by the compositor.
    pub pending_buffers: u32,
    /// Number of frames rendered (wrapping counter).
    pub frame_count: u32,
    /// Whether buffer caching is active (set on first `commit_cached_buffer`).
    ///
    /// When true, the `wl_buffer::Release` handler is a no-op because cached
    /// buffers are reused across frames. When false, per-frame buffers are
    /// destroyed on release.
    pub buffer_caching: bool,

    // -- Wayland objects (internal) --
    compositor: Option<wl_compositor::WlCompositor>,
    widget_manager: Option<DeckWidgetManagerV1>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    surface: Option<wl_surface::WlSurface>,
    widget_surface: Option<DeckWidgetSurfaceV1>,
    pending_events: Vec<DeckWidgetEvent>,
}

impl DeckWidgetSurfaceState {
    /// Drain all pending protocol events.
    pub fn drain_events(&mut self) -> std::vec::Drain<'_, DeckWidgetEvent> {
        self.pending_events.drain(..)
    }

    /// Get a reference to the `zwp_linux_dmabuf_v1` global, if bound.
    #[must_use]
    pub fn linux_dmabuf(&self) -> Option<&zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1> {
        self.linux_dmabuf.as_ref()
    }

    /// Get a reference to the `wl_surface`, if created.
    #[must_use]
    pub fn wl_surface(&self) -> Option<&wl_surface::WlSurface> {
        self.surface.as_ref()
    }
}

impl fmt::Debug for DeckWidgetSurfaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeckWidgetSurfaceState")
            .field("running", &self.running)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("needs_render", &self.needs_render)
            .field("pending_buffers", &self.pending_buffers)
            .field("frame_count", &self.frame_count)
            .field("buffer_caching", &self.buffer_caching)
            .field("pending_events", &self.pending_events.len())
            .finish_non_exhaustive()
    }
}

/// Single-connection Wayland client for `deck_widget_v1` widgets with DMA-BUF.
///
/// Handles connection, global binding, surface creation, frame callbacks, and
/// buffer submission using the `deck_widget_v1` protocol. Widgets own the
/// render loop and call [`commit_buffer`](Self::commit_buffer) after each
/// frame. Unlike [`XdgSurfaceClient`], buffers are created per-frame and
/// destroyed on release.
pub struct DeckWidgetSurfaceClient {
    conn: Connection,
    queue: EventQueue<DeckWidgetSurfaceState>,
    state: DeckWidgetSurfaceState,
    /// Per-slot cached `wl_buffer`s for double-buffered rendering.
    /// Same pattern as [`XdgSurfaceClient::cached_buffers`].
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
            pending_buffers: 0,
            frame_count: 0,
            buffer_caching: false,
            compositor: None,
            widget_manager: None,
            linux_dmabuf: None,
            surface: None,
            widget_surface: None,
            pending_events: Vec::new(),
        };

        // Roundtrip to discover globals
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        // Verify required globals
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

        // Create surface + widget surface
        let surface = compositor.create_surface(&qh, ());
        let widget_surface =
            widget_manager.get_widget_surface(&surface, instance_id.to_owned(), &qh, ());

        // Commit to register with the compositor
        surface.commit();

        state.surface = Some(surface);
        state.widget_surface = Some(widget_surface);

        // Wait for initial events
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

    /// Get a mutable reference to the surface state.
    pub fn state_mut(&mut self) -> &mut DeckWidgetSurfaceState {
        &mut self.state
    }

    /// Get a handle to the event queue for creating Wayland protocol objects.
    #[must_use]
    pub fn queue_handle(&self) -> QueueHandle<DeckWidgetSurfaceState> {
        self.queue.handle()
    }

    /// Commit a rendered DMA-BUF frame to the compositor.
    ///
    /// Creates a per-frame `wl_buffer` from the DMA-BUF info, attaches it to
    /// the surface, damages the full area, optionally requests a frame
    /// callback, and commits. The buffer is destroyed on release by the
    /// compositor (see `wl_buffer` dispatch).
    pub fn commit_buffer(&mut self, info: &DmaBufInfo, request_frame: bool) -> Result<()> {
        let qh = self.queue.handle();
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 not available")?;

        let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
        self.submit_buffer(&buffer, info, request_frame)?;
        self.state.pending_buffers += 1;

        Ok(())
    }

    /// Commit a cached DMA-BUF frame for a double-buffer slot.
    ///
    /// On first call for a given `slot`, creates a `wl_buffer` from the
    /// DMA-BUF info and caches it. Subsequent calls reuse the cached buffer,
    /// avoiding per-frame `wl_buffer` creation overhead. Enables buffer
    /// caching mode on first call, which makes the `wl_buffer::Release`
    /// handler a no-op.
    ///
    /// Call [`invalidate_cached_buffers`](Self::invalidate_cached_buffers)
    /// when the surface is resized or the underlying DMA-BUF changes.
    pub fn commit_cached_buffer(
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

        // Enable buffer caching mode on first call
        if !self.state.buffer_caching {
            self.state.buffer_caching = true;
            tracing::debug!("Buffer caching enabled for deck_widget surface");
        }

        // Grow slot storage if needed
        if slot >= self.cached_buffers.len() {
            self.cached_buffers.resize_with(slot + 1, || None);
        }

        // Create and cache the wl_buffer on first use for this slot
        if self.cached_buffers[slot].is_none() {
            let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
            self.cached_buffers[slot] = Some(buffer);
            tracing::debug!("Cached wl_buffer for slot {slot}");
        }

        let buffer = self.cached_buffers[slot]
            .as_ref()
            .expect("BUG: cached buffer should exist after creation above");
        self.submit_buffer(buffer, info, request_frame)
    }

    /// Attach buffer, damage, optionally request frame callback, and commit.
    fn submit_buffer(
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
    pub fn invalidate_cached_buffers(&mut self) {
        let destroyed = invalidate_cached_wl_buffers(&mut self.cached_buffers);
        if destroyed > 0 {
            self.state.buffer_caching = false;
        }
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
    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> Result<bool> {
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
        // deck_widget_v1 does not have resize events; size is fixed.
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

    fn pending_buffer_count(&self) -> u32 {
        self.state.pending_buffers
    }

    fn can_submit_frame(&self, max_pending: u32) -> bool {
        self.state.pending_buffers < max_pending
    }

    fn frame_count(&self) -> u32 {
        self.state.frame_count
    }

    fn blocking_dispatch(&mut self) -> anyhow::Result<()> {
        DeckWidgetSurfaceClient::blocking_dispatch(self)
    }

    fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<bool> {
        DeckWidgetSurfaceClient::poll_dispatch(self, timeout_ms)
    }

    fn request_frame(&self) {
        DeckWidgetSurfaceClient::request_frame(self);
    }

    fn commit_buffer(&mut self, info: &DmaBufInfo, request_frame: bool) -> anyhow::Result<()> {
        DeckWidgetSurfaceClient::commit_buffer(self, info, request_frame)
    }

    fn commit_cached_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> anyhow::Result<()> {
        DeckWidgetSurfaceClient::commit_cached_buffer(self, info, slot, request_frame)
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
            })
            .collect()
    }
}

// ── Wayland protocol dispatch for DeckWidgetSurfaceState ───────────────

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
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            if state.buffer_caching {
                // Buffer caching mode: cached buffers are reused across
                // frames, so don't destroy them on release.
            } else {
                // Per-frame mode: destroy the buffer and decrement the counter.
                buffer.destroy();
                state.pending_buffers = state.pending_buffers.saturating_sub(1);
            }
        }
    }
}

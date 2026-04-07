// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fmt;

use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::egl::DmaBufInfo;

use super::common::{
    blocking_dispatch_impl, create_buffer_from_dmabuf, impl_common_dispatch,
    invalidate_cached_wl_buffers, poll_dispatch, submit_buffer_to_surface,
};
use super::{WidgetEvent, WidgetSurface};

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
    /// A frame callback fired -- widget should render.
    pub needs_render: bool,
    /// Surface was resized -- widget should recreate GPU buffers.
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

/// Wayland client for XDG toplevel surfaces with DMA-BUF buffer management.
///
/// Handles connection, global binding, surface creation, frame callbacks, and
/// buffer submission. The widget owns the render loop and calls
/// [`submit_buffer`](Self::submit_buffer) after each frame.
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

        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

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

        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &qh, ());
        let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());

        xdg_toplevel.set_title(title.to_owned());
        xdg_toplevel.set_app_id(app_id.to_owned());

        surface.commit();

        state.surface = Some(surface);
        state.xdg_surface = Some(xdg_surface);
        state.xdg_toplevel = Some(xdg_toplevel);

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

    fn submit_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> anyhow::Result<()> {
        XdgSurfaceClient::submit_buffer(self, info, slot, request_frame)
    }

    fn invalidate_cached_buffers(&mut self) {
        XdgSurfaceClient::invalidate_cached_buffers(self);
    }

    fn drain_events(&mut self) -> Vec<WidgetEvent> {
        Vec::new()
    }
}

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
            // XDG only supports slot-based cached wl_buffer submission, so
            // release is intentionally ignored here.
        }
    }
}

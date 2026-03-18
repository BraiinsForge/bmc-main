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

use crate::egl::DmaBufInfo;

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
    #[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
    fn submit_buffer(
        &self,
        buffer: &wl_buffer::WlBuffer,
        info: &DmaBufInfo,
        request_frame: bool,
    ) -> Result<()> {
        let qh = self.queue.handle();
        let surface = self.state.surface.as_ref().context("surface not created")?;

        surface.attach(Some(buffer), 0, 0);
        surface.damage_buffer(0, 0, info.width as i32, info.height as i32);

        if request_frame {
            surface.frame(&qh, ());
        }

        surface.commit();
        Ok(())
    }

    /// Invalidate all cached buffer slots.
    ///
    /// Destroys any cached `wl_buffer`s. Call this when the surface is
    /// resized or when the underlying DMA-BUF export buffers are recreated.
    pub fn invalidate_cached_buffers(&mut self) {
        let mut destroyed = 0_u32;
        for cached in &mut self.cached_buffers {
            if let Some(buf) = cached.take() {
                buf.destroy();
                destroyed += 1;
            }
        }
        if destroyed > 0 {
            tracing::debug!("Destroyed {destroyed} cached wl_buffer(s)");
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
        self.queue
            .blocking_dispatch(&mut self.state)
            .context("Wayland dispatch failed")?;
        Ok(())
    }

    /// Poll for Wayland events with a timeout, then dispatch pending events.
    ///
    /// Returns `Ok(true)` on all normal paths (events read, timeout, or
    /// already-queued events dispatched). Returns `Ok(false)` only if
    /// `poll(2)` fails with `EAGAIN`/`EWOULDBLOCK` (non-fatal, pending
    /// events still dispatched). A timeout of `-1` blocks indefinitely;
    /// `0` is non-blocking.
    ///
    /// This follows the `prepare_read → poll → read/cancel → dispatch_pending`
    /// pattern required by `wayland-client`.
    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> Result<bool> {
        self.conn.flush()?;

        let read_guard = self.queue.prepare_read();

        match read_guard {
            None => {
                // Events already queued — just dispatch them
                self.queue
                    .dispatch_pending(&mut self.state)
                    .context("Wayland dispatch failed")?;
                return Ok(true);
            }
            Some(guard) => {
                let fd = self.conn.as_fd();
                let mut pollfd = libc::pollfd {
                    fd: std::os::fd::AsRawFd::as_raw_fd(&fd),
                    events: libc::POLLIN,
                    revents: 0,
                };

                let poll_ret = unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) };

                match poll_ret.cmp(&0) {
                    std::cmp::Ordering::Greater => {
                        // Data available — read events
                        let _ = guard.read();
                    }
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
                                self.queue
                                    .dispatch_pending(&mut self.state)
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

        self.queue
            .dispatch_pending(&mut self.state)
            .context("Wayland dispatch failed")?;

        Ok(true)
    }
}

/// Create a `wl_buffer` from DMA-BUF info using the `linux-dmabuf` protocol.
#[must_use]
#[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
pub fn create_buffer_from_dmabuf(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<XdgSurfaceState>,
) -> wl_buffer::WlBuffer {
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

// ── Wayland protocol dispatch implementations ─────────────────────────

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

impl Dispatch<wl_compositor::WlCompositor, ()> for XdgSurfaceState {
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

impl Dispatch<wl_surface::WlSurface, ()> for XdgSurfaceState {
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

impl Dispatch<wl_callback::WlCallback, ()> for XdgSurfaceState {
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

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for XdgSurfaceState {
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

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for XdgSurfaceState {
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

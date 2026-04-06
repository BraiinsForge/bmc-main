// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fmt;

use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1;

use bmc_widget_protocol::client::{
    deck_widget_manager_v1::DeckWidgetManagerV1,
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};

use crate::egl::DmaBufInfo;
use crate::wayland::setting_from_protocol;

use super::common::{
    blocking_dispatch_impl, impl_common_dispatch, invalidate_cached_wl_buffers, poll_dispatch,
    submit_buffer_to_surface,
};
use super::{WidgetEvent, WidgetSurface, create_buffer_from_dmabuf};

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

/// `wl_buffer` lifecycle mode for a deck widget surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferMode {
    /// No buffer submission API has been used yet.
    Unknown,
    /// `commit_buffer()` creates a fresh `wl_buffer` per frame and destroys it
    /// on `wl_buffer::Release`.
    PerFrame,
    /// `commit_cached_buffer()` reuses a small set of cached `wl_buffer`s and
    /// treats `wl_buffer::Release` as a no-op.
    Cached,
}

/// Encapsulate submission-mode transitions and `wl_buffer.release` handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubmissionMode {
    mode: BufferMode,
}

impl Default for SubmissionMode {
    fn default() -> Self {
        Self {
            mode: BufferMode::Unknown,
        }
    }
}

impl SubmissionMode {
    fn enter_per_frame(&mut self) -> Result<()> {
        match self.mode {
            BufferMode::Unknown => {
                self.mode = BufferMode::PerFrame;
                tracing::debug!("Per-frame buffer mode enabled for deck_widget surface");
                Ok(())
            }
            BufferMode::PerFrame => Ok(()),
            BufferMode::Cached => {
                anyhow::bail!(
                    "commit_buffer() cannot be used after commit_cached_buffer() on DeckWidgetSurfaceClient"
                );
            }
        }
    }

    fn enter_cached(&mut self) -> Result<()> {
        match self.mode {
            BufferMode::Unknown => {
                self.mode = BufferMode::Cached;
                tracing::debug!("Cached buffer mode enabled for deck_widget surface");
                Ok(())
            }
            BufferMode::Cached => Ok(()),
            BufferMode::PerFrame => {
                anyhow::bail!(
                    "commit_cached_buffer() cannot be used after commit_buffer() on DeckWidgetSurfaceClient"
                );
            }
        }
    }

    fn handle_release(self, buffer: &wl_buffer::WlBuffer, pending_buffers: &mut u32) {
        match self.mode {
            BufferMode::Cached => {}
            BufferMode::PerFrame => {
                buffer.destroy();
                *pending_buffers = pending_buffers.saturating_sub(1);
            }
            BufferMode::Unknown => {
                tracing::warn!(
                    "Received wl_buffer.release before DeckWidgetSurfaceClient buffer mode was established"
                );
            }
        }
    }

    #[cfg(test)]
    fn current(self) -> BufferMode {
        self.mode
    }
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
    /// Number of `wl_buffer`s currently held by the compositor.
    pub pending_buffers: u32,
    /// Number of frames rendered (wrapping counter).
    pub frame_count: u32,
    /// Current `wl_buffer` lifecycle mode for this client.
    ///
    /// This is set by the first submission API used and then remains fixed for
    /// the lifetime of the client, so late `wl_buffer::Release` events from
    /// invalidated cached buffers are not misclassified as per-frame buffers.
    submission_mode: SubmissionMode,

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

    /// Switch to per-frame `wl_buffer` lifecycle mode.
    fn enable_per_frame_mode(&mut self) -> Result<()> {
        self.submission_mode.enter_per_frame()
    }

    /// Switch to cached `wl_buffer` lifecycle mode.
    fn enable_cached_mode(&mut self) -> Result<()> {
        self.submission_mode.enter_cached()
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
            .field("submission_mode", &self.submission_mode)
            .field("pending_events", &self.pending_events.len())
            .finish_non_exhaustive()
    }
}

/// Single-connection Wayland client for `deck_widget_v1` widgets with DMA-BUF.
///
/// Handles connection, global binding, surface creation, frame callbacks, and
/// buffer submission using the `deck_widget_v1` protocol. Widgets can either
/// submit per-frame buffers that are destroyed on release or cached buffers
/// that are reused across frames. The first submission API call fixes that
/// lifecycle mode for the lifetime of the client.
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
            pending_buffers: 0,
            frame_count: 0,
            submission_mode: SubmissionMode::default(),
            compositor: None,
            widget_manager: None,
            linux_dmabuf: None,
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
        self.state.enable_per_frame_mode()?;

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
    /// avoiding per-frame `wl_buffer` creation overhead. The first cached
    /// submission fixes the client in cached-buffer mode, which makes the
    /// `wl_buffer::Release` handler a no-op.
    ///
    /// Call [`invalidate_cached_buffers`](Self::invalidate_cached_buffers)
    /// when the surface is resized or the underlying DMA-BUF changes.
    pub fn commit_cached_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
        request_frame: bool,
    ) -> Result<()> {
        self.state.enable_cached_mode()?;

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
    ///
    /// This does not reset the submission mode: late `wl_buffer::Release`
    /// events from previously cached buffers must still be treated as cached
    /// releases, i.e. as no-ops.
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
            state
                .submission_mode
                .handle_release(buffer, &mut state.pending_buffers);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferMode, SubmissionMode};

    fn submission_mode() -> SubmissionMode {
        SubmissionMode::default()
    }

    #[test]
    fn submission_mode_allows_repeated_per_frame_mode() {
        let mut mode = submission_mode();

        assert!(mode.enter_per_frame().is_ok());
        assert!(mode.enter_per_frame().is_ok());
        assert_eq!(mode.current(), BufferMode::PerFrame);
    }

    #[test]
    fn submission_mode_allows_repeated_cached_mode() {
        let mut mode = submission_mode();

        assert!(mode.enter_cached().is_ok());
        assert!(mode.enter_cached().is_ok());
        assert_eq!(mode.current(), BufferMode::Cached);
    }

    #[test]
    fn submission_mode_rejects_switch_from_per_frame_to_cached() {
        let mut mode = submission_mode();
        mode.enter_per_frame()
            .expect("BUG: initial per-frame mode must be accepted");

        let err = mode
            .enter_cached()
            .expect_err("switching from per-frame to cached mode must fail");

        assert!(
            err.to_string()
                .contains("commit_cached_buffer() cannot be used after commit_buffer()")
        );
    }

    #[test]
    fn submission_mode_rejects_switch_from_cached_to_per_frame() {
        let mut mode = submission_mode();
        mode.enter_cached()
            .expect("BUG: initial cached mode must be accepted");

        let err = mode
            .enter_per_frame()
            .expect_err("switching from cached to per-frame mode must fail");

        assert!(
            err.to_string()
                .contains("commit_buffer() cannot be used after commit_cached_buffer()")
        );
    }
}

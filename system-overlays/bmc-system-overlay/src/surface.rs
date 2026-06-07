// Copyright (C) 2026  Braiins Systems s.r.o.

//! wlr-layer-shell Wayland client for system overlays.
//!
//! Mirrors `bmc_widget`'s `deck_widget` surface client, swapping the
//! `deck_widget_manager_v1` surface for a `zwlr_layer_shell_v1` layer
//! surface. Overlays self-pace their redraws off the framework's
//! tick/`next_wake` schedule, so this client never requests a
//! `wl_surface.frame` callback.

use std::time::{Duration, Instant};

use anyhow::Context;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_region, wl_registry, wl_seat, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use bmc_widget::egl::DmaBufInfo;
use bmc_widget::surface::{
    BufferSlotMap, PollOutcome, ReleasedBuffer, ReleasedBufferSet, create_buffer_from_dmabuf,
    drain_released_buffers, poll_dispatch, submit_buffer_to_surface,
};

/// Wayland protocol state for a layer-shell overlay surface.
///
/// Holds the bound globals, surface objects, configure handshake state, and
/// the dmabuf buffer-release bookkeeping that mirrors the `deck_widget`
/// client. Lives behind [`LayerSurfaceClient`], which owns the connection and
/// event queue.
struct State {
    /// Whether the event loop should keep running. Cleared on `Closed`.
    running: bool,

    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    seat: Option<wl_seat::WlSeat>,
    touch: Option<wl_touch::WlTouch>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,

    /// Set true on the first layer-surface Configure (after which we may map).
    configured: bool,
    /// Compositor-suggested size from the latest Configure.
    configured_size: (u32, u32),
    pending_touch: Vec<crate::overlay::TouchEvent>,
    /// Surface-dirty from a Configure/resize only. Overlays do not use
    /// compositor frame callbacks; redraw pacing is the framework's
    /// tick/next_wake job, so no wl_surface.frame is ever requested.
    needs_render: bool,

    buffer_slots: BufferSlotMap,
    released_buffers: ReleasedBufferSet,
}

impl Default for State {
    fn default() -> Self {
        Self {
            running: true,
            compositor: None,
            layer_shell: None,
            linux_dmabuf: None,
            seat: None,
            touch: None,
            surface: None,
            layer_surface: None,
            configured: false,
            configured_size: (0, 0),
            pending_touch: Vec::new(),
            needs_render: false,
            buffer_slots: BufferSlotMap::new(),
            released_buffers: ReleasedBufferSet::new(),
        }
    }
}

/// How long [`LayerSurfaceClient::connect`] waits for the compositor to send
/// the first Configure before giving up.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Single-connection Wayland client for a wlr-layer-shell overlay surface.
///
/// Connects to the compositor, binds the layer-shell global, creates and
/// configures a layer surface, and mints/attaches DMA-BUF buffers through the
/// shared `bmc-widget` helpers.
pub struct LayerSurfaceClient {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

impl std::fmt::Debug for LayerSurfaceClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerSurfaceClient")
            .field("running", &self.state.running)
            .field("configured", &self.state.configured)
            .field("configured_size", &self.state.configured_size)
            .field("pending_touch", &self.state.pending_touch.len())
            .field("buffer_slots", &self.state.buffer_slots.len())
            .field("released_buffers", &self.state.released_buffers.len())
            .finish_non_exhaustive()
    }
}

impl LayerSurfaceClient {
    /// Connect to the Wayland display, create a layer surface from `config`,
    /// and block until the compositor emits its first Configure.
    pub fn connect(config: &crate::overlay::LayerConfig) -> anyhow::Result<Self> {
        let conn =
            Connection::connect_to_env().map_err(|e| anyhow::anyhow!("wayland connect: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());

        let mut state = State::default();
        queue
            .roundtrip(&mut state)
            .map_err(|e| anyhow::anyhow!("roundtrip: {e}"))?;

        let compositor = state.compositor.clone().context("wl_compositor missing")?;
        let layer_shell = state
            .layer_shell
            .clone()
            .context("zwlr_layer_shell_v1 missing")?;
        anyhow::ensure!(state.linux_dmabuf.is_some(), "zwp_linux_dmabuf_v1 missing");

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            config.layer,
            config.namespace.clone(),
            &qh,
            (),
        );
        layer_surface.set_anchor(config.anchor);
        layer_surface.set_size(config.size.0, config.size.1);
        layer_surface.set_margin(
            config.margin_top,
            config.margin_right,
            config.margin_bottom,
            config.margin_left,
        );
        layer_surface.set_exclusive_zone(config.exclusive_zone);
        if matches!(config.input, crate::overlay::InputRegion::None) {
            let region = compositor.create_region(&qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
        }
        surface.commit();

        state.surface = Some(surface);
        state.layer_surface = Some(layer_surface);

        let deadline = Instant::now() + CONFIGURE_TIMEOUT;
        while !state.configured {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let remaining_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            match poll_dispatch(&conn, &mut queue, &mut state, remaining_ms)
                .map_err(|e| anyhow::anyhow!("dispatch awaiting configure: {e}"))?
            {
                PollOutcome::Events => {}
                PollOutcome::Timeout => {
                    anyhow::bail!(
                        "timed out after {:?} waiting for layer-surface configure",
                        CONFIGURE_TIMEOUT
                    );
                }
            }
        }

        tracing::info!(
            "Layer surface ready: {}x{} namespace={}",
            state.configured_size.0,
            state.configured_size.1,
            config.namespace,
        );

        Ok(Self { conn, queue, state })
    }

    /// Mint a `wl_buffer` from DMA-BUF info and register it for the given slot.
    pub fn mint_wl_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
    ) -> anyhow::Result<wl_buffer::WlBuffer> {
        let qh = self.queue.handle();
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 missing")?;
        let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
        self.state.buffer_slots.insert(buffer.id(), slot);
        Ok(buffer)
    }

    /// Attach `buffer`, damage the surface, and commit. Never requests a frame
    /// callback: overlays self-pace via the framework tick.
    pub fn submit_buffer_with_wl_buffer(
        &mut self,
        info: &DmaBufInfo,
        buffer: &wl_buffer::WlBuffer,
    ) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        let surface = self.state.surface.as_ref().context("surface not created")?;
        submit_buffer_to_surface(surface, &qh, buffer, info, false);
        Ok(())
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.state.configured_size
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.state.running
    }

    pub fn take_needs_render(&mut self) -> bool {
        std::mem::take(&mut self.state.needs_render)
    }

    pub fn drain_touch(&mut self) -> Vec<crate::overlay::TouchEvent> {
        std::mem::take(&mut self.state.pending_touch)
    }

    pub fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        drain_released_buffers(&self.state.buffer_slots, &mut self.state.released_buffers)
    }

    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<()> {
        poll_dispatch(&self.conn, &mut self.queue, &mut self.state, timeout_ms)
            .map(|_outcome| ())
            .map_err(|e| anyhow::anyhow!("poll_dispatch: {e}"))
    }

    #[must_use]
    pub fn connection_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::{AsFd, AsRawFd};
        self.conn.as_fd().as_raw_fd()
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.conn
            .flush()
            .map_err(|e| anyhow::anyhow!("wl flush: {e}"))
    }

    #[expect(
        dead_code,
        reason = "consumed by the render-target wiring landing in a later task"
    )]
    pub(crate) fn dmabuf(&self) -> &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 {
        self.state
            .linux_dmabuf
            .as_ref()
            .expect("BUG: dmabuf checked at connect")
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
                    state.compositor = Some(compositor);
                }
                "zwlr_layer_shell_v1" => {
                    let layer_shell = registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    state.layer_shell = Some(layer_shell);
                }
                "zwp_linux_dmabuf_v1" => {
                    let dmabuf = registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    state.linux_dmabuf = Some(dmabuf);
                }
                "wl_seat" if state.seat.is_none() => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(9), qh, ());
                    state.seat = Some(seat);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                state.configured_size = (width, height);
                state.configured = true;
                state.needs_render = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.running = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            let buffer_id = buffer.id();
            if state.buffer_slots.contains_key(&buffer_id) {
                state.released_buffers.insert(buffer_id);
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
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

impl Dispatch<wl_surface::WlSurface, ()> for State {
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

impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        _: zwp_linux_buffer_params_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            let has_touch = caps.contains(wl_seat::Capability::Touch);
            if has_touch && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            } else if !has_touch && let Some(touch) = state.touch.take() {
                touch.release();
            }
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
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
                    .pending_touch
                    .push(crate::overlay::TouchEvent::Down { id, x, y });
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state
                    .pending_touch
                    .push(crate::overlay::TouchEvent::Motion { id, x, y });
            }
            wl_touch::Event::Up { id, .. } => {
                state
                    .pending_touch
                    .push(crate::overlay::TouchEvent::Up { id });
            }
            wl_touch::Event::Cancel => {
                state.pending_touch.push(crate::overlay::TouchEvent::Cancel);
            }
            wl_touch::Event::Frame => tracing::trace!("wl_touch::Frame"),
            wl_touch::Event::Shape { .. } => tracing::trace!("wl_touch::Shape (ignored)"),
            wl_touch::Event::Orientation { .. } => {
                tracing::trace!("wl_touch::Orientation (ignored)");
            }
            other => tracing::debug!(?other, "unhandled wl_touch event"),
        }
    }
}

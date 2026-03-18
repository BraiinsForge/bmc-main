// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland client for the WASM widget.
//!
//! Uses `poll(2)` instead of `blocking_dispatch` so the runtime's delayed
//! frames fire correctly without blocking Wayland event processing.
//! GPU resources are initialized lazily on first visibility to avoid
//! exhausting the GC400's 4-context limit on inactive scenes.

use crate::egl::{DmaBufInfo, EglState};
use anyhow::{Context, Result};
use bmc_wasm_runtime::RenderStatus;
use bmc_wasm_runtime::WasmWidgetRuntime;
use bmc_wasm_runtime::renderer::Renderer;
use bmc_widget_protocol::client::{
    deck_widget_manager_v1::DeckWidgetManagerV1,
    deck_widget_surface_v1::{self, DeckWidgetSurfaceV1},
};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Instant;
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

/// Rendering state — created lazily on first visibility, kept alive forever.
///
/// EGL context and WASM runtime persist across visibility changes. On hide we
/// just stop rendering (poll blocks on -1). On show we resume immediately with
/// the last widget state — no re-init, no flicker, no GPU resource churn.
struct RenderState {
    egl: EglState,
    runtime: WasmWidgetRuntime,
    last_frame: Instant,
    frame_count: u64,
}

/// WASM widget parameters from `DECK_PARAMS` JSON.
#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct WasmParams {
    /// Path to the `.wasm` file to execute.
    wasm_path: Option<String>,
}

/// Wayland client state.
pub struct WaylandClient {
    conn: Connection,
    queue: EventQueue<WaylandState>,
    state: WaylandState,
}

/// Internal state for Wayland protocol handling.
struct WaylandState {
    running: bool,
    compositor: Option<wl_compositor::WlCompositor>,
    widget_manager: Option<DeckWidgetManagerV1>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    surface: Option<wl_surface::WlSurface>,
    widget_surface: Option<DeckWidgetSurfaceV1>,
    width: u32,
    height: u32,
    needs_render: bool,
    pending_buffers: u32,
}

impl WaylandClient {
    /// Connect to the Wayland display.
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;

        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let instance_id = bmc_widget::read_instance_id().context("DECK_INSTANCE_ID not set")?;
        let size = bmc_widget::read_size().context("DECK_SIZE not set")?;
        let width = size.width;
        let height = size.height;

        tracing::info!("Widget config: instance_id={instance_id}, size={width}x{height}",);

        let mut state = WaylandState {
            running: true,
            compositor: None,
            widget_manager: None,
            linux_dmabuf: None,
            surface: None,
            widget_surface: None,
            width,
            height,
            needs_render: false,
            pending_buffers: 0,
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
            .context("deck_widget_manager_v1 not available — is this the BMC compositor?")?;

        let surface = compositor.create_surface(&qh, ());
        let widget_surface =
            widget_manager.get_widget_surface(&surface, instance_id.clone(), &qh, ());

        tracing::info!("Registered widget surface with instance_id: {instance_id}");
        surface.commit();

        state.surface = Some(surface);
        state.widget_surface = Some(widget_surface);

        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip after widget registration")?;

        tracing::info!("Widget surface ready: {}x{}", state.width, state.height);

        Ok(Self { conn, queue, state })
    }

    /// Run the event loop with poll(2)-based frame scheduling.
    ///
    /// EGL context and WASM runtime are initialized lazily on first visibility.
    #[expect(
        clippy::too_many_lines,
        reason = "render loop is a single sequential flow"
    )]
    pub fn run(&mut self) -> Result<()> {
        let qh = self.queue.handle();

        // Parse WASM path from DECK_PARAMS (cheap — no GPU resources yet)
        let params: WasmParams =
            bmc_widget::read_params().context("Failed to parse DECK_PARAMS")?;
        let wasm_path: PathBuf = params
            .wasm_path
            .context("wasmPath not set in DECK_PARAMS")?
            .into();

        tracing::info!("WASM path: {}", wasm_path.display());

        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 not available")?
            .clone();

        // Render state is None until first initialized.
        let mut render: Option<RenderState> = None;

        let wayland_raw_fd = std::os::fd::AsRawFd::as_raw_fd(&self.conn.as_fd());

        while self.state.running {
            // Compute poll timeout from WASM runtime's frame scheduling
            let timeout_ms = self.compute_poll_timeout(render.as_ref());

            let mut pollfd = libc::pollfd {
                fd: wayland_raw_fd,
                events: libc::POLLIN,
                revents: 0,
            };

            // Flush outgoing Wayland requests before polling
            self.conn.flush()?;

            // Prepare read guard before polling (required by wayland-client).
            // None means events are already queued — skip poll, just dispatch.
            if let Some(read_guard) = self.queue.prepare_read() {
                let poll_ret = unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) };

                match poll_ret.cmp(&0) {
                    std::cmp::Ordering::Greater => {
                        let _ = read_guard.read();
                    }
                    std::cmp::Ordering::Equal => {
                        drop(read_guard);
                        self.state.needs_render = true;
                    }
                    std::cmp::Ordering::Less => {
                        // poll error
                        let err = std::io::Error::last_os_error();
                        drop(read_guard);
                        if err.kind() != std::io::ErrorKind::Interrupted {
                            return Err(err).context("poll(2) on Wayland fd failed");
                        }
                    }
                }
            }

            // Dispatch any pending events
            self.queue
                .dispatch_pending(&mut self.state)
                .context("Wayland dispatch failed")?;

            // --- Lazy init: create EGL + WASM runtime on first visibility ---
            if render.is_none() {
                tracing::info!("Initializing GPU resources");

                let wasm_bytes = std::fs::read(&wasm_path).with_context(|| {
                    format!("Failed to read WASM file: {}", wasm_path.display())
                })?;
                tracing::info!("WASM loaded: {} bytes", wasm_bytes.len());

                let mut egl = EglState::new(self.state.width, self.state.height)?;
                tracing::info!("GBM-based EGL initialized");

                let fbo_id = egl.begin_frame()?;
                let mut runtime = unsafe {
                    WasmWidgetRuntime::new(
                        &wasm_bytes,
                        EglState::get_proc_address,
                        self.state.width,
                        self.state.height,
                        fbo_id,
                        WasmWidgetRuntime::FUEL_PER_FRAME,
                        bmc_wasm_protocol::FormatPreferences::default(),
                    )
                }?;
                let (major, minor, patch) = runtime.sdk_version();
                tracing::info!(
                    "WASM runtime initialized, SDK version {}.{}.{}",
                    major,
                    minor,
                    patch
                );

                // Render + commit first frame immediately
                runtime
                    .renderer()
                    .begin_frame(self.state.width, self.state.height, 1.0);
                runtime.deliver_fetch_responses();
                match runtime.render(0)? {
                    RenderStatus::Ok => {}
                    status @ (RenderStatus::FuelExhausted | RenderStatus::Dead) => {
                        tracing::warn!("First frame render status: {status:?}");
                    }
                }
                runtime.renderer().flush();

                egl.blit_to_export()?;
                let dmabuf_info = egl.end_frame()?;
                let buffer = create_buffer_from_dmabuf(&linux_dmabuf, &dmabuf_info, &qh);

                if let Some(ref surface) = self.state.surface {
                    surface.attach(Some(&buffer), 0, 0);
                    #[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
                    surface.damage_buffer(
                        0,
                        0,
                        dmabuf_info.width as i32,
                        dmabuf_info.height as i32,
                    );
                    surface.frame(&qh, ());
                    surface.commit();
                    self.state.pending_buffers += 1;
                }
                tracing::info!("First frame rendered and committed");

                render = Some(RenderState {
                    egl,
                    runtime,
                    last_frame: Instant::now(),
                    frame_count: 1,
                });

                self.state.needs_render = false;
                continue;
            }

            // Not yet initialized — wait for first visibility
            let Some(rs) = render.as_mut() else {
                continue;
            };

            let should_render = self.state.needs_render;

            if should_render && self.state.pending_buffers < 3 {
                self.state.needs_render = false;
                rs.frame_count += 1;

                // Compute delta time
                let now = Instant::now();
                #[expect(clippy::cast_possible_truncation, reason = "delta_ms fits in u32")]
                let delta_ms = now.duration_since(rs.last_frame).as_millis() as u32;
                rs.last_frame = now;

                tracing::debug!(
                    frame = rs.frame_count,
                    delta_ms,
                    pending_bufs = self.state.pending_buffers,
                    wants_frame = rs.runtime.wants_next_frame(),
                    delay = ?rs.runtime.next_frame_delay(),
                    pending_fetches = rs.runtime.has_pending_fetches(),
                    "rendering frame"
                );

                // Begin frame
                let _fbo_id = rs.egl.begin_frame()?;
                rs.runtime
                    .renderer()
                    .begin_frame(self.state.width, self.state.height, 1.0);

                // Deliver pending fetch responses
                rs.runtime.deliver_fetch_responses();

                // Render WASM frame
                let status = rs.runtime.render(delta_ms)?;
                rs.runtime.renderer().flush();

                match status {
                    RenderStatus::Ok => {}
                    RenderStatus::FuelExhausted => {
                        tracing::warn!(frame = rs.frame_count, "widget exceeded fuel budget");
                    }
                    RenderStatus::Dead => {
                        tracing::error!("widget killed (repeated fuel overages), shutting down");
                        self.state.running = false;
                    }
                }

                // Blit staging → export with Y-flip
                rs.egl.blit_to_export()?;
                let dmabuf_info = rs.egl.end_frame()?;
                let buffer = create_buffer_from_dmabuf(&linux_dmabuf, &dmabuf_info, &qh);

                if let Some(ref surface) = self.state.surface {
                    surface.attach(Some(&buffer), 0, 0);
                    #[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
                    surface.damage_buffer(
                        0,
                        0,
                        dmabuf_info.width as i32,
                        dmabuf_info.height as i32,
                    );

                    // Only request a frame callback for immediate animation.
                    // Delayed frames and fetch polling are driven by poll(2)
                    // timeout — requesting a callback for those would fire at
                    // vsync (~16ms), defeating the delay.
                    let wants_immediate_frame =
                        rs.runtime.wants_next_frame() && rs.runtime.next_frame_delay().is_none();
                    if wants_immediate_frame {
                        surface.frame(&qh, ());
                    }

                    surface.commit();
                    self.state.pending_buffers += 1;
                }
            }
        }

        tracing::info!("WASM widget shutting down");
        Ok(())
    }

    /// Compute the `poll(2)` timeout in milliseconds.
    fn compute_poll_timeout(&self, render: Option<&RenderState>) -> i32 {
        if self.state.needs_render {
            0
        } else if let Some(rs) = render {
            if rs.runtime.wants_next_frame() {
                match rs.runtime.next_frame_delay() {
                    Some(delay_ms) => i32::try_from(delay_ms).unwrap_or(i32::MAX),
                    None => 0,
                }
            } else if rs.runtime.has_pending_fetches() {
                100
            } else {
                -1
            }
        } else {
            0 // not yet initialized — init immediately
        }
    }
}

/// Create a `wl_buffer` from DMA-BUF info using `linux-dmabuf` protocol.
fn create_buffer_from_dmabuf(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<WaylandState>,
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

    #[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
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

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
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

impl Dispatch<wl_compositor::WlCompositor, ()> for WaylandState {
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

impl Dispatch<wl_surface::WlSurface, ()> for WaylandState {
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

impl Dispatch<DeckWidgetManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &DeckWidgetManagerV1,
        _: bmc_widget_protocol::client::deck_widget_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<DeckWidgetSurfaceV1, ()> for WaylandState {
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
                tracing::debug!("Received setting update: type={setting_type:?}, value={value}",);
            }
            deck_widget_surface_v1::Event::Shutdown => {
                tracing::info!("Shutdown requested by compositor");
                state.running = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.needs_render = true;
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for WaylandState {
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

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for WaylandState {
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

impl Dispatch<wl_buffer::WlBuffer, ()> for WaylandState {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
            state.pending_buffers = state.pending_buffers.saturating_sub(1);
        }
    }
}

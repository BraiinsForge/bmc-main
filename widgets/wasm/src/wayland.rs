// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland client implementation for WASM widget
//!
//! Handles connection to the Wayland compositor, surface creation,
//! and frame callback management. Uses the custom deck_widget_v1 protocol
//! to register with the BMC compositor.
//!
//! This is a simplified version of the settings widget, with the
//! FemtoVG renderer replaced by bmc-wasm-runtime's WasmWidgetRuntime.

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
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

/// Wayland client state
pub struct WaylandClient {
    /// Wayland connection
    #[expect(dead_code, reason = "kept alive for protocol operations")]
    conn: Connection,
    /// Event queue
    queue: EventQueue<WaylandState>,
    /// Client state
    state: WaylandState,
}

/// Internal state for Wayland protocol handling
struct WaylandState {
    /// Whether we should keep running
    running: bool,
    /// Compositor global
    compositor: Option<wl_compositor::WlCompositor>,
    /// Widget manager global (deck_widget_v1 protocol)
    widget_manager: Option<DeckWidgetManagerV1>,
    /// Linux DMA-BUF global
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    /// Our surface
    surface: Option<wl_surface::WlSurface>,
    /// Widget surface role (deck_widget_surface_v1)
    widget_surface: Option<DeckWidgetSurfaceV1>,
    /// Current width
    width: u32,
    /// Current height
    height: u32,
    /// Frame count (for logging)
    frame_count: u32,
    /// Whether we need to render
    needs_render: bool,
    /// Whether size changed (needs EGL resize)
    size_changed: bool,
    /// Number of buffers currently held by compositor (not yet released)
    pending_buffers: u32,
    /// Path to WASM file (from DECK_PARAMS)
    wasm_path: String,
}

impl WaylandClient {
    /// Connect to the Wayland display
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;

        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        // Get widget dimensions and instance ID from environment (set by coordinator)
        let width = std::env::var("DECK_WIDTH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1280);
        let height = std::env::var("DECK_HEIGHT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(480);
        let instance_id = std::env::var("DECK_INSTANCE_ID")
            .context("DECK_INSTANCE_ID environment variable not set")?;

        // Get WASM path from DECK_PARAMS (JSON format)
        let wasm_path = parse_wasm_path_from_params()?;

        tracing::info!(
            "Widget config: instance_id={}, size={}x{}, wasm={}",
            instance_id,
            width,
            height,
            wasm_path
        );

        let mut state = WaylandState {
            running: true,
            compositor: None,
            widget_manager: None,
            linux_dmabuf: None,
            surface: None,
            widget_surface: None,
            width,
            height,
            frame_count: 0,
            needs_render: false,
            size_changed: false,
            pending_buffers: 0,
            wasm_path,
        };

        // Roundtrip to get globals
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip for globals")?;

        // Verify we have required globals
        let compositor = state
            .compositor
            .as_ref()
            .context("wl_compositor not available")?;
        let widget_manager = state
            .widget_manager
            .as_ref()
            .context("deck_widget_manager_v1 not available - is this the BMC compositor?")?;

        // Create surface
        let surface = compositor.create_surface(&qh, ());

        // Register with compositor via deck_widget_v1 protocol
        let widget_surface =
            widget_manager.get_widget_surface(&surface, instance_id.clone(), &qh, ());

        tracing::info!(
            "Registered widget surface with instance_id: {}",
            instance_id
        );

        // Commit surface
        surface.commit();

        state.surface = Some(surface);
        state.widget_surface = Some(widget_surface);

        // Roundtrip to ensure registration is processed
        queue
            .roundtrip(&mut state)
            .context("Failed to roundtrip after widget registration")?;

        tracing::info!("Widget surface ready: {}x{}", state.width, state.height);

        Ok(Self { conn, queue, state })
    }

    /// Run the event loop with EGL rendering and WASM runtime
    #[expect(clippy::too_many_lines, reason = "Event loop is inherently complex")]
    pub fn run(&mut self) -> Result<()> {
        let qh = self.queue.handle();

        // Initialize GBM-based EGL (two-FBO pipeline for FemtoVG Y-flip)
        let mut egl = EglState::new(self.state.width, self.state.height)?;

        tracing::info!("GBM-based EGL initialized, loading WASM runtime");

        // Allocate first buffer and get its FBO for FemtoVG's screen target.
        let fbo_id = egl.begin_frame()?;

        // Load WASM bytes
        let wasm_bytes = std::fs::read(&self.state.wasm_path)
            .with_context(|| format!("Failed to read WASM file: {}", self.state.wasm_path))?;

        // Create WASM runtime with GL function loader
        let mut runtime = unsafe {
            WasmWidgetRuntime::new(
                &wasm_bytes,
                |symbol| smithay::backend::egl::get_proc_address(symbol),
                self.state.width,
                self.state.height,
                fbo_id,
                WasmWidgetRuntime::FUEL_PER_FRAME,
                bmc_wasm_protocol::FormatPreferences::default(),
            )?
        };

        let (major, minor, patch) = runtime.sdk_version();
        tracing::info!(
            "WASM runtime initialized, SDK version {}.{}.{}",
            major,
            minor,
            patch
        );

        // Verify we have linux-dmabuf
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 not available")?
            .clone();

        // Render first frame immediately to give compositor something to display
        {
            runtime
                .renderer()
                .begin_frame(self.state.width, self.state.height, 1.0);
            match runtime.render(0)? {
                RenderStatus::Ok => {}
                status => tracing::warn!("First frame render status: {status:?}"),
            }
            runtime.renderer().flush();

            // Blit staging -> export FBO with Y-flip
            egl.blit_to_export()?;

            // End frame — finish GL, export DMA-BUF
            let dmabuf_info = egl.end_frame()?;
            let buffer = create_buffer_from_dmabuf(&linux_dmabuf, &dmabuf_info, &qh);

            if let Some(ref surface) = self.state.surface {
                surface.attach(Some(&buffer), 0, 0);
                #[expect(clippy::cast_possible_wrap, reason = "surface dimensions fit in i32")]
                surface.damage_buffer(0, 0, dmabuf_info.width as i32, dmabuf_info.height as i32);
                surface.frame(&qh, ());
                surface.commit();
                self.state.pending_buffers += 1;
            }
            tracing::info!("First frame rendered and committed");
        }

        let mut last_frame = std::time::Instant::now();

        while self.state.running {
            // Dispatch Wayland events
            self.queue
                .blocking_dispatch(&mut self.state)
                .context("Wayland dispatch failed")?;

            // Handle resize if needed
            if self.state.size_changed {
                egl.resize(self.state.width, self.state.height);
                self.state.size_changed = false;
            }

            // Render if we got a frame callback and not too many buffers pending
            // Limit to 3 pending buffers to avoid GPU memory exhaustion
            if self.state.needs_render && self.state.pending_buffers < 3 {
                self.state.needs_render = false;

                let now = std::time::Instant::now();
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Frame delta is always small (< 1000ms typically), safe to truncate"
                )]
                let delta_ms = now.duration_since(last_frame).as_millis() as u32;
                last_frame = now;

                // Begin frame — bind staging FBO, clear
                let _fbo_id = egl.begin_frame()?;

                // Deliver any completed HTTP fetch responses before rendering
                runtime.deliver_fetch_responses();

                // Render to the staging FBO via WASM runtime
                runtime
                    .renderer()
                    .begin_frame(self.state.width, self.state.height, 1.0);
                match runtime.render(delta_ms) {
                    Ok(RenderStatus::Ok) => {}
                    Ok(RenderStatus::FuelExhausted) => {
                        tracing::warn!("Widget exceeded fuel budget");
                    }
                    Ok(RenderStatus::Dead) => {
                        tracing::error!("Widget killed (repeated fuel overages), exiting");
                        self.state.running = false;
                    }
                    Err(e) => {
                        tracing::error!("WASM render error: {e}");
                    }
                }
                runtime.renderer().flush();

                // Blit staging -> export FBO with Y-flip
                egl.blit_to_export()?;

                // End frame — finish GL, export DMA-BUF
                let dmabuf_info = egl.end_frame()?;

                // Create wl_buffer from DMA-BUF
                let buffer = create_buffer_from_dmabuf(&linux_dmabuf, &dmabuf_info, &qh);

                // Attach buffer to surface
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

                self.state.frame_count += 1;
                if self.state.frame_count % 60 == 0 {
                    tracing::debug!(
                        "Frame {} (pending: {})",
                        self.state.frame_count,
                        self.state.pending_buffers
                    );
                }

                // Check if WASM wants another frame (animation) or has pending
                // delayed fetches that need polling
                if runtime.wants_next_frame() || runtime.has_pending_fetches() {
                    if let Some(delay_ms) = runtime.next_frame_delay() {
                        // Cap sleep to 100ms so Wayland events (shutdown, buffer
                        // release, resize) are still processed promptly
                        let capped = delay_ms.min(100);
                        std::thread::sleep(std::time::Duration::from_millis(u64::from(capped)));
                    }
                    self.state.needs_render = true;
                }
            }
        }

        Ok(())
    }
}

/// Parse WASM path from DECK_PARAMS environment variable (JSON format)
fn parse_wasm_path_from_params() -> Result<String> {
    let params_json =
        std::env::var("DECK_PARAMS").context("DECK_PARAMS environment variable not set")?;

    // Parse JSON to extract wasmPath
    let params: serde_json::Value =
        serde_json::from_str(&params_json).context("Failed to parse DECK_PARAMS as JSON")?;

    params
        .get("wasmPath")
        .and_then(|v| v.as_str())
        .map(String::from)
        .context("wasmPath not found in DECK_PARAMS")
}

/// Create a wl_buffer from DMA-BUF info using linux-dmabuf protocol
fn create_buffer_from_dmabuf(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<WaylandState>,
) -> wl_buffer::WlBuffer {
    // Create buffer params
    let params = linux_dmabuf.create_params(qh, ());

    // Add the plane (single plane for XRGB8888)
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

// ── Wayland protocol implementations ──────────────────────────────────

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
                    state.compositor = Some(registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version,
                        qh,
                        (),
                    ));
                    tracing::debug!("Bound wl_compositor v{}", version);
                }
                "deck_widget_manager_v1" => {
                    state.widget_manager =
                        Some(registry.bind::<DeckWidgetManagerV1, _, _>(name, version, qh, ()));
                    tracing::debug!("Bound deck_widget_manager_v1 v{}", version);
                }
                "zwp_linux_dmabuf_v1" => {
                    state.linux_dmabuf = Some(
                        registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                            name,
                            version,
                            qh,
                            (),
                        ),
                    );
                    tracing::debug!("Bound zwp_linux_dmabuf_v1 v{}", version);
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
        // wl_compositor has no events
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
        // wl_surface has no events we need to handle
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

impl Dispatch<DeckWidgetManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &DeckWidgetManagerV1,
        _: bmc_widget_protocol::client::deck_widget_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // deck_widget_manager_v1 has no events we need
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
            deck_widget_surface_v1::Event::Shutdown => {
                tracing::info!("Received shutdown event");
                state.running = false;
            }
            deck_widget_surface_v1::Event::Setting {
                setting_type,
                value,
            } => {
                tracing::debug!("Setting: {:?} = {}", setting_type, value);
                // Could pass settings to WASM if needed
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // We use create_immed, so we don't need to handle format events
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        _: zwp_linux_buffer_params_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // We use create_immed, so we don't need to handle created/failed events
    }
}

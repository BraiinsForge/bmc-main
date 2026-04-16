// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland client for the WASM widget.
//!
//! Uses [`bmc_widget::surface::WidgetSurface`] for Wayland connection,
//! surface management, and DMA-BUF buffer submission. This module only
//! contains the WASM render loop and frame scheduling logic.

use crate::egl::EglState;
use anyhow::{Context, Result};
use bmc_wasm_runtime::renderer::Renderer;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};
use bmc_widget::surface::{DeckWidgetSurfaceClient, WidgetEvent, WidgetSurface};
use std::path::PathBuf;
use std::time::Instant;

/// Rendering state — created lazily, kept alive forever.
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

/// Wayland client for the WASM widget.
pub struct WaylandClient {
    surface: DeckWidgetSurfaceClient,
}

impl WaylandClient {
    /// Connect to the Wayland display.
    pub fn connect() -> Result<Self> {
        let instance_id = bmc_widget::read_instance_id().context("DECK_INSTANCE_ID not set")?;
        let size = bmc_widget::read_size().context("DECK_SIZE not set")?;

        tracing::info!(
            "Widget config: instance_id={instance_id}, size={}x{}",
            size.width,
            size.height,
        );

        let surface = DeckWidgetSurfaceClient::connect(&instance_id, size.width, size.height)?;

        Ok(Self { surface })
    }

    /// Run the event loop with poll(2)-based frame scheduling.
    #[expect(
        clippy::too_many_lines,
        reason = "render loop is a single sequential flow"
    )]
    pub fn run(&mut self) -> Result<()> {
        let params: WasmParams =
            bmc_widget::read_params().context("Failed to parse DECK_PARAMS")?;
        let wasm_path: PathBuf = params
            .wasm_path
            .context("wasmPath not set in DECK_PARAMS")?
            .into();

        tracing::info!("WASM path: {}", wasm_path.display());

        let mut render: Option<RenderState> = None;

        while self.surface.running() {
            let timeout_ms = self.compute_poll_timeout(render.as_ref());
            self.surface.poll_dispatch(timeout_ms)?;

            // On poll timeout, treat as a frame scheduling tick
            if timeout_ms >= 0 {
                self.surface.mark_needs_render();
            }

            // Process protocol events
            for event in self.surface.drain_events() {
                match event {
                    WidgetEvent::Setting(update) => {
                        tracing::debug!("Setting update: {update:?}");
                    }
                    WidgetEvent::Shutdown => {
                        // Already handled by the client (sets running=false)
                    }
                    WidgetEvent::TouchDown { .. }
                    | WidgetEvent::TouchMotion { .. }
                    | WidgetEvent::TouchUp { .. }
                    | WidgetEvent::TouchCancel => todo!(),
                }
            }

            // --- Lazy init: create EGL + WASM runtime ---
            if render.is_none() {
                tracing::info!("Initializing GPU resources");

                let wasm_bytes = std::fs::read(&wasm_path).with_context(|| {
                    format!("Failed to read WASM file: {}", wasm_path.display())
                })?;
                tracing::info!("WASM loaded: {} bytes", wasm_bytes.len());

                let w = self.surface.width();
                let h = self.surface.height();

                let mut egl = EglState::new(w, h)?;
                tracing::info!("GBM-based EGL initialized");

                let fbo_id = egl.begin_frame()?;
                let mut runtime = unsafe {
                    WasmWidgetRuntime::new(
                        &wasm_bytes,
                        EglState::get_proc_address,
                        w,
                        h,
                        fbo_id,
                        RuntimeConfig::default(),
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
                runtime.renderer().begin_frame(w, h, 1.0);
                runtime.deliver_fetch_responses();
                match runtime.render(0)? {
                    RenderStatus::Ok => {}
                    status @ (RenderStatus::FuelExhausted | RenderStatus::Dead) => {
                        tracing::warn!("First frame render status: {status:?}");
                    }
                }
                runtime.renderer().flush();

                egl.blit_to_export()?;
                let (dmabuf_info, slot) = egl.end_frame()?;
                self.surface.submit_buffer(&dmabuf_info, slot, true)?;
                tracing::info!("First frame rendered and committed");

                render = Some(RenderState {
                    egl,
                    runtime,
                    last_frame: Instant::now(),
                    frame_count: 1,
                });

                // Clear the render flag since we just rendered
                let _ = self.surface.take_render_requested();
                continue;
            }

            let rs = render
                .as_mut()
                .expect("BUG: render should be Some after init");

            if self.surface.needs_render() {
                let _ = self.surface.take_render_requested();
                rs.frame_count += 1;

                let now = Instant::now();
                #[expect(clippy::cast_possible_truncation, reason = "delta_ms fits in u32")]
                let delta_ms = now.duration_since(rs.last_frame).as_millis() as u32;
                rs.last_frame = now;

                let w = self.surface.width();
                let h = self.surface.height();

                tracing::debug!(
                    frame = rs.frame_count,
                    delta_ms,
                    wants_frame = rs.runtime.wants_next_frame(),
                    delay = ?rs.runtime.next_frame_delay(),
                    pending_fetches = rs.runtime.has_pending_fetches(),
                    "rendering frame"
                );

                // Begin frame
                let _fbo_id = rs.egl.begin_frame()?;
                rs.runtime.renderer().begin_frame(w, h, 1.0);

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
                        self.surface.request_shutdown();
                    }
                }

                // Blit staging → export with Y-flip
                rs.egl.blit_to_export()?;

                let wants_immediate_frame =
                    rs.runtime.wants_next_frame() && rs.runtime.next_frame_delay().is_none();

                let (dmabuf_info, slot) = rs.egl.end_frame()?;
                self.surface
                    .submit_buffer(&dmabuf_info, slot, wants_immediate_frame)?;
            }
        }

        tracing::info!("WASM widget shutting down");
        Ok(())
    }

    /// Compute the `poll(2)` timeout in milliseconds.
    fn compute_poll_timeout(&self, render: Option<&RenderState>) -> i32 {
        if self.surface.needs_render() {
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

// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Wayland client for the WASM widget.
//!
//! Uses [`bmc_widget::surface::WidgetSurface`] for Wayland connection,
//! surface management, and DMA-BUF buffer submission. This module only
//! contains the WASM render loop and frame scheduling logic.

use crate::egl::EglState;
use anyhow::{Context, Result};
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};
use bmc_widget::surface::{DeckWidgetSurfaceClient, WidgetEvent, WidgetSurface};
use std::path::Path;
use std::time::Instant;

/// Rendering state — created lazily, kept alive forever.
struct RenderState {
    egl: EglState,
    runtime: WasmWidgetRuntime,
    last_frame: Instant,
    /// Baseline for monotonic_ms passed to the wasm runtime.
    monotonic_origin: Instant,
    frame_count: u64,
}

impl RenderState {
    fn monotonic_ms(&self, now: Instant) -> u64 {
        u64::try_from(now.duration_since(self.monotonic_origin).as_millis()).unwrap_or(u64::MAX)
    }
}

/// Wayland client for the WASM widget.
pub struct WaylandClient {
    surface: DeckWidgetSurfaceClient,
}

impl WaylandClient {
    /// Connect to the Wayland display and read the initial configure batch.
    pub fn connect() -> Result<Self> {
        let (surface, initial) = DeckWidgetSurfaceClient::connect()?;

        tracing::info!("Widget config: {}x{}", initial.width, initial.height);

        Ok(Self { surface })
    }

    /// Run the event loop with poll(2)-based frame scheduling.
    #[expect(
        clippy::too_many_lines,
        reason = "render loop is a single sequential flow"
    )]
    pub fn run(&mut self, wasm_path: &Path) -> Result<()> {
        let mut render: Option<RenderState> = None;

        while self.surface.running() {
            let timeout_ms = self.compute_poll_timeout(render.as_ref());
            let outcome = self.surface.poll_dispatch(timeout_ms)?;

            // Only a real timeout advances delayed frame scheduling.
            if outcome == bmc_widget::surface::PollOutcome::Timeout {
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
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "wl_touch f64 → f32 is lossless for pixel coordinates"
                    )]
                    WidgetEvent::TouchDown { x, y, .. } => {
                        if let Some(rs) = render.as_mut() {
                            use bmc_render::interaction::TouchEvent;
                            rs.runtime.push_touch_event(TouchEvent::Down {
                                x: x as f32,
                                y: y as f32,
                            });
                            self.surface.mark_needs_render();
                        }
                    }
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "wl_touch f64 → f32 is lossless for pixel coordinates"
                    )]
                    WidgetEvent::TouchMotion { x, y, .. } => {
                        if let Some(rs) = render.as_mut() {
                            use bmc_render::interaction::TouchEvent;
                            rs.runtime.push_touch_event(TouchEvent::Move {
                                x: x as f32,
                                y: y as f32,
                            });
                            self.surface.mark_needs_render();
                        }
                    }
                    WidgetEvent::TouchUp { .. } => {
                        if let Some(rs) = render.as_mut() {
                            use bmc_render::interaction::TouchEvent;
                            rs.runtime.push_touch_event(TouchEvent::Up);
                            self.surface.mark_needs_render();
                        }
                    }
                    WidgetEvent::TouchCancel => {
                        if let Some(rs) = render.as_mut() {
                            use bmc_render::interaction::TouchEvent;
                            rs.runtime.push_touch_event(TouchEvent::Cancel);
                            self.surface.mark_needs_render();
                        }
                    }
                }
            }

            // --- Lazy init: create EGL + WASM runtime ---
            if render.is_none() {
                tracing::info!("Initializing GPU resources");

                let wasm_bytes = std::fs::read(wasm_path).with_context(|| {
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

                // Seed clock so animations and frame deadlines tick from t=0
                let monotonic_origin = Instant::now();
                runtime.set_time(chrono::Local::now().fixed_offset(), 0);

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
                    monotonic_origin,
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

                rs.runtime
                    .set_time(chrono::Local::now().fixed_offset(), rs.monotonic_ms(now));

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

                // Deliver pending async I/O to the WASM widget
                rs.runtime.deliver_fetch_responses();
                rs.runtime.deliver_ws_messages();
                rs.runtime.deliver_socket_events();
                rs.runtime.deliver_mdns_events();
                rs.runtime.deliver_ssdp_events();
                rs.runtime.deliver_udp_broadcast_events();
                rs.runtime.deliver_http_requests();

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
            } else if has_async_io(&rs.runtime) {
                100
            } else {
                -1
            }
        } else {
            0 // not yet initialized — init immediately
        }
    }
}

/// Returns true if any background async I/O is in flight that may produce
/// events the widget needs delivered.
fn has_async_io(runtime: &WasmWidgetRuntime) -> bool {
    runtime.has_pending_fetches()
        || runtime.has_active_websockets()
        || runtime.has_active_sockets()
        || runtime.has_active_mdns_browses()
        || runtime.has_active_ssdp_searches()
        || runtime.has_active_udp_broadcasts()
        || runtime.has_active_http_listeners()
}

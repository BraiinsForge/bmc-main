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
use bmc_wasm_runtime::{LedEffect, LedRequest, RenderStatus, RuntimeConfig, WasmWidgetRuntime};
use bmc_widget::surface::{DeckWidgetSurfaceClient, WidgetEvent, WidgetSurface};
use bmc_widget_protocol::{ActionPayload, LedEffect as ProtoEffect, RgbColor};
use std::path::Path;
use std::sync::mpsc;
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
    led_rx: mpsc::Receiver<LedRequest>,
    /// Moved into the runtime on first init; `None` afterwards.
    led_tx: Option<mpsc::Sender<LedRequest>>,
}

impl WaylandClient {
    /// Connect to the Wayland display and read the initial configure batch.
    pub fn connect() -> Result<Self> {
        let (surface, initial) = DeckWidgetSurfaceClient::connect()?;

        tracing::info!("Widget config: {}x{}", initial.width, initial.height);

        let (led_tx, led_rx) = mpsc::channel();
        Ok(Self {
            surface,
            led_rx,
            led_tx: Some(led_tx),
        })
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
            self.flush_led_requests()?;

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
                    WidgetEvent::ParamUpdate(_) => {
                        tracing::debug!("wasm: ignoring runtime params update");
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
                let runtime_config = RuntimeConfig {
                    led_request_sender: self.led_tx.take(),
                    ..RuntimeConfig::default()
                };
                let mut runtime = unsafe {
                    WasmWidgetRuntime::new(
                        &wasm_bytes,
                        EglState::get_proc_address,
                        w,
                        h,
                        fbo_id,
                        runtime_config,
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

    /// Drain LED requests from the runtime and forward each one as a
    /// `deck_widget_v1` action.
    fn flush_led_requests(&mut self) -> Result<()> {
        while let Ok(req) = self.led_rx.try_recv() {
            let action = led_request_to_action(&req);
            self.surface.request_action(&action)?;
        }
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

/// Map a `LedRequest` from the wasm runtime to the `deck_widget_v1`
/// action shape used by `DeckWidgetSurfaceClient::request_action`.
///
/// Six-variant `LedEffect` (runtime side) and `bmc_widget_protocol::
/// LedEffect` are both protocol-aligned, so the effect transform is
/// an exhaustive variant identity. Duration is `Option<Duration>` on
/// the runtime side, which picks `LedEndless` (None) vs `LedTemporary`
/// (Some). `LedRequest::Stop` carries the widget's request id; the
/// runtime emits `0` for stop-all and any non-zero for targeted, but
/// the current SDK only emits stop-all.
fn led_request_to_action(req: &LedRequest) -> ActionPayload {
    match req {
        LedRequest::SetEffect {
            request_id,
            effect,
            color,
            period_ms,
            duration,
        } => {
            let effect = match effect {
                LedEffect::Chase => ProtoEffect::Chase,
                LedEffect::KnightRider => ProtoEffect::KnightRider,
                LedEffect::Scan => ProtoEffect::Scan,
                LedEffect::Snake => ProtoEffect::Snake,
                LedEffect::Breathe => ProtoEffect::Breathe,
                LedEffect::Solid => ProtoEffect::Solid,
            };
            let color = RgbColor {
                r: color.r,
                g: color.g,
                b: color.b,
            };
            match duration {
                None => ActionPayload::LedEndless {
                    request_id: *request_id,
                    effect,
                    color,
                    period_ms: *period_ms,
                },
                Some(d) => ActionPayload::LedTemporary {
                    request_id: *request_id,
                    effect,
                    color,
                    period_ms: *period_ms,
                    duration_ms: u32::try_from(d.as_millis()).unwrap_or(u32::MAX),
                },
            }
        }
        LedRequest::Stop { request_id } => ActionPayload::StopLed {
            request_id: *request_id,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::led_request_to_action;
    use bmc_wasm_runtime::{LedEffect, LedRequest, Rgb};
    use bmc_widget_protocol::{ActionPayload, LedEffect as ProtoEffect, RgbColor};
    use std::time::Duration;

    fn red() -> Rgb {
        Rgb::new(255, 0, 0)
    }

    #[test]
    fn endless_maps_to_led_endless() {
        let req = LedRequest::SetEffect {
            request_id: 7,
            effect: LedEffect::Breathe,
            color: red(),
            period_ms: 750,
            duration: None,
        };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::LedEndless {
                request_id: 7,
                effect: ProtoEffect::Breathe,
                color: RgbColor { r: 255, g: 0, b: 0 },
                period_ms: 750,
            }
        );
    }

    #[test]
    fn temporary_maps_to_led_temporary() {
        let req = LedRequest::SetEffect {
            request_id: 9,
            effect: LedEffect::Solid,
            color: red(),
            period_ms: 0,
            duration: Some(Duration::from_millis(5_000)),
        };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::LedTemporary {
                request_id: 9,
                effect: ProtoEffect::Solid,
                color: RgbColor { r: 255, g: 0, b: 0 },
                period_ms: 0,
                duration_ms: 5_000,
            }
        );
    }

    #[test]
    fn temporary_zero_duration_is_preserved() {
        let req = LedRequest::SetEffect {
            request_id: 1,
            effect: LedEffect::Snake,
            color: red(),
            period_ms: 0,
            duration: Some(Duration::ZERO),
        };
        let action = led_request_to_action(&req);
        let ActionPayload::LedTemporary { duration_ms, .. } = action else {
            panic!("BUG: temporary mapping must produce LedTemporary");
        };
        assert_eq!(duration_ms, 0);
    }

    #[test]
    fn stop_maps_to_stop_led() {
        let req = LedRequest::Stop { request_id: 0 };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::StopLed { request_id: 0 }
        );
    }
}

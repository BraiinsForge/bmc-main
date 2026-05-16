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
    /// Initial params from the configure batch, kept until the runtime is constructed.
    /// Applied to the runtime via `set_params` immediately after first init.
    initial_params: serde_json::Map<String, serde_json::Value>,
}

impl WaylandClient {
    /// Connect to the Wayland display and read the initial configure batch.
    pub fn connect() -> Result<Self> {
        let (surface, initial) = DeckWidgetSurfaceClient::connect()?;

        tracing::info!(
            "Widget config: {}x{}, {} params key(s)",
            initial.width,
            initial.height,
            initial.params.len(),
        );

        Ok(Self {
            surface,
            initial_params: initial.params,
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
                    WidgetEvent::ParamUpdate(params) => {
                        if let Some(rs) = render.as_mut() {
                            // Runtime is up — parse + deliver. `parse_params_json` rejects
                            // the entire update on the first invalid entry; on Err we keep
                            // the runtime's previous snapshot rather than apply a partial map.
                            // A failure here means the compositor sent something off-spec — warn loudly.
                            match bmc_wasm_runtime::parse_params_json(&params) {
                                Ok(table) => {
                                    rs.runtime.deliver_params_update(table);
                                    self.surface.mark_needs_render();
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        "dropping params update — keeping previous snapshot: {err}"
                                    );
                                }
                            }
                        } else {
                            // Runtime not yet constructed (first ParamUpdate arrived in the same
                            // drain that precedes lazy init in this loop iteration).
                            //
                            // Buffer the keys into `initial_params` so `RuntimeConfig::params`
                            // at construction sees the latest delivered values
                            // rather than silently dropping the update.
                            merge_into_initial_params(&mut self.initial_params, params);
                            tracing::debug!(
                                "buffered param update until runtime init; \
                                 initial_params now has {} key(s)",
                                self.initial_params.len(),
                            );
                        }
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
                    WidgetEvent::Lifecycle(state) => {
                        tracing::trace!(?state, "wasm: ignoring lifecycle event");
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
                // Parse the compositor-delivered params and pass them
                // via `RuntimeConfig` so they're staged on `HostState`
                // before `init` runs — the widget's first  `params::current()`
                // call (inside `init` or its first `render`) observes
                // operator-configured values, not an empty map.
                //
                // On a parse failure (off-spec compositor) we fall back
                // to an empty map rather than refuse to start: the widget
                // still renders with its manifest defaults, which is
                // a better UX than bricking startup on an upstream bug.
                let params = bmc_wasm_runtime::parse_params_json(&self.initial_params)
                    .unwrap_or_else(|err| {
                        tracing::warn!("invalid initial params — starting with empty map: {err}");
                        std::collections::BTreeMap::new()
                    });
                tracing::info!("Applying {} params key(s) to runtime", params.len());
                let mut runtime = unsafe {
                    WasmWidgetRuntime::new(
                        &wasm_bytes,
                        EglState::get_proc_address,
                        w,
                        h,
                        fbo_id,
                        RuntimeConfig {
                            params,
                            ..RuntimeConfig::default()
                        },
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

/// Returns true if any background async I/O is in flight
/// that may produce events the widget needs delivered.
fn has_async_io(runtime: &WasmWidgetRuntime) -> bool {
    runtime.has_pending_fetches()
        || runtime.has_active_websockets()
        || runtime.has_active_sockets()
        || runtime.has_active_mdns_browses()
        || runtime.has_active_ssdp_searches()
        || runtime.has_active_udp_broadcasts()
        || runtime.has_active_http_listeners()
}

/// Merge an incoming params map into the buffered initial-params snapshot.
///
/// Keys in `incoming` overwrite same-named keys in `initial`;
/// keys present only in `initial` stay.
///
/// Used when a `ParamUpdate` arrives before the runtime is constructed
/// The buffered state is what gets handed to `RuntimeConfig::params`
/// at lazy init.
fn merge_into_initial_params(
    initial: &mut serde_json::Map<String, serde_json::Value>,
    incoming: serde_json::Map<String, serde_json::Value>,
) {
    initial.extend(incoming);
}

#[cfg(test)]
mod tests {
    use super::merge_into_initial_params;
    use serde_json::{Value, json};

    fn map_of(pairs: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn merge_into_empty_initial_keeps_all_incoming_keys() {
        let mut initial = serde_json::Map::new();
        let incoming = map_of(&[("a", json!(1)), ("b", json!("two"))]);
        merge_into_initial_params(&mut initial, incoming);
        assert_eq!(initial.get("a"), Some(&json!(1)));
        assert_eq!(initial.get("b"), Some(&json!("two")));
        assert_eq!(initial.len(), 2);
    }

    #[test]
    fn merge_overwrites_existing_keys_with_latest_value() {
        let mut initial = map_of(&[("a", json!(1)), ("b", json!("two"))]);
        let incoming = map_of(&[("a", json!(99))]);
        merge_into_initial_params(&mut initial, incoming);
        assert_eq!(initial.get("a"), Some(&json!(99)));
        assert_eq!(initial.get("b"), Some(&json!("two")));
    }

    #[test]
    fn merge_preserves_keys_not_present_in_incoming() {
        let mut initial = map_of(&[("a", json!(1)), ("b", json!(2))]);
        let incoming = map_of(&[("c", json!(3))]);
        merge_into_initial_params(&mut initial, incoming);
        assert_eq!(initial.get("a"), Some(&json!(1)));
        assert_eq!(initial.get("b"), Some(&json!(2)));
        assert_eq!(initial.get("c"), Some(&json!(3)));
    }
}

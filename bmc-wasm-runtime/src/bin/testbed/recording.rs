// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Recording-mode state and UI: gesture tracking, the event-log panel,
//! and the `finish_recording` path that merges user / network / fetch event
//! sources into a fixture on disk plus updates the widget's capture config.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::items_after_statements,
    reason = "wall-clock ms / pixel deltas on positive bounded ranges, \
              inline ui-block constants placed next to where they're used"
)]

use std::path::PathBuf;

use bmc_wasm_runtime::fixtures;
use bmc_wasm_runtime::unified_fixture::{
    FixtureHeader, TimelineEvent, UnifiedEvent, UnifiedFixture,
};

use super::TestbedApp;

// ── Recording state ─────────────────────────────────────────────────

/// Tracks an in-progress touch gesture for recording mode.
pub(super) struct GestureTracker {
    pub(super) start_pos: (f32, f32),
    pub(super) current_pos: (f32, f32),
    pub(super) start_element: Option<String>,
}

/// Delay between a user action and its auto-inserted capture event (ms).
pub(super) const AUTO_CAPTURE_DELAY_MS: u64 = 500;

/// Append a delivery event to the timeline at `recording_start.elapsed()`
/// and, when `auto_capture` is on, attach a debounced auto-`Capture`
/// 500 ms later so each settled state yields one baseline frame.
///
/// # Debounce
///
/// Slider drags, text typing, and rapid system mutations produce
/// one delivery per intermediate value (dozens per second).
///
/// A naive "Capture 500 ms after every delivery" rule would mint hundreds
/// of nearly-identical frames. Instead, when a new delivery arrives
/// and the most-recently-pushed auto-Capture's `at_ms` is still
/// in the future relative to the new delivery, slide that Capture
/// forward to `at_ms + 500` rather than appending another.
///
/// Net effect: one Capture per cluster of changes ≤500 ms apart,
/// fired 500 ms after the cluster's last delivery.
///
/// The scan-backwards approach matches only `Capture { duration_ms: None,
/// fps: None }` so gesture-path animation Captures (with concrete duration)
/// never get slid forward by a param / system delivery.
pub(super) fn record_delivery(rec: &mut RecordingState, make_event: impl FnOnce() -> UnifiedEvent) {
    let at_ms = rec.recording_start.elapsed().as_millis() as u64;
    rec.events.push(TimelineEvent {
        at_ms,
        event: make_event(),
    });
    if !rec.auto_capture {
        return;
    }
    let capture_at = at_ms + AUTO_CAPTURE_DELAY_MS;
    let pending = rec
        .events
        .iter_mut()
        .rev()
        .find(|e| {
            matches!(
                e.event,
                UnifiedEvent::Capture {
                    duration_ms: None,
                    fps: None,
                }
            )
        })
        .filter(|e| e.at_ms > at_ms);
    if let Some(prev_capture) = pending {
        prev_capture.at_ms = capture_at;
    } else {
        rec.events.push(TimelineEvent {
            at_ms: capture_at,
            event: UnifiedEvent::Capture {
                duration_ms: None,
                fps: None,
            },
        });
    }
}
/// Pixel threshold separating "click" from "drag" / "scroll" gestures.
const GESTURE_THRESHOLD: f32 = 5.0;

/// Recording-mode state. Owned by the `TestbedApp` while a recording is active; replaced with
/// `None` on save/cancel. The recording UI panel reads this to render its event log.
pub(super) struct RecordingState {
    pub(super) active_tile: usize,
    /// The (platform, viewport) being recorded,
    /// and the dataset name its fixture and config entry are written under.
    pub(super) target: bmc_wasm_runtime::platform_catalog::Target,
    pub(super) dataset: String,
    /// Unified timeline events (user actions + fetch recordings).
    pub(super) events: Vec<TimelineEvent>,
    pub(super) gesture: Option<GestureTracker>,
    /// Widget root directory (for output paths).
    pub(super) widget_root: Option<PathBuf>,
    /// Wall-clock reference for `at_ms` calculation.
    pub(super) recording_start: std::time::Instant,
    /// Snapshot of KV dir state at recording start.
    pub(super) kv_snapshot: std::collections::HashMap<String, String>,
    /// Snapshot of params at recording start (manifest defaults plus any operator
    /// changes made BEFORE the record button was hit). Pre-encoded as the JSON
    /// shape `FixtureHeader::initial_params` expects so replay can read them
    /// without parsing the manifest; the host installs these as the runtime's
    /// initial `RuntimeConfig::params`, so the first `ParamDelivery` event
    /// in `events` diffs against them rather than against an empty snapshot.
    pub(super) params_snapshot: serde_json::Map<String, serde_json::Value>,
    /// Snapshot of the deck-wide system state at recording start.
    /// Same role as [`Self::params_snapshot`] but for the `system` channel
    /// — installed into `RuntimeConfig::system` by replay so the first
    /// `SystemDelivery` event diffs against the actual starting state
    /// rather than the `SystemSnapshot::default()` fallback.
    pub(super) system_snapshot: bmc_wasm_runtime::SystemSnapshot,
    /// Bound credential slots at recording start.
    /// Same role [`Self::params_snapshot`] plays for the params channel.
    pub(super) credentials_snapshot: serde_json::Map<String, serde_json::Value>,
    /// Start time (ISO 8601) captured at recording start.
    pub(super) start_time_iso: String,
    /// When true, a Capture event is auto-inserted after each user action.
    pub(super) auto_capture: bool,
}

/// Map a record-size name to its tile index, or `None` for an unknown name.
/// Unknown names are rejected loudly by `validate_recording_target` rather
/// than silently defaulting to the full tile.
pub(super) fn record_size_to_idx(s: &str) -> Option<usize> {
    match s {
        "full" => Some(0),
        "large" => Some(1),
        "medium" => Some(2),
        "small" => Some(3),
        _ => None,
    }
}

/// Short label for the event log (the icon already carries the type info).
fn format_event_label(event: &UnifiedEvent) -> String {
    match event {
        UnifiedEvent::Capture { duration_ms, fps } => match (duration_ms, fps) {
            (Some(d), Some(f)) => format!("capture({d}ms, {f}fps)"),
            (Some(d), None) => format!("capture({d}ms)"),
            _ => "capture".to_owned(),
        },
        UnifiedEvent::Click { element } => format!("click #{element}"),
        UnifiedEvent::Scroll { element, delta } => format!("scroll #{element}  Δ{delta}"),
        UnifiedEvent::Drag { element, from, to } => {
            format!("drag #{element}  {from:.0}→{to:.0}")
        }
        UnifiedEvent::ParamDelivery { params } => format!("params Δ{} key(s)", params.len()),
        UnifiedEvent::SystemDelivery { .. } => "system Δ".to_owned(),
        UnifiedEvent::CredentialDelivery { credentials } => {
            format!("credentials Δ{} slot(s)", credentials.len())
        }
        UnifiedEvent::Fetch {
            method,
            url,
            status,
            ..
        } => format!("{method} {status} {url}"),
        UnifiedEvent::WsOpen { ws_id } | UnifiedEvent::WsMessage { ws_id, .. } => {
            format!("ws#{ws_id}")
        }
        UnifiedEvent::WsClose { ws_id, code } => format!("ws#{ws_id} code={code}"),
        UnifiedEvent::SocketConnected { socket_id }
        | UnifiedEvent::SocketData { socket_id, .. } => format!("tcp#{socket_id}"),
        UnifiedEvent::SocketClosed { socket_id, code } => format!("tcp#{socket_id} code={code}"),
        UnifiedEvent::SsdpFound { search_id, .. } | UnifiedEvent::SsdpRemoved { search_id, .. } => {
            format!("ssdp#{search_id}")
        }
        UnifiedEvent::MdnsFound { browse_id, .. } | UnifiedEvent::MdnsRemoved { browse_id, .. } => {
            format!("mdns#{browse_id}")
        }
        UnifiedEvent::UdpResponse {
            broadcast_id,
            source,
            ..
        } => format!("udp#{broadcast_id} ← {source}"),
        UnifiedEvent::AudioPlay {
            name,
            volume,
            duration_ms,
            ..
        } => format!("audio {name} vol={volume} {duration_ms}ms"),
        UnifiedEvent::LedSetEndless {
            effect,
            r,
            g,
            b,
            period_ms,
            scope,
        } => format!("LED endless effect={effect} rgb=({r},{g},{b}) p={period_ms}ms scope={scope}"),
        UnifiedEvent::LedSetTemporary {
            effect,
            r,
            g,
            b,
            period_ms,
            duration_ms,
            scope,
        } => format!(
            "LED temporary effect={effect} rgb=({r},{g},{b}) p={period_ms}ms d={duration_ms}ms scope={scope}"
        ),
        UnifiedEvent::LedStop => "LED stop".to_owned(),
    }
}

/// Classify a finished gesture into a `UnifiedEvent` and append it to `rec.events`.
/// Auto-inserts a `Capture` event 500ms later when `auto_capture` is on.
pub(super) fn classify_and_record_gesture(rec: &mut RecordingState, gesture: &GestureTracker) {
    let dx = gesture.current_pos.0 - gesture.start_pos.0;
    let dy = gesture.current_pos.1 - gesture.start_pos.1;
    let adx = dx.abs();
    let ady = dy.abs();
    let at_ms = rec.recording_start.elapsed().as_millis() as u64;

    let Some(ref id) = gesture.start_element else {
        if adx < GESTURE_THRESHOLD && ady < GESTURE_THRESHOLD {
            eprintln!("Recording: click on empty area (no element ID)");
        }
        return;
    };

    let event = if adx < GESTURE_THRESHOLD && ady < GESTURE_THRESHOLD {
        eprintln!("Recording: click(#{id})");
        UnifiedEvent::Click {
            element: id.clone(),
        }
    } else if ady >= GESTURE_THRESHOLD && ady > adx {
        let delta = dy.round() as i32;
        eprintln!("Recording: scroll(#{id}, {delta})");
        UnifiedEvent::Scroll {
            element: id.clone(),
            delta,
        }
    } else if adx >= GESTURE_THRESHOLD && adx > ady {
        eprintln!(
            "Recording: drag(#{id}, {:.2}, {:.2})",
            gesture.start_pos.0, gesture.current_pos.0
        );
        UnifiedEvent::Drag {
            element: id.clone(),
            from: gesture.start_pos.0,
            to: gesture.current_pos.0,
        }
    } else {
        return;
    };

    rec.events.push(TimelineEvent { at_ms, event });

    if rec.auto_capture {
        let capture_at = at_ms + AUTO_CAPTURE_DELAY_MS;
        rec.events.push(TimelineEvent {
            at_ms: capture_at,
            event: UnifiedEvent::Capture {
                duration_ms: Some(2_000),
                fps: Some(4),
            },
        });
        eprintln!("Recording: auto-capture at {capture_at}ms");
    }
}

// ── Recording panel / finish ────────────────────────────────────────

/// Action dispatched by the recording panel each frame.
/// `None` between frames; one of the variants on the frame the operator clicks.
#[derive(Clone, Copy, Debug)]
pub(super) enum RecordingAction {
    Save,
    Cancel,
    Capture,
}

impl TestbedApp {
    /// Paint the recording panel in `rect` — title, scrollable event log, Save/Cancel/Capture
    /// buttons, and the auto-capture toggle. Returns the operator action for this frame.
    pub(super) fn paint_recording_panel(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
    ) -> Option<RecordingAction> {
        let rec = self.recording_mode.state.as_mut()?;
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_gray(18));
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(180, 80, 20)),
            egui::StrokeKind::Inside,
        );

        let pad = 8.0;
        let inner = rect.shrink(pad);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let mut action: Option<RecordingAction> = None;

        child.label(
            egui::RichText::new(format!("RECORDING — {}", rec.target))
                .color(egui::Color32::from_rgb(255, 170, 80))
                .strong(),
        );
        child.separator();

        // Bottom button row pinned to inner.max.y; reserve the space first so the scroll
        // area knows how tall it can be.
        const BUTTON_ROW_H: f32 = 28.0;
        let log_max_y = inner.max.y - BUTTON_ROW_H - 6.0;
        let log_rect = egui::Rect::from_min_max(
            egui::pos2(inner.min.x, child.cursor().min.y),
            egui::pos2(inner.max.x, log_max_y),
        );
        if log_rect.height() > 16.0 {
            let mut log_child = child.new_child(egui::UiBuilder::new().max_rect(log_rect));
            let mono = egui::FontId::monospace(10.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .max_height(log_rect.height())
                .show(&mut log_child, |scroll| {
                    if rec.events.is_empty() {
                        scroll.label(
                            egui::RichText::new("(no events yet — click / drag a tile)")
                                .color(egui::Color32::from_gray(120))
                                .font(mono.clone()),
                        );
                    } else {
                        for ev in &rec.events {
                            let secs = ev.at_ms as f32 / 1000.0;
                            let line = format!("{secs:>6.2}s  {}", format_event_label(&ev.event));
                            scroll.label(egui::RichText::new(line).font(mono.clone()));
                        }
                    }
                });
        }

        // Button row at the bottom edge.
        let row_rect = egui::Rect::from_min_max(
            egui::pos2(inner.min.x, inner.max.y - BUTTON_ROW_H),
            inner.max,
        );
        let mut row_child = child.new_child(egui::UiBuilder::new().max_rect(row_rect));
        row_child.horizontal(|row| {
            if row.button("Save").clicked() {
                action = Some(RecordingAction::Save);
            }
            if row.button("Cancel").clicked() {
                action = Some(RecordingAction::Cancel);
            }
            if row.button("Capture").clicked() {
                action = Some(RecordingAction::Capture);
            }
            row.checkbox(&mut rec.auto_capture, "auto");
        });

        action
    }

    /// Append a manual single-frame `Capture` event to the recording timeline.
    pub(super) fn push_manual_capture(&mut self) {
        if let Some(rec) = self.recording_mode.state.as_mut() {
            let at_ms = rec.recording_start.elapsed().as_millis() as u64;
            rec.events.push(TimelineEvent {
                at_ms,
                event: UnifiedEvent::Capture {
                    duration_ms: None,
                    fps: None,
                },
            });
        }
    }

    /// Take ownership of the active recording, merge all event sources
    /// (user actions, network events from the runtime, fetch events from the shared buffer),
    /// validate, and write a `.jsonl.gz` fixture into the widget's `capture/fixtures/<size>.jsonl.gz`.
    /// Also updates the widget's `capture/config.toml` to point at the new fixture.
    pub(super) fn finish_recording(&mut self) {
        let Some(rec) = self.recording_mode.state.take() else {
            return;
        };

        // Pull network events out of the active tile's runtime, plus the fetch events the
        // observer pushed into the shared buffer.
        let runtime_events = match self
            .tiles
            .get_mut(rec.active_tile)
            .and_then(|tile| tile.runtime.as_mut())
        {
            Some(runtime) => runtime.take_recorded_events(),
            None => Vec::new(),
        };
        let network_timeline = fixtures::fixture_events_to_timeline(&runtime_events);
        let fetch_timeline: Vec<TimelineEvent> = std::mem::take(
            &mut *self
                .recording_mode
                .fetch_events
                .lock()
                .expect("BUG: fetch events poisoned"),
        );

        // Merge: user actions + network + fetch, sorted by at_ms (stable so insertion order
        // breaks ties), then collapse consecutive scrolls on the same element into one event.
        let mut all_events = rec.events;
        all_events.extend(network_timeline);
        all_events.extend(fetch_timeline);
        all_events.sort_by_key(|e| e.at_ms);
        let mut merged: Vec<TimelineEvent> = Vec::with_capacity(all_events.len());
        for event in all_events {
            let should_merge = if let UnifiedEvent::Scroll { ref element, .. } = event.event {
                merged.last().is_some_and(|prev: &TimelineEvent| {
                    matches!(&prev.event, UnifiedEvent::Scroll { element: prev_el, .. } if prev_el == element)
                })
            } else {
                false
            };
            if should_merge {
                if let UnifiedEvent::Scroll { delta, .. } = event.event
                    && let Some(prev) = merged.last_mut()
                    && let UnifiedEvent::Scroll {
                        delta: ref mut prev_delta,
                        ..
                    } = prev.event
                {
                    *prev_delta += delta;
                }
            } else {
                merged.push(event);
            }
        }

        let fixture = UnifiedFixture {
            header: FixtureHeader {
                time: rec.start_time_iso,
                kv: rec.kv_snapshot,
                initial_params: rec.params_snapshot,
                initial_system: rec.system_snapshot,
                initial_credentials: rec.credentials_snapshot,
            },
            events: merged,
        };

        let Some(widget_root) = rec.widget_root else {
            eprintln!(
                "error: could not find widget root — fixture not saved ({} event(s))",
                fixture.events.len()
            );
            return;
        };
        let fixture_dir = widget_root.join("capture").join("fixtures");
        let fixture_path = fixture_dir.join(format!("{}.jsonl.gz", rec.dataset));

        if let Err(e) = bmc_wasm_runtime::unified_fixture::validate_fixture(&fixture) {
            eprintln!("warning: fixture validation failed: {e:#} (writing anyway)");
        }
        if let Err(e) = fixtures::write_jsonl_fixture(&fixture_path, &fixture) {
            eprintln!("error: failed to write fixture: {e:#}");
            return;
        }
        eprintln!(
            "wrote: {} event(s) → {}",
            fixture.events.len(),
            fixture_path.display()
        );

        let config_path = widget_root.join("capture").join("config.toml");
        let fixture_rel = format!("fixtures/{}.jsonl.gz", rec.dataset);
        if let Err(e) = fixtures::update_config_toml_fixtures(
            &config_path,
            &rec.dataset,
            &fixture_rel,
            rec.target,
        ) {
            eprintln!("warning: failed to update config.toml: {e:#}");
        } else {
            eprintln!("updated: {}", config_path.display());
        }

        let widget_name = widget_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("WIDGET");
        eprintln!("hint: run `just wasm::update-baselines {widget_name}` to set baselines");
    }
}

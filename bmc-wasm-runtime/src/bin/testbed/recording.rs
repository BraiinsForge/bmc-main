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
    reason = "wall-clock ms / pixel deltas on positive bounded ranges"
)]

use std::path::PathBuf;

use bmc_wasm_runtime::fixtures;
use bmc_wasm_runtime::unified_fixture::{
    FixtureHeader, TimelineEvent, UnifiedEvent, UnifiedFixture,
};

use super::TestbedApp;
use super::view::DeviceView;

// ── Recording state ─────────────────────────────────────────────────

/// Tracks an in-progress touch gesture for recording mode.
struct GestureTracker {
    start_pos: (f32, f32),
    current_pos: (f32, f32),
    start_element: Option<String>,
}

/// Delay between a user action and its auto-inserted capture event (ms).
const AUTO_CAPTURE_DELAY_MS: u64 = 500;

impl RecordingState {
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
    fn record_delivery(&mut self, make_event: impl FnOnce() -> UnifiedEvent) {
        let at_ms = self.recording_start.elapsed().as_millis() as u64;
        self.events.push(TimelineEvent {
            at_ms,
            event: make_event(),
        });
        if !self.auto_capture {
            return;
        }
        let capture_at = at_ms + AUTO_CAPTURE_DELAY_MS;
        let pending = self
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
            self.events.push(TimelineEvent {
                at_ms: capture_at,
                event: UnifiedEvent::Capture {
                    duration_ms: None,
                    fps: None,
                },
            });
        }
    }
}
/// Pixel threshold separating "click" from "drag" / "scroll" gestures.
const GESTURE_THRESHOLD: f32 = 5.0;

/// One running take. Fields are module-private: outside callers read through
/// the getters and mutate through the ops, so the timeline only grows through
/// the ops that stamp it, no matter who holds the handle.
pub(super) struct RecordingState {
    active_tile: usize,
    /// The (platform, viewport) being recorded,
    /// and the dataset name its fixture and config entry are written under.
    target: bmc_wasm_runtime::platform_catalog::Target,
    dataset: String,
    /// Unified timeline events (user actions + fetch recordings).
    events: Vec<TimelineEvent>,
    gesture: Option<GestureTracker>,
    /// Widget root directory (for output paths).
    widget_root: Option<PathBuf>,
    /// Wall-clock reference for `at_ms` calculation.
    recording_start: std::time::Instant,
    /// Snapshot of KV dir state at recording start.
    kv_snapshot: std::collections::HashMap<String, String>,
    /// Snapshot of params at recording start (manifest defaults plus any operator
    /// changes made BEFORE the record button was hit). Pre-encoded as the JSON
    /// shape `FixtureHeader::initial_params` expects so replay can read them
    /// without parsing the manifest; the host installs these as the runtime's
    /// initial `RuntimeConfig::params`, so the first `ParamDelivery` event
    /// in `events` diffs against them rather than against an empty snapshot.
    params_snapshot: serde_json::Map<String, serde_json::Value>,
    /// Snapshot of the deck-wide system state at recording start.
    /// Same role as [`Self::params_snapshot`] but for the `system` channel
    /// — installed into `RuntimeConfig::system` by replay so the first
    /// `SystemDelivery` event diffs against the actual starting state
    /// rather than the `SystemSnapshot::default()` fallback.
    system_snapshot: bmc_wasm_runtime::SystemSnapshot,
    /// Bound credential slots at recording start.
    /// Same role [`Self::params_snapshot`] plays for the params channel.
    credentials_snapshot: serde_json::Map<String, serde_json::Value>,
    /// Start time (ISO 8601) captured at recording start.
    start_time_iso: String,
    /// When true, a Capture event is auto-inserted after each user action.
    auto_capture: bool,
}

impl RecordingState {
    pub(super) fn target(&self) -> bmc_wasm_runtime::platform_catalog::Target {
        self.target
    }

    /// Index of the recorded viewport, on its platform and — recording pins
    /// one platform open — in `tiles` alike.
    pub(super) fn active_tile(&self) -> usize {
        self.active_tile
    }

    pub(super) fn dataset(&self) -> &str {
        &self.dataset
    }

    /// Whether the take holds anything worth saving yet.
    pub(super) fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// The wiped-and-seeded KV state the fixture header reproduces on replay.
    /// `build_views` reports it once the take's KV dir exists.
    pub(super) fn set_kv_baseline(&mut self, kv: std::collections::HashMap<String, String>) {
        self.kv_snapshot = kv;
    }

    /// A quick click: a zero-distance gesture, classified immediately —
    /// `drag_started` never fires for it.
    pub(super) fn record_tap(&mut self, pos: (f32, f32), start_element: Option<String>) {
        let gesture = GestureTracker {
            start_pos: pos,
            current_pos: pos,
            start_element,
        };
        self.classify_gesture(&gesture);
    }

    /// A drag began on `start_element`; positions accumulate until the release.
    pub(super) fn begin_gesture(&mut self, pos: (f32, f32), start_element: Option<String>) {
        self.gesture = Some(GestureTracker {
            start_pos: pos,
            current_pos: pos,
            start_element,
        });
    }

    pub(super) fn update_gesture(&mut self, pos: (f32, f32)) {
        if let Some(gesture) = self.gesture.as_mut() {
            gesture.current_pos = pos;
        }
    }

    /// The pointer released: classify what the gesture became and log it.
    pub(super) fn finish_gesture(&mut self) {
        if let Some(gesture) = self.gesture.take() {
            self.classify_gesture(&gesture);
        }
    }

    /// Append a manual single-frame `Capture` — the operator committing the
    /// current widget state as a baseline frame.
    pub(super) fn record_capture(&mut self) {
        let at_ms = self.recording_start.elapsed().as_millis() as u64;
        self.events.push(TimelineEvent {
            at_ms,
            event: UnifiedEvent::Capture {
                duration_ms: None,
                fps: None,
            },
        });
    }
}

/// The record mode: off, choosing a target, or mid-take.
///
/// One machine rather than scattered fields, so a phase cannot exist without
/// what its exit must put back — the canvas is saved exactly while the mode
/// is engaged, and the KV stash exactly while a take runs.
pub(crate) struct RecordingMode {
    phase: RecordPhase,
    /// Shared buffer for fetch events captured by the active view's fetch
    /// observer. Outside the phase, because the observer holds an `Arc` clone
    /// across view rebuilds; cleared at each phase boundary instead.
    fetch_events: std::sync::Arc<std::sync::Mutex<Vec<TimelineEvent>>>,
}

enum RecordPhase {
    Off,
    /// Overlays up over every candidate viewport; no take yet.
    Choosing {
        /// The datasets each already-recorded target carries in the widget's
        /// capture config, by canonical target string. The overlays badge
        /// these, since Save overwrites.
        recorded: std::collections::HashMap<String, Vec<String>>,
        /// A recorded target clicked once: the overlay asks again before it
        /// starts a take that will overwrite the fixture.
        confirming: Option<bmc_wasm_runtime::platform_catalog::Target>,
        /// The overlay clicked this frame, consumed at the frame's end —
        /// the click lands inside a borrow of the choosing state.
        chosen: Option<bmc_wasm_runtime::platform_catalog::Target>,
        /// The canvas the mode replaced.
        restore_platforms: Vec<&'static bmc_wasm_runtime::platform_catalog::Platform>,
    },
    Recording {
        /// Boxed: a take dwarfs the other phases, and every transition moves the enum.
        take: Box<RecordingState>,
        restore_platforms: Vec<&'static bmc_wasm_runtime::platform_catalog::Platform>,
        /// The take viewport's KV dir moved aside as `(live, stash)`;
        /// `stash` is `None` when no dir existed.
        kv_stash: (PathBuf, Option<PathBuf>),
    },
}

/// What ending the mode hands back to be undone.
pub(super) enum RecordUnwind {
    /// Choosing backed out: only the canvas changed, and the open views are
    /// still plain — the extras close, nothing rebuilds.
    Choosing {
        restore_platforms: Vec<&'static bmc_wasm_runtime::platform_catalog::Platform>,
    },
    /// A take ended: every view was rebuilt with the recording config, so all
    /// of them retire, and the stashed KV dir goes back.
    Take {
        restore_platforms: Vec<&'static bmc_wasm_runtime::platform_catalog::Platform>,
        kv_stash: (PathBuf, Option<PathBuf>),
    },
}

impl RecordingMode {
    pub(super) fn new() -> Self {
        Self {
            phase: RecordPhase::Off,
            fetch_events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Whether the mode holds the canvas — choosing or mid-take.
    pub(crate) fn engaged(&self) -> bool {
        !matches!(self.phase, RecordPhase::Off)
    }

    pub(super) fn is_choosing(&self) -> bool {
        matches!(self.phase, RecordPhase::Choosing { .. })
    }

    /// The running take, if one is on.
    pub(crate) fn active(&self) -> Option<&RecordingState> {
        match &self.phase {
            RecordPhase::Recording { take, .. } => Some(take),
            RecordPhase::Off | RecordPhase::Choosing { .. } => None,
        }
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut RecordingState> {
        match &mut self.phase {
            RecordPhase::Recording { take, .. } => Some(take),
            RecordPhase::Off | RecordPhase::Choosing { .. } => None,
        }
    }

    /// Off → Choosing. Refused while engaged, so a phase cannot be lost.
    pub(super) fn open_choosing(
        &mut self,
        recorded: std::collections::HashMap<String, Vec<String>>,
        restore_platforms: Vec<&'static bmc_wasm_runtime::platform_catalog::Platform>,
    ) -> bool {
        if self.engaged() {
            return false;
        }
        self.phase = RecordPhase::Choosing {
            recorded,
            confirming: None,
            chosen: None,
            restore_platforms,
        };
        true
    }

    /// The datasets already recorded for `target`, for its choose overlay;
    /// empty when none — or outside the choosing phase, which paints none.
    pub(super) fn recorded_datasets(
        &self,
        target: bmc_wasm_runtime::platform_catalog::Target,
    ) -> &[String] {
        match &self.phase {
            RecordPhase::Choosing { recorded, .. } => {
                recorded.get(&target.to_string()).map_or(&[], Vec::as_slice)
            }
            RecordPhase::Off | RecordPhase::Recording { .. } => &[],
        }
    }

    /// Whether `target`'s overlay is waiting for its overwrite confirmation.
    pub(super) fn is_confirming(&self, target: bmc_wasm_runtime::platform_catalog::Target) -> bool {
        match &self.phase {
            RecordPhase::Choosing { confirming, .. } => confirming.is_some_and(|c| {
                c.platform.id == target.platform.id && c.viewport.id == target.viewport.id
            }),
            RecordPhase::Off | RecordPhase::Recording { .. } => false,
        }
    }

    /// An overlay was clicked. A first click on a recorded target only arms
    /// the confirmation; the choice lands on the second, or immediately for a
    /// target with nothing to overwrite.
    pub(super) fn choose(&mut self, target: bmc_wasm_runtime::platform_catalog::Target) {
        let needs_confirmation = !self.recorded_datasets(target).is_empty();
        if let RecordPhase::Choosing {
            confirming, chosen, ..
        } = &mut self.phase
        {
            if needs_confirmation
                && !confirming.is_some_and(|c| {
                    c.platform.id == target.platform.id && c.viewport.id == target.viewport.id
                })
            {
                *confirming = Some(target);
            } else {
                *chosen = Some(target);
            }
        }
    }

    /// The choice an overlay registered this frame, if any.
    pub(super) fn take_choice(&mut self) -> Option<bmc_wasm_runtime::platform_catalog::Target> {
        match &mut self.phase {
            RecordPhase::Choosing { chosen, .. } => chosen.take(),
            RecordPhase::Off | RecordPhase::Recording { .. } => None,
        }
    }

    /// Off or Choosing → Recording, snapshotting the app's live state into
    /// the take's baseline via [`RecordingState::begin`].
    ///
    /// Carries the canvas saved at choosing through to the take; the CLI path
    /// arrives from Off and saves `current_canvas` instead. The fetch buffer
    /// is cleared here, so an earlier take cannot leak into this fixture.
    /// Refused mid-take, so a running recording cannot be lost.
    #[expect(
        clippy::too_many_arguments,
        reason = "the take's full baseline, taken at once"
    )]
    pub(super) fn begin_take(
        &mut self,
        target: bmc_wasm_runtime::platform_catalog::Target,
        dataset: String,
        widget_root: Option<PathBuf>,
        params: &std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
        system: &bmc_wasm_runtime::SystemSnapshot,
        credentials: &serde_json::Map<String, serde_json::Value>,
        kv_stash: (PathBuf, Option<PathBuf>),
        current_canvas: &[&'static bmc_wasm_runtime::platform_catalog::Platform],
    ) -> bool {
        if self.active().is_some() {
            return false;
        }
        self.fetch_events
            .lock()
            .expect("BUG: fetch-event buffer poisoned")
            .clear();
        let restore_platforms = match std::mem::replace(&mut self.phase, RecordPhase::Off) {
            RecordPhase::Choosing {
                restore_platforms, ..
            } => restore_platforms,
            RecordPhase::Off => current_canvas.to_vec(),
            RecordPhase::Recording { .. } => {
                unreachable!("BUG: refused above while a take runs")
            }
        };
        self.phase = RecordPhase::Recording {
            take: Box::new(RecordingState::begin(
                target,
                dataset,
                widget_root,
                params,
                system,
                credentials,
            )),
            restore_platforms,
            kv_stash,
        };
        true
    }

    /// The take's own timestamp origin, which the fetch observer stamps
    /// `at_ms` against; `None` outside a take.
    pub(super) fn take_epoch(&self) -> Option<std::time::Instant> {
        self.active().map(|rec| rec.recording_start)
    }

    /// A handle on the fetch buffer for a recording view's observer.
    /// The mode keeps ownership: it alone clears and drains the buffer.
    pub(super) fn fetch_buffer(&self) -> std::sync::Arc<std::sync::Mutex<Vec<TimelineEvent>>> {
        std::sync::Arc::clone(&self.fetch_events)
    }

    /// Log a params/system/credentials delivery on the running take;
    /// a no-op outside one.
    pub(super) fn record_delivery(&mut self, make_event: impl FnOnce() -> UnifiedEvent) {
        if let Some(rec) = self.active_mut() {
            rec.record_delivery(make_event);
        }
    }

    /// Report the take's wiped-and-seeded KV baseline; a no-op outside one.
    pub(super) fn set_kv_baseline(&mut self, kv: std::collections::HashMap<String, String>) {
        if let Some(rec) = self.active_mut() {
            rec.set_kv_baseline(kv);
        }
    }

    /// Recording → Off, yielding the take to write and the unwind to apply.
    /// The order matters: the fixture drains the live view, so the caller
    /// writes first and unwinds after.
    pub(super) fn finish(&mut self) -> Option<(RecordingState, RecordUnwind)> {
        match std::mem::replace(&mut self.phase, RecordPhase::Off) {
            RecordPhase::Recording {
                take,
                restore_platforms,
                kv_stash,
            } => Some((
                *take,
                RecordUnwind::Take {
                    restore_platforms,
                    kv_stash,
                },
            )),
            phase @ (RecordPhase::Off | RecordPhase::Choosing { .. }) => {
                self.phase = phase;
                None
            }
        }
    }

    /// Any phase → Off, discarding a running take. The fetch buffer dies with
    /// it, so a cancelled take cannot leak into the next fixture.
    pub(super) fn end(&mut self) -> Option<RecordUnwind> {
        self.fetch_events
            .lock()
            .expect("BUG: fetch-event buffer poisoned")
            .clear();
        match std::mem::replace(&mut self.phase, RecordPhase::Off) {
            RecordPhase::Off => None,
            RecordPhase::Choosing {
                restore_platforms, ..
            } => Some(RecordUnwind::Choosing { restore_platforms }),
            RecordPhase::Recording {
                restore_platforms,
                kv_stash,
                ..
            } => Some(RecordUnwind::Take {
                restore_platforms,
                kv_stash,
            }),
        }
    }
}

impl RecordingState {
    /// Begin a take against the app's live state.
    ///
    /// Shared by startup (`--record`) and the toolbar's runtime entry,
    /// so a fixture header carries the same facts either way.
    /// The snapshots are the take's baseline: replay installs them as the
    /// runtime's starting params, system and credentials, so the first
    /// delivery event diffs against what the operator was looking at.
    fn begin(
        target: bmc_wasm_runtime::platform_catalog::Target,
        dataset: String,
        widget_root: Option<PathBuf>,
        params: &std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
        system: &bmc_wasm_runtime::SystemSnapshot,
        credentials: &serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        let active_tile = target
            .platform
            .viewports
            .iter()
            .position(|v| v.id == target.viewport.id)
            .expect("BUG: a target's viewport must exist on its own platform");
        Self {
            active_tile,
            target,
            dataset,
            events: Vec::new(),
            gesture: None,
            widget_root,
            recording_start: std::time::Instant::now(),
            // Filled by `build_views` once the take's KV dir is wiped and seeded.
            kv_snapshot: std::collections::HashMap::new(),
            // Pre-encoded into the JSON shape `FixtureHeader::initial_params`
            // expects, so the fixture is self-contained: replay never has to
            // locate the widget's `manifest.json` to reconstruct the baseline.
            params_snapshot: params
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
                .collect(),
            system_snapshot: system.clone(),
            credentials_snapshot: credentials.clone(),
            // Capture's fixture-header parser requires a timezone suffix on the
            // time field (e.g. `2026-05-13T15:48:38+02:00`); a naive datetime
            // is rejected.
            start_time_iso: chrono::Local::now().to_rfc3339(),
            auto_capture: true,
        }
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
impl RecordingState {
    fn classify_gesture(&mut self, gesture: &GestureTracker) {
        let dx = gesture.current_pos.0 - gesture.start_pos.0;
        let dy = gesture.current_pos.1 - gesture.start_pos.1;
        let adx = dx.abs();
        let ady = dy.abs();
        let at_ms = self.recording_start.elapsed().as_millis() as u64;

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

        self.events.push(TimelineEvent { at_ms, event });

        if self.auto_capture {
            let capture_at = at_ms + AUTO_CAPTURE_DELAY_MS;
            self.events.push(TimelineEvent {
                at_ms: capture_at,
                event: UnifiedEvent::Capture {
                    duration_ms: Some(2_000),
                    fps: Some(4),
                },
            });
            eprintln!("Recording: auto-capture at {capture_at}ms");
        }
    }
}

// ── Recording panel / finish ────────────────────────────────────────

/// Width of the docked recording sidebar — enough for the log's monospace
/// lines, and the pinned canvas leaves the room to spare.
const RECORDING_PANEL_W: f32 = 320.0;

/// The sidebar's content: the take, its dataset, the Capture control, and the
/// event log filling every remaining pixel. Save and Cancel live in the
/// toolbar, which the mode owns while it runs.
fn paint_recording_panel(
    ui: &mut egui::Ui,
    rec: &mut RecordingState,
    icons: &mut super::icon::Icons,
    palette: &super::theme::Palette,
) {
    let accent = palette.record_accent;

    // The dataset alone: the toolbar chip already names the target. A readout
    // rather than an input, since it follows from the chosen target and a
    // scenario-named take is the CLI's job.
    ui.horizontal(|row| {
        row.add(super::ui_helpers::key_caption("dataset"));
        row.label(
            egui::RichText::new(&rec.dataset)
                .font(egui::FontId::monospace(13.0))
                .color(accent),
        );
    });
    ui.add_space(6.0);

    egui::Frame::NONE
        .fill(palette.section_fill)
        .inner_margin(8.0)
        .corner_radius(4.0)
        .show(ui, |group| {
            group.label("Commit each settled widget state to the take:");
            group.add_space(6.0);
            let mut capture = false;
            group.horizontal(|row| {
                capture = row
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Capture")
                                .color(egui::Color32::WHITE)
                                .strong(),
                        )
                        .fill(accent),
                    )
                    .on_hover_text("commit the current widget state as a baseline frame")
                    .clicked();
                row.checkbox(&mut rec.auto_capture, "after every action")
                    .on_hover_text("append a capture automatically after each recorded action");
            });
            if capture {
                rec.record_capture();
            }
        });
    ui.add_space(4.0);

    let mono = egui::FontId::monospace(12.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |scroll| {
            if rec.events.is_empty() {
                // The empty ledger is the placeholder: a dim mark of the
                // mode, and what the first entry will take.
                scroll.add_space(24.0);
                scroll.vertical_centered(|centre| {
                    let icon_rect = centre
                        .allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover())
                        .0;
                    icons
                        .record
                        .paint(centre, icon_rect, egui::Color32::from_gray(90));
                    centre.add_space(6.0);
                    centre.label(
                        egui::RichText::new("No events yet.").color(egui::Color32::from_gray(140)),
                    );
                    centre.label(
                        egui::RichText::new(
                            "Interact with the widget, or press Capture\n\
                             to commit its current state.",
                        )
                        .color(egui::Color32::from_gray(120)),
                    );
                });
            } else {
                for ev in &rec.events {
                    let secs = ev.at_ms as f32 / 1000.0;
                    let kind = EventKind::of(&ev.event);
                    scroll.horizontal(|line| {
                        line.spacing_mut().item_spacing.x = 4.0;
                        let icon_rect = line
                            .allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::hover())
                            .0;
                        kind.icon(icons)
                            .paint(line, icon_rect, kind.colour(palette));
                        line.label(
                            egui::RichText::new(format!(
                                "{secs:>5.2}s {}",
                                format_event_label(&ev.event)
                            ))
                            .font(mono.clone()),
                        );
                    });
                }
            }
        });
}

/// The log's per-event mark: an icon and a colour, so kinds read at a
/// glance down the ledger rather than by reading each line's text.
enum EventKind {
    Capture,
    Touch,
    Delivery,
    Network,
    Output,
}

impl EventKind {
    fn of(event: &UnifiedEvent) -> Self {
        match event {
            UnifiedEvent::Capture { .. } => Self::Capture,
            UnifiedEvent::Click { .. }
            | UnifiedEvent::Scroll { .. }
            | UnifiedEvent::Drag { .. } => Self::Touch,
            UnifiedEvent::ParamDelivery { .. }
            | UnifiedEvent::SystemDelivery { .. }
            | UnifiedEvent::CredentialDelivery { .. } => Self::Delivery,
            UnifiedEvent::Fetch { .. }
            | UnifiedEvent::SsdpFound { .. }
            | UnifiedEvent::SsdpRemoved { .. }
            | UnifiedEvent::MdnsFound { .. }
            | UnifiedEvent::MdnsRemoved { .. }
            | UnifiedEvent::WsOpen { .. }
            | UnifiedEvent::WsMessage { .. }
            | UnifiedEvent::WsClose { .. }
            | UnifiedEvent::SocketConnected { .. }
            | UnifiedEvent::SocketData { .. }
            | UnifiedEvent::SocketClosed { .. }
            | UnifiedEvent::UdpResponse { .. } => Self::Network,
            UnifiedEvent::AudioPlay { .. }
            | UnifiedEvent::LedSetEndless { .. }
            | UnifiedEvent::LedSetTemporary { .. }
            | UnifiedEvent::LedStop => Self::Output,
        }
    }

    fn icon<'i>(&self, icons: &'i mut super::icon::Icons) -> &'i mut super::icon::Icon {
        match self {
            Self::Capture => &mut icons.record,
            Self::Touch => &mut icons.touch,
            Self::Delivery => &mut icons.delivery,
            Self::Network => &mut icons.network,
            Self::Output => &mut icons.output,
        }
    }

    fn colour(&self, palette: &super::theme::Palette) -> egui::Color32 {
        match self {
            Self::Capture => palette.record_accent,
            Self::Touch => egui::Color32::from_rgb(120, 170, 255),
            Self::Delivery => egui::Color32::from_rgb(170, 140, 255),
            Self::Network => egui::Color32::from_rgb(120, 200, 150),
            Self::Output => egui::Color32::from_rgb(160, 160, 160),
        }
    }
}

impl TestbedApp {
    /// The take's own sidebar, docked on the left while recording runs.
    ///
    /// A panel rather than a window: the event log is the operator's ledger —
    /// the recommendation is to commit each settled state to it via `Capture` —
    /// so it must stay visible for the whole take,
    /// not draggable away or buried under a device window.
    pub(super) fn paint_recording_sidebar(&mut self, root_ui: &mut egui::Ui) {
        if self.recording_mode.active().is_none() {
            return;
        }
        let palette = self.theme.palette(root_ui.ctx());

        // Same idiom as the right panel: the panel reserves the space,
        // and a foreground area paints in it, above the floating device windows.
        let panel = egui::SidePanel::left("recording_panel")
            .resizable(false)
            .exact_width(RECORDING_PANEL_W)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show_inside(root_ui, |_| {});
        let rect = panel.response.rect;
        let record_accent = palette.record_accent;
        let panel_fill = palette.panel_fill;
        egui::Area::new(egui::Id::new("recording_chrome"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(root_ui.ctx(), |area| {
                area.set_clip_rect(rect);
                area.painter().rect_filled(rect, 0.0, panel_fill);
                area.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0_f32, record_accent),
                    egui::StrokeKind::Inside,
                );
                let mut ui = area.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)));
                let icons = &mut self.icons;
                if let Some(rec) = self.recording_mode.active_mut() {
                    paint_recording_panel(&mut ui, rec, icons, palette);
                }
            });
    }

    /// Merge the take's event sources (user actions, network events from the
    /// runtime, fetch events from the shared buffer), validate, and write a
    /// `.jsonl.gz` fixture into the widget's `capture/fixtures/<dataset>.jsonl.gz`.
    /// Also updates the widget's `capture/config.toml` to point at the new
    /// fixture. Runs before the unwind: the drain needs the view live.
    ///
    /// Returns what happened, worded for the on-screen notice; the exit
    /// unwind erases every other trace of the take from the UI.
    pub(super) fn write_recording(&mut self, rec: RecordingState) -> Result<String, String> {
        // Pull network events out of the active tile's runtime, plus the fetch events the
        // observer pushed into the shared buffer.
        let runtime_events = self
            .tiles
            .get_mut(rec.active_tile)
            .map(DeviceView::take_recorded_events)
            .unwrap_or_default();
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
            let message = format!(
                "could not find the widget root — fixture not saved ({} event(s) lost)",
                fixture.events.len()
            );
            eprintln!("error: {message}");
            return Err(message);
        };
        let fixture_dir = widget_root.join("capture").join("fixtures");
        let fixture_path = fixture_dir.join(format!("{}.jsonl.gz", rec.dataset));

        if let Err(e) = bmc_wasm_runtime::unified_fixture::validate_fixture(&fixture) {
            eprintln!("warning: fixture validation failed: {e:#} (writing anyway)");
        }
        if let Err(e) = fixtures::write_jsonl_fixture(&fixture_path, &fixture) {
            let message = format!("failed to write {}: {e:#}", fixture_path.display());
            eprintln!("error: {message}");
            return Err(message);
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

        let bytes = std::fs::metadata(&fixture_path).map_or(0, |m| m.len());
        Ok(format!(
            "Saved {} — {} event(s), {:.1} KiB\n{}\nconfig.toml updated; \
             `just wasm::update-baselines {widget_name}` sets baselines",
            rec.dataset,
            fixture.events.len(),
            bytes as f64 / 1024.0,
            fixture_path.display(),
        ))
    }
}

#[cfg(test)]
mod begin_tests {
    use super::RecordingState;

    fn begin(target: &str) -> RecordingState {
        let manifest_json = serde_json::json!({
            "uid": "550e8400-e29b-41d4-a716-446655440201",
            "version": "0.1.0",
            "name": "Test",
            "description": "Fixture",
            "author": { "name": "Braiins Forge", "url": "https://braiinsforge.com" },
            "binary": "bin/test",
            "icon": "assets/icon.svg",
            "category": "utility",
            "settings": [],
            "supported_viewports": [{ "type": "rectangular" }],
            "params": {
                "city": {
                    "name": "City",
                    "description": "Fixture param",
                    "type": "string",
                    "default_value": "Prague",
                },
            },
        })
        .to_string();
        let manifest =
            <bmc_widget_manifest::Manifest as std::str::FromStr>::from_str(&manifest_json)
                .expect("BUG: the fixture manifest must parse");
        let params = bmc_wasm_runtime::manifest_default_params(&manifest);
        let mut credentials = serde_json::Map::new();
        credentials.insert("pool".to_owned(), serde_json::json!({ "token": "t" }));

        RecordingState::begin(
            target.parse().expect("BUG: target must parse"),
            "take".to_owned(),
            None,
            &params,
            &bmc_wasm_runtime::SystemSnapshot::default(),
            &credentials,
        )
    }

    #[test]
    fn the_active_tile_is_the_viewport_position_on_its_platform() {
        assert_eq!(begin("bmc100:full").active_tile(), 0);
        assert_eq!(begin("bmc100:small").active_tile(), 3);
    }

    #[test]
    fn snapshots_carry_the_state_the_operator_was_looking_at() {
        let state = begin("bmc100:full");

        assert_eq!(
            state.params_snapshot.get("city"),
            Some(&serde_json::json!("Prague")),
            "the fixture header must carry the params as JSON, self-contained",
        );
        assert!(state.credentials_snapshot.contains_key("pool"));
    }

    #[test]
    fn the_start_time_satisfies_the_fixture_header_parser() {
        let state = begin("bmc100:full");

        chrono::DateTime::parse_from_rfc3339(&state.start_time_iso)
            .expect("BUG: the header parser rejects times without a timezone suffix");
    }
}

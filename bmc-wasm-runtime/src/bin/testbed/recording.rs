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

//! Recording-mode state and UI: gesture tracking, the event-log panel, and
//! the write path that merges user / network / fetch event sources into a
//! fixture on disk plus updates the widget's capture config.

use std::path::PathBuf;

use bmc_wasm_runtime::fixtures;
use bmc_wasm_runtime::unified_fixture::{
    FixtureHeader, TimelineEvent, UnifiedEvent, UnifiedFixture,
};

use super::TestbedApp;
use super::theme::{Tone, spacing};
use super::ui_helpers::{
    DialogPrimary, FooterClick, dialog_body, dialog_footer, dialog_header, dialog_surface,
    target_name, text_field,
};
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

/// The take's own clock: milliseconds since the operator pressed Record,
/// so a fixture's timeline starts at zero however long the testbed ran first.
///
/// It follows the host's monotonic clock, not wall time: the fast-forward
/// lives there, so replay advances through the same span the widget saw.
///
/// Shared, because a threaded view's fetch observer stamps off-thread.
struct TakeClock {
    /// The host reading this take treats as zero.
    epoch_monotonic_ms: u64,
    now_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl TakeClock {
    fn start(monotonic_ms: u64) -> Self {
        Self {
            epoch_monotonic_ms: monotonic_ms,
            now_ms: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Republish the host's reading on the take's scale, once per frame.
    ///
    /// An off-thread reader lags by up to one repaint (`DRAIN_TICK_MS`, 33 ms),
    /// which replay's 16 ms frame and the 500 ms capture debounce both swallow.
    /// Worth it to keep every stamp derived from a single reading.
    fn advance(&self, monotonic_ms: u64) {
        self.now_ms.store(
            self.rebase(monotonic_ms),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    fn now_ms(&self) -> u64 {
        self.now_ms.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Put a host-stamped `at_ms` on the take's scale: the runtime stamps events
    /// with the clock the host feeds it, so only their origin needs moving.
    fn rebase(&self, monotonic_ms: u64) -> u64 {
        monotonic_ms.saturating_sub(self.epoch_monotonic_ms)
    }

    /// A handle for the fetch observer, which stamps from a view's own thread.
    fn reader(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        std::sync::Arc::clone(&self.now_ms)
    }
}

impl RecordingState {
    /// Append a delivery event to the timeline at the take's current time
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
    /// The one it slides is remembered, not recognised: an operator's Capture
    /// is the same shape, and a gesture's carries a duration.
    fn record_delivery(&mut self, make_event: impl FnOnce() -> UnifiedEvent) {
        let at_ms = self.clock.now_ms();
        self.events.push(TimelineEvent {
            at_ms,
            event: make_event(),
        });
        if !self.auto_capture {
            return;
        }
        let capture_at = at_ms + AUTO_CAPTURE_DELAY_MS;
        let pending = self
            .pending_auto_capture
            .and_then(|index| self.events.get_mut(index))
            .filter(|capture| capture.at_ms > at_ms);
        if let Some(prev_capture) = pending {
            prev_capture.at_ms = capture_at;
        } else {
            self.pending_auto_capture = Some(self.events.len());
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
    /// Which event in `events` is this cluster's auto-`Capture`.
    /// Stable because a take only ever appends.
    pending_auto_capture: Option<usize>,
    /// Events already pulled out of the view's runtime and the fetch buffer.
    /// Both hand their contents over once and forget them, so a failed write
    /// would lose them; held here, every Save attempt merges them afresh.
    drained: Vec<TimelineEvent>,
    gesture: Option<GestureTracker>,
    /// Widget root directory (for output paths).
    widget_root: Option<PathBuf>,
    clock: TakeClock,
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
        !self.events.is_empty() || !self.drained.is_empty()
    }

    /// Take custody of events drained from the runtime and the fetch buffer.
    pub(super) fn absorb(&mut self, events: Vec<TimelineEvent>) {
        self.drained.extend(events);
    }

    /// The fixture this take would write: user actions and drained events
    /// merged, sorted by `at_ms` (stable, so insertion order breaks ties),
    /// with consecutive scrolls of one element collapsed into a single event.
    ///
    /// Built from a borrow rather than consuming the take, so a failed write
    /// can be retried once the operator has dealt with what failed.
    fn fixture(&self) -> UnifiedFixture {
        let mut all_events = self.events.clone();
        all_events.extend(self.drained.iter().cloned());
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

        UnifiedFixture {
            header: FixtureHeader {
                time: self.start_time_iso.clone(),
                kv: self.kv_snapshot.clone(),
                initial_params: self.params_snapshot.clone(),
                initial_system: self.system_snapshot.clone(),
                initial_credentials: self.credentials_snapshot.clone(),
            },
            events: merged,
        }
    }

    /// The wiped-and-seeded KV state the fixture header reproduces on replay.
    /// `build_views` reports it once the take's KV dir exists.
    pub(super) fn set_kv_baseline(&mut self, kv: std::collections::HashMap<String, String>) {
        self.kv_snapshot = kv;
    }

    /// Write the take as `capture/fixtures/<dataset>.jsonl.gz` and point the
    /// widget's `capture/config.toml` at it. Every outcome is worded for the
    /// on-screen notice.
    ///
    /// Leaves the take intact either way, so a failure can be answered by
    /// fixing what failed and pressing Save again.
    pub(super) fn write(&self) -> Result<Saved, String> {
        let fixture = self.fixture();

        let Some(widget_root) = &self.widget_root else {
            let message = "could not find the widget root — nothing was written".to_owned();
            eprintln!("error: {message}");
            return Err(message);
        };
        let fixture_path = widget_root
            .join("capture")
            .join("fixtures")
            .join(format!("{}.jsonl.gz", self.dataset));

        // Reported rather than refused: validation judges what was recorded.
        // A second Save fails identically, so an `Err` would leave the operator
        // a take they can only discard.
        let warning = bmc_wasm_runtime::unified_fixture::validate_fixture(&fixture)
            .err()
            .map(|e| format!("the fixture did not validate: {e:#}"));
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
        let fixture_rel = format!("fixtures/{}.jsonl.gz", self.dataset);
        // A fixture nothing points at is not a saved take.
        if let Err(e) = fixtures::update_config_toml_fixtures(
            &config_path,
            &self.dataset,
            &fixture_rel,
            self.target,
        ) {
            let message = format!("failed to update {}: {e:#}", config_path.display());
            eprintln!("error: {message}");
            return Err(message);
        }
        eprintln!("updated: {}", config_path.display());

        let widget_name = widget_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("WIDGET");
        eprintln!("hint: run `just wasm::update-baselines {widget_name}` to set baselines");

        let bytes = std::fs::metadata(&fixture_path).map_or(0, |m| m.len());
        #[expect(
            clippy::cast_precision_loss,
            reason = "a fixture that reached 2^53 bytes has a bigger problem than its rounding"
        )]
        let kib = bytes as f64 / 1024.0;
        // Widget-relative, since the absolute path is mostly the operator's
        // own home directory and pushes the part that identifies the take
        // off the readable width.
        Ok(Saved {
            summary: indoc::formatdoc! {"
                {dataset} — {events} events, {kib:.1} KiB
                {fixture_rel}
                baselines: just wasm::update-baselines {widget_name}",
                dataset = self.dataset,
                events = fixture.events.len(),
            },
            warning,
        })
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
        let at_ms = self.clock.now_ms();
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

/// A written take: the notice's body, plus any complaint about its contents.
///
/// The complaint is a value rather than a stderr line, so the GUI can say it.
pub(super) struct Saved {
    pub(super) summary: String,
    /// The fixture is on disk and bound; its contents failed validation.
    pub(super) warning: Option<String>,
}

/// One dataset a target already replays, as the naming dialog lists it.
pub(super) struct RecordedDataset {
    pub(super) name: String,
    /// The other targets this dataset drives.
    pub(super) also_drives: Vec<String>,
    pub(super) settle_delay: Option<u32>,
    pub(super) kv_keys: usize,
}

impl RecordedDataset {
    /// The row's attributes on one line, empty when it carries none.
    fn notes(&self) -> String {
        let mut notes = Vec::new();
        if !self.also_drives.is_empty() {
            notes.push(format!("also drives {}", self.also_drives.join(", ")));
        }
        if let Some(settle) = self.settle_delay {
            notes.push(format!("settle {settle}"));
        }
        match self.kv_keys {
            0 => {}
            1 => notes.push("1 KV key".to_owned()),
            keys => notes.push(format!("{keys} KV keys")),
        }
        notes.join(" · ")
    }
}

/// What the dialog's primary button would do with the name typed into it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NameVerdict {
    /// Nothing typed yet, or characters a dataset name cannot carry.
    Unusable,
    /// A name the config does not hold — a scenario alongside the rest.
    New,
    /// Re-records the dataset this viewport already carries under that name.
    Replaces,
    /// The name belongs to other viewports, listed. Recording it replaces
    /// their data too, since the config writer keeps their bindings and a
    /// dataset holds one fixture.
    Rebinds { drives: Vec<String> },
    /// The config would not load, so what this name replaces is unknown.
    Unknown { why: String },
}

impl NameVerdict {
    /// Whether committing would, or might, replace data already recorded.
    fn destructive(&self) -> bool {
        matches!(
            self,
            Self::Replaces | Self::Rebinds { .. } | Self::Unknown { .. }
        )
    }
}

/// Every dataset the widget's capture config holds, by the target it drives.
///
/// Wrapped rather than passed as the bare map, because a name is unique across
/// the whole config — one `[fixtures.<name>]` table, one fixture file — so no
/// single target's rows can judge one.
pub(super) struct RecordedFixtures {
    by_target: std::collections::HashMap<String, Vec<RecordedDataset>>,
    /// Why the config could not be read, when it could not be.
    ///
    /// An empty map otherwise means the widget recorded nothing.
    /// A failed load must not borrow that meaning, since the writer
    /// would replace a fixture it never saw.
    unreadable: Option<String>,
}

impl RecordedFixtures {
    pub(super) fn new(by_target: std::collections::HashMap<String, Vec<RecordedDataset>>) -> Self {
        Self {
            by_target,
            unreadable: None,
        }
    }

    /// A config that would not load, so nothing is known about what it holds.
    pub(super) fn unreadable(why: String) -> Self {
        Self {
            by_target: std::collections::HashMap::new(),
            unreadable: Some(why),
        }
    }

    /// Whether an empty row list means "nothing here" or "cannot tell".
    pub(super) fn is_unreadable(&self) -> bool {
        self.unreadable.is_some()
    }

    /// What `target` already replays.
    pub(super) fn of(
        &self,
        target: bmc_wasm_runtime::platform_catalog::Target,
    ) -> &[RecordedDataset] {
        self.by_target
            .get(&target.to_string())
            .map_or(&[], Vec::as_slice)
    }

    /// Judge `dataset` for a take on `target`.
    ///
    /// A name that is empty or malformed is `Unusable`, which is what forces
    /// the operator to name the take: `is_valid_dataset_name` rejects the
    /// empty string.
    pub(super) fn judge(
        &self,
        dataset: &str,
        target: bmc_wasm_runtime::platform_catalog::Target,
    ) -> NameVerdict {
        if !bmc_wasm_runtime::capture_config::is_valid_dataset_name(dataset) {
            return NameVerdict::Unusable;
        }
        if let Some(why) = &self.unreadable {
            return NameVerdict::Unknown { why: why.clone() };
        }
        if self.of(target).iter().any(|row| row.name == dataset) {
            return NameVerdict::Replaces;
        }
        let mine = target.to_string();
        // Sorted: the map's order is arbitrary, and this reads as a caption.
        let mut drives: Vec<String> = self
            .by_target
            .iter()
            .filter(|(id, rows)| **id != mine && rows.iter().any(|row| row.name == dataset))
            .map(|(id, _)| id.clone())
            .collect();
        drives.sort();
        if drives.is_empty() {
            NameVerdict::New
        } else {
            NameVerdict::Rebinds { drives }
        }
    }
}

/// A target clicked in the choosing phase, and the dialog naming its take.
///
/// One value for the whole lifecycle — open, typed into, submitted — so a
/// submitted name cannot drift from the dialog that produced it, and no state
/// exists where a choice is pending without the dialog it came from.
pub(super) struct Naming {
    pub(super) target: bmc_wasm_runtime::platform_catalog::Target,
    /// Starts empty: a take cannot be written without a name, so the dialog
    /// asks for one rather than defaulting into an overwrite.
    pub(super) dataset: String,
    submitted: bool,
}

impl Naming {
    fn new(target: bmc_wasm_runtime::platform_catalog::Target) -> Self {
        Self {
            target,
            dataset: String::new(),
            submitted: false,
        }
    }

    /// Start the take on the name typed so far. The dialog stops painting from
    /// here, and the app picks the choice up at the end of the frame.
    pub(super) fn submit(&mut self) {
        self.submitted = true;
    }
}

enum RecordPhase {
    Off,
    /// Overlays up over every candidate viewport; no take yet.
    Choosing {
        /// The widget's capture config as the dialog reads it: what the clicked
        /// target replays, and whether the typed name is anyone else's.
        recorded: RecordedFixtures,
        /// The open dialog, from the click that opened it to the submit the
        /// app consumes at the end of that frame.
        naming: Option<Naming>,
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
        /// The take's own state, dropped with this phase.
        sandbox: super::SandboxedState,
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

    /// The running take's state, which displaces the playground.
    pub(crate) fn sandbox(&self) -> Option<&super::SandboxedState> {
        match &self.phase {
            RecordPhase::Recording { sandbox, .. } => Some(sandbox),
            RecordPhase::Off | RecordPhase::Choosing { .. } => None,
        }
    }

    pub(crate) fn sandbox_mut(&mut self) -> Option<&mut super::SandboxedState> {
        match &mut self.phase {
            RecordPhase::Recording { sandbox, .. } => Some(sandbox),
            RecordPhase::Off | RecordPhase::Choosing { .. } => None,
        }
    }

    /// Off → Choosing. Refused while engaged, so a phase cannot be lost.
    pub(super) fn open_choosing(
        &mut self,
        recorded: RecordedFixtures,
        restore_platforms: Vec<&'static bmc_wasm_runtime::platform_catalog::Platform>,
    ) -> bool {
        if self.engaged() {
            return false;
        }
        self.phase = RecordPhase::Choosing {
            recorded,
            naming: None,
            restore_platforms,
        };
        true
    }

    /// What `target` already replays; empty outside the choosing phase.
    pub(super) fn recorded_datasets(
        &self,
        target: bmc_wasm_runtime::platform_catalog::Target,
    ) -> &[RecordedDataset] {
        match &self.phase {
            RecordPhase::Choosing { recorded, .. } => recorded.of(target),
            RecordPhase::Off | RecordPhase::Recording { .. } => &[],
        }
    }

    /// The open dialog: the name being typed, and the config it is judged
    /// against. Both live in the same phase, so one borrow serves.
    pub(super) fn naming_dialog(&mut self) -> Option<(&mut Naming, &RecordedFixtures)> {
        match &mut self.phase {
            RecordPhase::Choosing {
                recorded, naming, ..
            } => {
                // A submitted dialog is on its way out: painting it again
                // would offer a second submit against a take already starting.
                let naming = naming.as_mut().filter(|n| !n.submitted)?;
                Some((naming, recorded))
            }
            RecordPhase::Off | RecordPhase::Recording { .. } => None,
        }
    }

    /// An overlay was clicked: open its dialog, which asks for the name.
    pub(super) fn choose(&mut self, target: bmc_wasm_runtime::platform_catalog::Target) {
        if let RecordPhase::Choosing { naming, .. } = &mut self.phase {
            *naming = Some(Naming::new(target));
        }
    }

    pub(super) fn cancel_naming(&mut self) {
        if let RecordPhase::Choosing { naming, .. } = &mut self.phase {
            *naming = None;
        }
    }

    /// The target and dataset a submitted dialog carries, taken once.
    pub(super) fn take_choice(
        &mut self,
    ) -> Option<(bmc_wasm_runtime::platform_catalog::Target, String)> {
        let RecordPhase::Choosing { naming, .. } = &mut self.phase else {
            return None;
        };
        if !naming.as_ref().is_some_and(|n| n.submitted) {
            return None;
        }
        naming.take().map(|n| (n.target, n.dataset))
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
        sandbox: super::SandboxedState,
        kv_stash: (PathBuf, Option<PathBuf>),
        current_canvas: &[&'static bmc_wasm_runtime::platform_catalog::Platform],
        monotonic_ms: u64,
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
                &sandbox,
                monotonic_ms,
            )),
            restore_platforms,
            kv_stash,
            sandbox,
        };
        true
    }

    /// A reader on the take's clock for the fetch observer, which stamps
    /// `at_ms` against it; `None` outside a take.
    pub(super) fn take_clock(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicU64>> {
        self.active().map(|rec| rec.clock.reader())
    }

    /// Move the running take's clock to the host's reading for this frame.
    pub(super) fn advance_clock(&self, monotonic_ms: u64) {
        if let Some(rec) = self.active() {
            rec.clock.advance(monotonic_ms);
        }
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

    /// Write the running take without touching the phase; `None` outside a
    /// take. Leaving the mode standing is what lets a failed Save be retried.
    pub(super) fn write_take(&self) -> Option<Result<Saved, String>> {
        self.active().map(RecordingState::write)
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
        sandbox: &super::SandboxedState,
        monotonic_ms: u64,
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
            pending_auto_capture: None,
            drained: Vec::new(),
            gesture: None,
            widget_root,
            clock: TakeClock::start(monotonic_ms),
            // Filled by `build_views` once the take's KV dir is wiped and seeded.
            kv_snapshot: std::collections::HashMap::new(),
            // Pre-encoded into the JSON shape `FixtureHeader::initial_params`
            // expects, so the fixture is self-contained: replay never has to
            // locate the widget's `manifest.json` to reconstruct the baseline.
            params_snapshot: sandbox
                .params
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
                .collect(),
            system_snapshot: sandbox.system.clone(),
            credentials_snapshot: sandbox.credentials.clone(),
            // The fast-forward is included: the host's raw clock
            // would start replay at a time the take never saw.
            // Capture's parser also rejects a naive datetime,
            // so keep the timezone suffix (e.g. `2026-05-13T15:48:38+02:00`).
            start_time_iso: (chrono::Local::now()
                + chrono::Duration::milliseconds(sandbox.clock_offset_ms.cast_signed()))
            .to_rfc3339(),
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
        let at_ms = self.clock.now_ms();

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

/// Wide enough that a dataset row's name and its attributes
/// share one line in the common cases.
const DIALOG_W: f32 = 460.0;

/// A dataset row's height, and the column its icon sits in.
const ROW_H: f32 = 32.0;
const ROW_ICON: f32 = 18.0;

/// The datasets a viewport already replays: one row each, the whole row
/// filling the name in — re-recording one should not mean retyping it.
///
/// The empty case keeps the icon column and the row's height so the section
/// holds its shape, but says its own thing rather than posing as a row with
/// a control that would do nothing.
fn paint_dataset_rows(
    ui: &mut egui::Ui,
    rows: &[RecordedDataset],
    unreadable: bool,
    dataset: &mut String,
    icons: &mut super::icon::Icons,
    palette: &super::theme::Palette,
) {
    let width = ui.available_width();
    if rows.is_empty() {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::hover());
        let (say, colour) = if unreadable {
            (
                "the config would not load — what it holds is unknown",
                palette.action_danger,
            )
        } else {
            (
                "nothing recorded here yet — this take would be the first",
                palette.text_disabled,
            )
        };
        icons.record.paint(ui, row_icon_rect(rect), colour);
        ui.painter().text(
            egui::pos2(rect.left() + row_text_x(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            say,
            egui::FontId::proportional(13.0),
            colour,
        );
        return;
    }
    let strong = ui.visuals().strong_text_color();
    for (order, row) in rows.iter().enumerate() {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.field);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // Between rows only: a rule under the last one would read as the
        // section's own border rather than as a separator.
        if order > 0 {
            ui.painter().hline(
                rect.x_range(),
                rect.top(),
                egui::Stroke::new(1.0_f32, palette.border_subtle),
            );
        }
        icons
            .saved
            .paint(ui, row_icon_rect(rect), palette.accent_record);
        let name = ui.painter().text(
            egui::pos2(rect.left() + row_text_x(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            &row.name,
            egui::FontId::proportional(13.0),
            strong,
        );
        let notes = row.notes();
        if !notes.is_empty() {
            ui.painter().text(
                egui::pos2(name.right() + spacing::S05, rect.center().y),
                egui::Align2::LEFT_CENTER,
                notes,
                egui::FontId::proportional(12.0),
                palette.text_disabled,
            );
        }
        if response.clicked() {
            dataset.clone_from(&row.name);
        }
        response.on_hover_text("Use this name to re-record it");
    }
}

/// Smaller than the button's square, so the face reads as a button, not a glyph.
const NAME_FILL_ICON: f32 = 14.0;

/// Offers the conventional name rather than prefilling it:
/// no take is written under a name nobody looked at.
fn paint_name_field(ui: &mut egui::Ui, naming: &mut Naming, icons: &mut super::icon::Icons) {
    let conventional = bmc_wasm_runtime::capture_config::conventional_dataset_name(naming.target);
    ui.horizontal(|row| {
        row.spacing_mut().item_spacing.x = spacing::S02;
        let side = super::ui_helpers::field_height(row);
        let width = (row.available_width() - side - spacing::S02).max(0.0);
        // No hint: a placeholder name reads as one already given.
        text_field(row, width, &mut naming.dataset, "");
        let (rect, fill) = row.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
        let visuals = *row.style().interact(&fill);
        row.painter()
            .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        icons.automatic.paint(
            row,
            egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(NAME_FILL_ICON)),
            visuals.fg_stroke.color,
        );
        if super::ui_helpers::with_pointer(fill)
            .on_hover_text(format!("name it {conventional}"))
            .clicked()
        {
            naming.dataset.clone_from(&conventional);
        }
    });
}

/// The icon's square, centred in the row's leading column.
fn row_icon_rect(row: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(row.left() + spacing::S02 + ROW_ICON / 2.0, row.center().y),
        egui::Vec2::splat(ROW_ICON),
    )
}

/// Where a row's text starts, clear of the icon column.
fn row_text_x() -> f32 {
    spacing::S02 + ROW_ICON + spacing::S03
}

/// The sidebar's content: the take, its dataset, the Capture control,
/// and the event log filling every remaining pixel.
/// Save and Cancel live in the toolbar, which the mode owns while it runs.
fn paint_recording_panel(
    ui: &mut egui::Ui,
    rec: &mut RecordingState,
    icons: &mut super::icon::Icons,
    palette: &super::theme::Palette,
) {
    let accent = palette.accent_record;

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
        .fill(palette.layer_inset)
        .inner_margin(8.0)
        .corner_radius(4.0)
        .show(ui, |group| {
            group.label("Commit each settled widget state to the take:");
            group.add_space(6.0);
            let mut capture = false;
            group.horizontal(|row| {
                capture = super::ui_helpers::Button::inline("Capture")
                    .icon(&mut icons.camera)
                    .tone(Tone::record(palette))
                    .show(row, palette)
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
            Self::Capture => &mut icons.camera,
            Self::Touch => &mut icons.touch,
            Self::Delivery => &mut icons.delivery,
            Self::Network => &mut icons.network,
            Self::Output => &mut icons.output,
        }
    }

    fn colour(&self, palette: &super::theme::Palette) -> egui::Color32 {
        match self {
            Self::Capture => palette.accent_record,
            Self::Touch => egui::Color32::from_rgb(120, 170, 255),
            Self::Delivery => egui::Color32::from_rgb(170, 140, 255),
            Self::Network => egui::Color32::from_rgb(120, 200, 150),
            Self::Output => egui::Color32::from_rgb(160, 160, 160),
        }
    }
}

impl TestbedApp {
    /// The dialog a chosen target opens: what that viewport already replays,
    /// and the name this take will write.
    ///
    /// The name is required rather than defaulted, because a default is what
    /// silently re-recorded whatever the viewport already carried.
    pub(super) fn paint_record_dialog(&mut self, ctx: &egui::Context) {
        let palette = self.theme.palette(ctx);
        let icons = &mut self.icons;
        let Some((naming, recorded)) = self.recording_mode.naming_dialog() else {
            return;
        };
        let target = naming.target;
        let rows = recorded.of(target);
        let mut cancel = false;

        let dialog = egui::Modal::new(egui::Id::new("record_dataset"))
            .frame(dialog_surface(palette))
            .backdrop_color(palette.backdrop)
            .show(ctx, |ui| {
                ui.set_width(DIALOG_W);
                let verdict = recorded.judge(&naming.dataset, target);

                dialog_body(ui, |ui| {
                    dialog_header(
                        ui,
                        &format!("Record {}", target_name(target)),
                        indoc::indoc! {"
                            A dataset is one recorded scenario.
                            A viewport can hold several, each with its own baselines."},
                    );

                    ui.label(egui::RichText::new("Already recorded").strong());
                    ui.add_space(spacing::S02);
                    paint_dataset_rows(
                        ui,
                        rows,
                        recorded.is_unreadable(),
                        &mut naming.dataset,
                        icons,
                        palette,
                    );

                    ui.add_space(spacing::S05);
                    ui.label(egui::RichText::new("Dataset name").strong());
                    ui.add_space(spacing::S02);
                    paint_name_field(ui, naming, icons);

                    // The caption keeps its line in every case, so typing
                    // into an empty field does not shunt the footer down
                    // under the pointer.
                    ui.label(match &verdict {
                        NameVerdict::Unusable if naming.dataset.is_empty() => {
                            egui::RichText::new("name the scenario this take records").weak()
                        }
                        NameVerdict::Unusable => {
                            egui::RichText::new("letters, digits, '-', '_' and '.' only")
                                .color(palette.action_danger)
                        }
                        NameVerdict::Replaces => {
                            egui::RichText::new("replaces the recording of that name")
                                .color(palette.action_danger)
                        }
                        NameVerdict::Rebinds { drives } => egui::RichText::new(format!(
                            "that name is {}'s — recording it replaces theirs too",
                            drives.join(", ")
                        ))
                        .color(palette.action_danger),
                        NameVerdict::Unknown { why } => egui::RichText::new(format!(
                            "cannot read what is already recorded — {why}"
                        ))
                        .color(palette.action_danger),
                        NameVerdict::New => {
                            egui::RichText::new("a new dataset for this viewport").weak()
                        }
                    });
                });

                let primary = if verdict.destructive() {
                    DialogPrimary {
                        // "Re-record" claims a replacement — the one thing
                        // an unreadable config cannot claim.
                        label: match verdict {
                            NameVerdict::Unknown { .. } => "Record anyway",
                            NameVerdict::Unusable
                            | NameVerdict::New
                            | NameVerdict::Replaces
                            | NameVerdict::Rebinds { .. } => "Re-record",
                        },
                        tone: Tone::danger(palette),
                        enabled: true,
                    }
                } else {
                    DialogPrimary {
                        label: "Record",
                        tone: Tone::primary(palette),
                        enabled: verdict != NameVerdict::Unusable,
                    }
                };
                match dialog_footer(ui, primary, palette) {
                    FooterClick::Primary => naming.submit(),
                    FooterClick::Cancel => cancel = true,
                    FooterClick::None => {}
                }
            });

        // Deliberately not `should_close`, which counts a backdrop click:
        // a stray click beside a half-typed name should not discard the take
        // it was about to start. Cancel and Escape are the ways out.
        let escaped = dialog.is_top_modal
            && !dialog.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        // Dismissal is the deferred one: it drops the value this closure holds.
        if cancel || escaped {
            self.recording_mode.cancel_naming();
        }
    }

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
        let record_accent = palette.accent_record;
        let panel_fill = palette.layer;
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

    /// Move everything the view's runtime and the fetch observer have
    /// buffered into the take. A no-op outside one.
    ///
    /// Called every frame rather than at Save, or a take of network traffic
    /// alone looks empty until the moment it is written.
    pub(super) fn drain_take_sources(&mut self) {
        let Some(active_tile) = self
            .recording_mode
            .active()
            .map(RecordingState::active_tile)
        else {
            return;
        };
        let runtime_events = self
            .stage
            .tile_mut(active_tile)
            .map(DeviceView::take_recorded_events)
            .unwrap_or_default();
        let mut drained = fixtures::fixture_events_to_timeline(&runtime_events);
        let fetched = std::mem::take(
            &mut *self
                .recording_mode
                .fetch_events
                .lock()
                .expect("BUG: fetch events poisoned"),
        );
        if let Some(rec) = self.recording_mode.active_mut() {
            // The observer already stamped the fetches on the take's clock;
            // only the runtime's own events arrive on the host's.
            for event in &mut drained {
                event.at_ms = rec.clock.rebase(event.at_ms);
            }
            drained.extend(fetched);
            rec.absorb(drained);
        }
    }
}

#[cfg(test)]
mod naming_tests {
    use super::{NameVerdict, RecordedDataset, RecordedFixtures, RecordingMode};

    fn row(name: &str) -> RecordedDataset {
        RecordedDataset {
            name: name.to_owned(),
            also_drives: Vec::new(),
            settle_delay: None,
            kv_keys: 0,
        }
    }

    fn target(spec: &str) -> bmc_wasm_runtime::platform_catalog::Target {
        spec.parse().expect("BUG: the target must parse")
    }

    fn fixtures(recorded: Vec<(&str, Vec<RecordedDataset>)>) -> RecordedFixtures {
        RecordedFixtures::new(
            recorded
                .into_iter()
                .map(|(target, rows)| (target.to_owned(), rows))
                .collect(),
        )
    }

    fn choosing(recorded: Vec<(&str, Vec<RecordedDataset>)>) -> RecordingMode {
        let mut mode = RecordingMode::new();
        assert!(
            mode.open_choosing(fixtures(recorded), Vec::new()),
            "BUG: an idle mode must open the choosing phase",
        );
        mode
    }

    /// The dialog opens on any target, not only one that would be overwritten.
    /// It is where the name is entered, so a viewport with nothing recorded
    /// needs it every bit as much as one with a fixture already.
    #[test]
    fn choosing_a_target_with_nothing_recorded_still_opens_the_dialog() {
        let mut mode = choosing(Vec::new());
        mode.choose(target("bmc100:small"));

        let (naming, recorded) = mode
            .naming_dialog()
            .expect("BUG: choosing a target must open its dialog");
        assert_eq!(naming.target.viewport.id, "small");
        assert!(
            recorded.of(naming.target).is_empty(),
            "nothing was recorded for that viewport",
        );
        assert!(
            naming.dataset.is_empty(),
            "the name is asked for, never defaulted",
        );
    }

    #[test]
    fn an_empty_or_malformed_name_cannot_start_a_take() {
        let recorded = fixtures(vec![("bmc100:small", vec![row("practice")])]);
        for unusable in ["", "../escape", "a/b", "with space"] {
            assert_eq!(
                recorded.judge(unusable, target("bmc100:small")),
                NameVerdict::Unusable,
                "'{unusable}' must not start a take",
            );
        }
    }

    /// Re-recording is offered, not blocked: refreshing a scenario's data is
    /// as ordinary as adding one. The verdict is what turns the button red.
    #[test]
    fn a_name_already_recorded_reads_as_a_replacement() {
        let recorded = fixtures(vec![(
            "bmc100:small",
            vec![row("practice"), row("qualifying")],
        )]);
        let small = target("bmc100:small");

        assert_eq!(recorded.judge("qualifying", small), NameVerdict::Replaces);
        assert_eq!(recorded.judge("race", small), NameVerdict::New);
    }

    /// An unreadable config holds no rows, exactly as a widget that recorded
    /// nothing does — and only one of those is safe to call new.
    #[test]
    fn an_unreadable_config_judges_no_name_as_new() {
        let recorded = RecordedFixtures::unreadable("fixture 'race' not found".to_owned());

        let verdict = recorded.judge("race", target("bmc100:small"));
        assert_eq!(
            verdict,
            NameVerdict::Unknown {
                why: "fixture 'race' not found".to_owned(),
            },
        );
        assert!(
            verdict.destructive(),
            "an unknown name must arm the footer as a re-record would",
        );
    }

    /// A dataset name is unique across the whole config, so one belonging to
    /// another viewport is not a free name: the writer would point it at this
    /// take's fixture and leave that viewport replaying it.
    #[test]
    fn a_name_another_viewport_owns_is_not_a_new_dataset() {
        let recorded = fixtures(vec![
            ("bmc100:full", vec![row("qualifying")]),
            ("bmc100:medium", vec![row("qualifying")]),
        ]);

        assert_eq!(
            recorded.judge("qualifying", target("bmc100:small")),
            NameVerdict::Rebinds {
                drives: vec!["bmc100:full".to_owned(), "bmc100:medium".to_owned()],
            },
        );
    }

    #[test]
    fn the_take_starts_on_the_name_the_dialog_carries() {
        let mut mode = choosing(vec![("bmc100:small", vec![row("practice")])]);
        mode.choose(target("bmc100:small"));

        let (naming, recorded) = mode.naming_dialog().expect("BUG: the dialog must be open");
        assert_eq!(
            recorded.of(naming.target).len(),
            1,
            "the dialog lists what that viewport carries",
        );
        naming.dataset = "qualifying".to_owned();
        naming.submit();

        let (chosen, dataset) = mode
            .take_choice()
            .expect("BUG: a submitted dialog must yield its choice");
        assert_eq!(chosen.viewport.id, "small");
        assert_eq!(dataset, "qualifying");
    }

    #[test]
    fn a_cancelled_dialog_leaves_the_phase_choosing() {
        let mut mode = choosing(Vec::new());
        mode.choose(target("bmc100:full"));
        mode.cancel_naming();

        assert!(mode.naming_dialog().is_none(), "the dialog closed");
        assert!(mode.take_choice().is_none(), "and started no take");
        assert!(mode.is_choosing(), "the overlays stay up to pick again");
    }
}

#[cfg(test)]
mod begin_tests {
    use super::RecordingState;

    /// A take entered three minutes into the session, so a stamp that skipped
    /// the rebase lands far from zero instead of coincidentally on it.
    const TAKE_EPOCH_MS: u64 = 180_000;

    fn begin(target: &str) -> RecordingState {
        begin_fast_forwarded(target, 0)
    }

    fn begin_fast_forwarded(target: &str, clock_offset_ms: u64) -> RecordingState {
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
            &crate::SandboxedState {
                params,
                system: bmc_wasm_runtime::SystemSnapshot::default(),
                credentials,
                offline: false,
                dormant: false,
                clock_offset_ms,
            },
            TAKE_EPOCH_MS,
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

    /// A manual `Capture` is the same shape as the debounced one, so a cluster
    /// spanning it must still settle into one auto-`Capture`, not two.
    #[test]
    fn a_manual_capture_mid_cluster_does_not_strand_the_debounced_one() {
        let mut state = begin("bmc100:full");
        let at = |state: &RecordingState, ms: u64| state.clock.advance(TAKE_EPOCH_MS + ms);

        at(&state, 1_000);
        state.record_delivery(delivery);
        at(&state, 1_100);
        state.record_capture();
        at(&state, 1_200);
        state.record_delivery(delivery);

        // Sorted as the fixture writer does: the slid Capture keeps the index
        // it was pushed at, so `events` is not in time order.
        let mut captures: Vec<u64> = state
            .events
            .iter()
            .filter(|e| matches!(e.event, super::UnifiedEvent::Capture { .. }))
            .map(|e| e.at_ms)
            .collect();
        captures.sort_unstable();
        assert_eq!(
            captures,
            vec![1_100, 1_700],
            "the operator's capture stands, and the cluster fires once 500 ms \
             after its last delivery",
        );
    }

    fn delivery() -> super::UnifiedEvent {
        super::UnifiedEvent::ParamDelivery {
            params: serde_json::Map::new(),
        }
    }

    #[test]
    fn every_source_stamps_one_host_reading_the_same_way() {
        let state = begin("bmc100:full");
        let host_ms = TAKE_EPOCH_MS + 2_000;
        state.clock.advance(host_ms);

        assert_eq!(state.clock.now_ms(), 2_000, "a gesture or fetch stamp");
        assert_eq!(
            state.clock.rebase(host_ms),
            2_000,
            "a drained runtime stamp"
        );
    }

    #[test]
    fn a_take_starts_at_zero_however_long_the_testbed_ran() {
        let state = begin("bmc100:full");
        state.clock.advance(TAKE_EPOCH_MS);

        assert_eq!(state.clock.now_ms(), 0);
    }

    #[test]
    fn the_start_time_satisfies_the_fixture_header_parser() {
        let state = begin("bmc100:full");

        chrono::DateTime::parse_from_rfc3339(&state.start_time_iso)
            .expect("BUG: the header parser rejects times without a timezone suffix");
    }

    /// Replay starts the widget at the header's time, so a take recorded
    /// on a fast-forwarded clock must be stamped on that clock, not the host's.
    #[test]
    fn the_start_time_carries_the_clock_the_take_ran_on() {
        const FAST_FORWARD_MS: u64 = 300_000;
        let fast_forward = chrono::Duration::milliseconds(FAST_FORWARD_MS.cast_signed());

        // Bracketed rather than given a tolerance:
        // the stamp reads the host clock once, inside this span.
        let before = chrono::Local::now().fixed_offset();
        let state = begin_fast_forwarded("bmc100:full", FAST_FORWARD_MS);
        let after = chrono::Local::now().fixed_offset();

        let stamp = chrono::DateTime::parse_from_rfc3339(&state.start_time_iso)
            .expect("BUG: the header parser rejects times without a timezone suffix");
        assert!(
            (before + fast_forward..=after + fast_forward).contains(&stamp),
            "stamped {stamp}, which is not inside {before}..={after} \
             shifted by the {FAST_FORWARD_MS} ms fast-forward",
        );
    }

    #[test]
    fn a_failed_write_leaves_the_take_whole_for_a_retry() {
        use bmc_wasm_runtime::unified_fixture::{TimelineEvent, UnifiedEvent};

        let mut rec = begin("bmc100:full");
        rec.auto_capture = false;
        // A regular file as the widget root: `capture/fixtures/` cannot be
        // created under it, so the write fails on the filesystem itself.
        rec.widget_root =
            Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
        rec.record_tap((10.0, 20.0), Some("#start".to_owned()));
        rec.absorb(vec![TimelineEvent {
            at_ms: 5,
            event: UnifiedEvent::Capture {
                duration_ms: None,
                fps: None,
            },
        }]);

        assert!(
            rec.write().is_err(),
            "a fixture written under a regular file must fail"
        );

        assert!(rec.has_events(), "a failed write must not empty the take");
        assert_eq!(
            rec.fixture().events.len(),
            2,
            "the tap and the drained event must both survive for the retry",
        );
        assert!(
            rec.write().is_err(),
            "the retry must reach the same failure, not a different one",
        );
    }

    /// The config write carries the same contract as the fixture write —
    /// a failure keeps the take whole to be retried.
    #[test]
    fn a_fixture_left_unbound_is_a_failed_save() {
        use bmc_wasm_runtime::unified_fixture::{TimelineEvent, UnifiedEvent};

        let widget_root = tempfile::tempdir().expect("BUG: tempdir");
        // A directory where the config file belongs: the fixture still lands,
        // and only the binding fails.
        std::fs::create_dir_all(widget_root.path().join("capture").join("config.toml"))
            .expect("BUG: seed an unwritable config path");

        let mut rec = begin("bmc100:full");
        rec.auto_capture = false;
        rec.widget_root = Some(widget_root.path().to_owned());
        rec.absorb(vec![TimelineEvent {
            at_ms: 5,
            event: UnifiedEvent::Capture {
                duration_ms: None,
                fps: None,
            },
        }]);

        let message = rec
            .write()
            .err()
            .expect("BUG: an unwritable config must fail the save");
        assert!(
            message.contains("config.toml"),
            "the notice must name what failed: {message}",
        );
        assert!(
            widget_root
                .path()
                .join("capture/fixtures/take.jsonl.gz")
                .exists(),
            "the fixture is written before the binding that failed",
        );
        assert!(rec.has_events(), "the take stays whole for the retry");
    }
}

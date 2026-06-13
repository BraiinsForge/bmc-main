// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::Duration;

use bmc_led::data::{LedEffect as HwLedEffect, LedScene, Rgb};
use bmc_widget_protocol::{
    LED_REQUEST_ID_ALL, LedEffect as ProtoLedEffect, LedRequestId, LedRequestStatus, LedScope,
    RgbColor,
};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::warn;

use crate::compositor::{InstanceId, LedRequestStatusEvent};
use crate::config::WidgetSceneMap;
use crate::led_coordinator::{Layer, LedCoordinatorHandle};
use crate::scene::SceneId;

#[derive(Debug, Clone)]
struct EndlessEntry {
    instance_id: InstanceId,
    request_id: LedRequestId,
    scene: LedScene,
}

#[derive(Debug, Clone)]
struct TempEntry {
    instance_id: InstanceId,
    request_id: LedRequestId,
    scene: LedScene,
    /// Duration the temp wants to run. Set at submission and never
    /// mutated — the wall-clock deadline (`RunningTemp::until`) is
    /// derived once when the temp is promoted from the queue.
    duration: Duration,
}

#[derive(Debug)]
struct RunningTemp {
    entry: TempEntry,
    until: Instant,
}

#[derive(Debug, Default)]
struct SceneEffectState {
    /// Single endless slot per tier. A new endless request displaces
    /// any prior holder (same widget or different) with `Superseded` —
    /// last-write-wins; the displaced request does not come back.
    endless: Option<EndlessEntry>,
    temp_queue: VecDeque<TempEntry>,
    active_temp: Option<RunningTemp>,
}

pub(crate) struct LedSceneManager {
    coordinator: LedCoordinatorHandle,
    status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    widget_to_scene: HashMap<InstanceId, SceneId>,
    active_scene: Option<SceneId>,
    scenes: HashMap<SceneId, SceneEffectState>,
    global_state: SceneEffectState,
    /// Last scene published per layer. Used purely for in-process
    /// dedupe — `LedCoordinator` also dedupes on `(Layer, LedScene)`,
    /// but skipping the watch-channel send saves work on every refresh.
    applied_local: Option<LedScene>,
    applied_global: Option<LedScene>,
}

impl LedSceneManager {
    pub(crate) fn new(
        coordinator: LedCoordinatorHandle,
        status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    ) -> Self {
        Self {
            coordinator,
            status_tx,
            widget_to_scene: HashMap::new(),
            active_scene: None,
            scenes: HashMap::new(),
            global_state: SceneEffectState::default(),
            applied_local: None,
            applied_global: None,
        }
    }

    pub(crate) fn on_scene_changed(&mut self, scene_id: SceneId) {
        if self.active_scene == Some(scene_id) {
            return;
        }
        // Leaving a scene drops its in-flight temp — a local temp never
        // ticks off-screen and is not paused/resumed. The scene's
        // remaining queue stays put and plays from the next entry on its
        // next visit.
        self.drop_active_scene_temp();
        self.active_scene = Some(scene_id);
        self.scenes.entry(scene_id).or_default();
        self.refresh_active_scene_effect();
    }

    /// The compositor reports no active scene. Drops the outgoing scene's
    /// in-flight temp, same as navigating to a different scene.
    pub(crate) fn on_active_scene_cleared(&mut self) {
        if self.active_scene.is_none() {
            return;
        }
        self.drop_active_scene_temp();
        self.active_scene = None;
        self.refresh_active_scene_effect();
    }

    /// Drop the active scene's running temp (if any), emitting `Expired`.
    fn drop_active_scene_temp(&mut self) {
        let dropped = self
            .active_scene
            .and_then(|prev| self.scenes.get_mut(&prev))
            .and_then(|state| state.active_temp.take())
            .map(|active| (active.entry.instance_id, active.entry.request_id));
        if let Some((instance_id, request_id)) = dropped {
            self.emit(instance_id, request_id, LedRequestStatus::Expired);
        }
    }

    /// Reconcile `widget_to_scene` with the authoritative config snapshot.
    ///
    /// Widgets newly present in config are added to the mapping. Widgets
    /// that vanished from config have all their outstanding requests
    /// swept with `Superseded`. This is the only path that removes
    /// widgets from `widget_to_scene` — wayland-level disconnects (e.g.
    /// a widget process restart on size change) leave the mapping
    /// untouched, so transient respawns do not lose state.
    pub(crate) fn on_config_snapshot(&mut self, snapshot: WidgetSceneMap) {
        let removed: Vec<InstanceId> = self
            .widget_to_scene
            .keys()
            .filter(|id| !snapshot.contains_key(*id))
            .cloned()
            .collect();

        for instance_id in &removed {
            self.widget_to_scene.remove(instance_id);
            self.on_stop(instance_id, LED_REQUEST_ID_ALL);
        }

        for (instance_id, scene_id) in snapshot {
            self.widget_to_scene.insert(instance_id, scene_id);
            self.scenes.entry(scene_id).or_default();
        }

        self.evict_empty_scenes();
        self.refresh_active_scene_effect();
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "request-shape parameters mirror the wire arg list; bundling them adds a struct without clarifying anything"
    )]
    pub(crate) fn on_temporary(
        &mut self,
        instance_id: InstanceId,
        request_id: LedRequestId,
        effect: ProtoLedEffect,
        color: RgbColor,
        period_ms: u32,
        duration_ms: u32,
        scope: LedScope,
    ) {
        let entry = TempEntry {
            instance_id: instance_id.clone(),
            request_id,
            scene: build_scene(effect, color, period_ms, Some(u64::from(duration_ms))),
            duration: Duration::from_millis(u64::from(duration_ms)),
        };

        match scope {
            LedScope::Local => {
                if let Some(scene_id) = self.widget_to_scene.get(&instance_id).copied() {
                    self.scenes
                        .entry(scene_id)
                        .or_default()
                        .temp_queue
                        .push_back(entry);
                } else {
                    warn!(
                        widget = %instance_id,
                        request_id,
                        "led_state on_temporary: widget not in config snapshot; rejecting"
                    );
                    self.emit(instance_id, request_id, LedRequestStatus::Superseded);
                    return;
                }
            }
            LedScope::Global => self.global_state.temp_queue.push_back(entry),
        }

        self.emit(instance_id, request_id, LedRequestStatus::Accepted);
        self.refresh_active_scene_effect();
    }

    pub(crate) fn on_endless(
        &mut self,
        instance_id: InstanceId,
        request_id: LedRequestId,
        effect: ProtoLedEffect,
        color: RgbColor,
        period_ms: u32,
        scope: LedScope,
    ) {
        let new_entry = EndlessEntry {
            instance_id: instance_id.clone(),
            request_id,
            scene: build_scene(effect, color, period_ms, None),
        };

        let mut emissions: Vec<(InstanceId, LedRequestId, LedRequestStatus)> = Vec::new();
        match scope {
            LedScope::Local => {
                if let Some(scene_id) = self.widget_to_scene.get(&instance_id).copied() {
                    let slot = &mut self.scenes.entry(scene_id).or_default().endless;
                    apply_endless(slot, new_entry, &mut emissions);
                } else {
                    warn!(
                        widget = %instance_id,
                        request_id,
                        "led_state on_endless: widget not in config snapshot; rejecting"
                    );
                    emissions.push((
                        instance_id.clone(),
                        request_id,
                        LedRequestStatus::Superseded,
                    ));
                    for (id, rid, status) in emissions {
                        self.emit(id, rid, status);
                    }
                    return;
                }
            }
            LedScope::Global => {
                apply_endless(&mut self.global_state.endless, new_entry, &mut emissions);
            }
        }

        for (id, rid, status) in emissions {
            self.emit(id, rid, status);
        }
        self.emit(instance_id, request_id, LedRequestStatus::Accepted);

        self.refresh_active_scene_effect();
    }

    pub(crate) fn on_stop(&mut self, instance_id: &str, request_id: LedRequestId) {
        let cancel_all = request_id == LED_REQUEST_ID_ALL;
        let matches = |stored_instance: &str, stored_id: LedRequestId| -> bool {
            stored_instance == instance_id && (cancel_all || stored_id == request_id)
        };

        let mut superseded: Vec<(InstanceId, LedRequestId)> = Vec::new();

        for state in self
            .scenes
            .values_mut()
            .chain(std::iter::once(&mut self.global_state))
        {
            sweep_state(state, &matches, &mut superseded);
        }

        for (id, rid) in superseded {
            self.emit(id, rid, LedRequestStatus::Superseded);
        }

        self.refresh_active_scene_effect();
    }

    /// Reconcile against the compositor's connected-widget set: sweep
    /// every effect owned by a widget no longer connected. Idempotent —
    /// reconciling against the same set twice is a no-op. This is the
    /// disconnect-cleanup path; it works off latest state, so a dropped
    /// notification cannot leak a dead widget's effect.
    pub(crate) fn reconcile_connected(&mut self, connected: &BTreeSet<InstanceId>) {
        let stale: HashSet<InstanceId> = self
            .scenes
            .values()
            .chain(std::iter::once(&self.global_state))
            .flat_map(state_widget_ids)
            .filter(|widget| !connected.contains(widget))
            .collect();
        for widget in stale {
            self.on_stop(&widget, LED_REQUEST_ID_ALL);
        }
    }

    /// Drop scene entries that no widget still references and that hold
    /// no in-flight effects. Called from `on_config_snapshot` after
    /// widgets vanish from the config snapshot, so `scenes` does not
    /// accumulate dead entries across config reloads. The active scene
    /// is always retained so its tier state stays addressable.
    fn evict_empty_scenes(&mut self) {
        let referenced: HashSet<SceneId> = self.widget_to_scene.values().copied().collect();
        let active = self.active_scene;
        self.scenes.retain(|scene_id, state| {
            if referenced.contains(scene_id) || Some(*scene_id) == active {
                return true;
            }
            state.endless.is_some() || !state.temp_queue.is_empty() || state.active_temp.is_some()
        });
    }

    pub(crate) fn on_active_expiry(&mut self) {
        let now = Instant::now();
        let mut expired: Vec<(InstanceId, LedRequestId)> = Vec::new();

        if let Some(state) = self.active_scene.and_then(|id| self.scenes.get_mut(&id))
            && let Some(done) = expire_state_if_due(state, now)
        {
            expired.push(done);
        }
        if let Some(done) = expire_state_if_due(&mut self.global_state, now) {
            expired.push(done);
        }

        if expired.is_empty() {
            return;
        }

        for (instance_id, request_id) in expired {
            self.emit(instance_id, request_id, LedRequestStatus::Expired);
        }
        self.refresh_active_scene_effect();
    }

    /// Earliest wall-clock deadline across the active scene's local tier
    /// and the global tier — the only two that advance. A local temp
    /// ticks only while its scene is active, and the global tier has no
    /// scene affinity, so no inactive scene ever holds a running temp.
    pub(crate) fn active_deadline(&self) -> Option<Instant> {
        let active_local = self
            .active_scene
            .and_then(|id| self.scenes.get(&id))
            .and_then(|state| state.active_temp.as_ref())
            .map(|t| t.until);
        let global = self.global_state.active_temp.as_ref().map(|t| t.until);
        [active_local, global].into_iter().flatten().min()
    }

    /// Publish the winning scene for each widget layer.
    ///
    /// Cross-tier priority (local-wins-over-global) is owned by
    /// `LedCoordinator` via its layer ordering, not by this manager.
    /// Each layer is published independently; `LedCoordinator` picks
    /// the highest-priority filled one. Only the active scene's local
    /// tier and the global tier are promoted here; an active temp keeps
    /// running through cross-layer loss (it is not paused), but a scene
    /// change drops the outgoing scene's temp.
    fn refresh_active_scene_effect(&mut self) {
        let local_winner = self.active_scene.and_then(|id| {
            self.scenes
                .get_mut(&id)
                .and_then(pick_winner_scene_for_layer)
        });
        let global_winner = pick_winner_scene_for_layer(&mut self.global_state);

        if self.applied_local != local_winner {
            self.applied_local = local_winner;
            self.coordinator.publish(Layer::LocalScene, local_winner);
        }
        if self.applied_global != global_winner {
            self.applied_global = global_winner;
            self.coordinator
                .publish(Layer::GlobalAmbient, global_winner);
        }
    }

    fn emit(&self, instance_id: InstanceId, request_id: LedRequestId, status: LedRequestStatus) {
        if self
            .status_tx
            .send(LedRequestStatusEvent {
                instance_id,
                request_id,
                status,
            })
            .is_err()
        {
            warn!("led status receiver gone; dropping status update");
        }
    }
}

/// What the given layer wants to show on the strip, promoting from the
/// temp queue if the active slot is empty. Returns the `LedScene` to
/// publish, or `None` if the layer has nothing.
///
/// Promotion sets `until` to `Instant::now() + duration` once and never
/// adjusts it. This runs only for the active scene's local tier and the
/// global tier, so a temp's clock starts when it reaches the active slot
/// — for a local temp, only while its scene is active.
fn pick_winner_scene_for_layer(state: &mut SceneEffectState) -> Option<LedScene> {
    if state.active_temp.is_none()
        && let Some(entry) = state.temp_queue.pop_front()
    {
        let until = Instant::now() + entry.duration;
        state.active_temp = Some(RunningTemp { entry, until });
    }

    state
        .active_temp
        .as_ref()
        .map(|t| t.entry.scene)
        .or_else(|| state.endless.as_ref().map(|entry| entry.scene))
}

/// Every widget id that currently owns any effect in this state — the
/// endless slot, the running temp, or a queued temp.
fn state_widget_ids(state: &SceneEffectState) -> Vec<InstanceId> {
    let mut ids = Vec::new();
    if let Some(entry) = &state.endless {
        ids.push(entry.instance_id.clone());
    }
    if let Some(active) = &state.active_temp {
        ids.push(active.entry.instance_id.clone());
    }
    for entry in &state.temp_queue {
        ids.push(entry.instance_id.clone());
    }
    ids
}

/// Remove every entry from a state that matches the given predicate,
/// collecting their `(instance_id, request_id)` into `superseded`.
fn sweep_state(
    state: &mut SceneEffectState,
    matches: &impl Fn(&str, LedRequestId) -> bool,
    superseded: &mut Vec<(InstanceId, LedRequestId)>,
) {
    if let Some(active) = state.active_temp.take() {
        if matches(&active.entry.instance_id, active.entry.request_id) {
            superseded.push((active.entry.instance_id, active.entry.request_id));
        } else {
            state.active_temp = Some(active);
        }
    }

    let kept_queue: VecDeque<_> = std::mem::take(&mut state.temp_queue)
        .into_iter()
        .filter_map(|entry| {
            if matches(&entry.instance_id, entry.request_id) {
                superseded.push((entry.instance_id, entry.request_id));
                None
            } else {
                Some(entry)
            }
        })
        .collect();
    state.temp_queue = kept_queue;

    if let Some(entry) = state.endless.take() {
        if matches(&entry.instance_id, entry.request_id) {
            superseded.push((entry.instance_id, entry.request_id));
        } else {
            state.endless = Some(entry);
        }
    }
}

/// Place `new_entry` into the single endless slot, emitting `Superseded`
/// for any prior holder (same widget or different). Last-write-wins:
/// the displaced request does not come back, just like a `stop_led` on
/// it would have done.
fn apply_endless(
    slot: &mut Option<EndlessEntry>,
    new_entry: EndlessEntry,
    emissions: &mut Vec<(InstanceId, LedRequestId, LedRequestStatus)>,
) {
    if let Some(prev) = slot.take() {
        emissions.push((
            prev.instance_id,
            prev.request_id,
            LedRequestStatus::Superseded,
        ));
    }
    *slot = Some(new_entry);
}

/// If the state's `active_temp` is `Running` with `until <= now`, take it
/// and return the request's identity for `Expired` emission.
fn expire_state_if_due(
    state: &mut SceneEffectState,
    now: Instant,
) -> Option<(InstanceId, LedRequestId)> {
    let active = state.active_temp.take()?;
    if active.until <= now {
        Some((active.entry.instance_id, active.entry.request_id))
    } else {
        state.active_temp = Some(active);
        None
    }
}

fn build_scene(
    effect: ProtoLedEffect,
    color: RgbColor,
    period_ms: u32,
    duration_ms: Option<u64>,
) -> LedScene {
    LedScene {
        effect: proto_to_hw_effect(effect, color),
        period: (period_ms > 0).then(|| Duration::from_millis(u64::from(period_ms))),
        duration: duration_ms.map(Duration::from_millis),
    }
}

fn proto_to_hw_effect(effect: ProtoLedEffect, color: RgbColor) -> HwLedEffect {
    let rgb = Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    };
    match effect {
        ProtoLedEffect::Chase => HwLedEffect::Chase(rgb),
        ProtoLedEffect::KnightRider => HwLedEffect::KnightRider(rgb),
        ProtoLedEffect::Scan => HwLedEffect::Scan(rgb),
        ProtoLedEffect::Snake => HwLedEffect::Snake(rgb),
        ProtoLedEffect::Breathe => HwLedEffect::Breathe(rgb),
        ProtoLedEffect::Solid => HwLedEffect::Solid(rgb),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::led_coordinator::spawn_led_coordinator;

    struct Harness {
        manager: LedSceneManager,
        status_rx: mpsc::UnboundedReceiver<LedRequestStatusEvent>,
        _led_cmd_rx: mpsc::Receiver<bmc_led::data::LedCommand>,
        _runtime: tokio::runtime::Runtime,
    }

    impl Harness {
        fn new() -> Self {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("BUG: test runtime must build");
            let (led_cmd_tx, led_cmd_rx) = mpsc::channel(16);
            let coordinator = {
                let _guard = runtime.enter();
                spawn_led_coordinator(led_cmd_tx)
            };
            let (status_tx, status_rx) = mpsc::unbounded_channel();
            let manager = LedSceneManager::new(coordinator, status_tx);
            Self {
                manager,
                status_rx,
                _led_cmd_rx: led_cmd_rx,
                _runtime: runtime,
            }
        }

        fn drain_statuses(&mut self) -> Vec<LedRequestStatusEvent> {
            let mut statuses = Vec::new();
            while let Ok(status) = self.status_rx.try_recv() {
                statuses.push(status);
            }
            statuses
        }
    }

    fn rgb(r: u8, g: u8, b: u8) -> RgbColor {
        RgbColor { r, g, b }
    }

    fn instance_id(tag: &str) -> InstanceId {
        format!("widget-{tag}")
    }

    fn scene_id() -> SceneId {
        SceneId::from(Uuid::new_v4())
    }

    fn snapshot_of(pairs: &[(&InstanceId, SceneId)]) -> WidgetSceneMap {
        pairs
            .iter()
            .map(|(id, scene)| ((*id).clone(), *scene))
            .collect()
    }

    fn active_temp_request_id(h: &Harness, scene: SceneId) -> Option<LedRequestId> {
        h.manager
            .scenes
            .get(&scene)?
            .active_temp
            .as_ref()
            .map(|t| t.entry.request_id)
    }

    fn force_running_temp_until(h: &mut Harness, scene: SceneId, until: Instant) {
        let state = h
            .manager
            .scenes
            .get_mut(&scene)
            .expect("BUG: scene state must exist");
        let active = state
            .active_temp
            .as_mut()
            .expect("BUG: active temp must exist");
        active.until = until;
    }

    #[test]
    fn on_active_expiry_completes_running_temp_only_when_due() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("due-check");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_temporary(
            widget.clone(),
            11,
            ProtoLedEffect::Breathe,
            rgb(1, 1, 1),
            0,
            30_000,
            LedScope::Local,
        );
        h.drain_statuses();

        force_running_temp_until(&mut h, scene, Instant::now() + Duration::from_secs(30));
        h.manager.on_active_expiry();
        assert!(h.drain_statuses().is_empty());
        assert_eq!(active_temp_request_id(&h, scene), Some(11));

        force_running_temp_until(&mut h, scene, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget);
        assert_eq!(statuses[0].request_id, 11);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
        assert_eq!(active_temp_request_id(&h, scene), None);
    }

    #[test]
    fn queued_temporary_starts_after_active_temporary_completes() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("queue-start");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_temporary(
            widget.clone(),
            21,
            ProtoLedEffect::Solid,
            rgb(2, 0, 0),
            0,
            1_000,
            LedScope::Local,
        );
        h.manager.on_temporary(
            widget.clone(),
            22,
            ProtoLedEffect::Solid,
            rgb(3, 0, 0),
            0,
            2_000,
            LedScope::Local,
        );
        h.drain_statuses();
        assert_eq!(active_temp_request_id(&h, scene), Some(21));

        force_running_temp_until(&mut h, scene, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].request_id, 21);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
        assert_eq!(active_temp_request_id(&h, scene), Some(22));
    }

    #[test]
    fn endless_fallback_becomes_active_when_temporary_queue_drains() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("fallback");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            31,
            ProtoLedEffect::Solid,
            rgb(9, 9, 9),
            0,
            LedScope::Local,
        );
        h.manager.on_temporary(
            widget.clone(),
            32,
            ProtoLedEffect::Solid,
            rgb(4, 0, 0),
            0,
            1_000,
            LedScope::Local,
        );
        h.manager.on_temporary(
            widget.clone(),
            33,
            ProtoLedEffect::Solid,
            rgb(5, 0, 0),
            0,
            1_000,
            LedScope::Local,
        );
        h.drain_statuses();
        assert_eq!(active_temp_request_id(&h, scene), Some(32));

        force_running_temp_until(&mut h, scene, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();
        h.drain_statuses();
        assert_eq!(active_temp_request_id(&h, scene), Some(33));

        force_running_temp_until(&mut h, scene, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].request_id, 33);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
        assert_eq!(active_temp_request_id(&h, scene), None);
        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(9, 9, 9), 0, None))
        );
    }

    #[test]
    fn endless_in_inactive_scene_is_accepted_but_not_applied() {
        let mut h = Harness::new();
        let scene_active = scene_id();
        let scene_inactive = scene_id();
        let active_widget = instance_id("active");
        let inactive_widget = instance_id("inactive");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&active_widget, scene_active)]));
        h.manager.on_scene_changed(scene_active);
        h.manager
            .widget_to_scene
            .insert(inactive_widget.clone(), scene_inactive);

        let baseline_local = h.manager.applied_local;
        let baseline_global = h.manager.applied_global;
        h.manager.on_endless(
            inactive_widget.clone(),
            10,
            ProtoLedEffect::Solid,
            rgb(1, 2, 3),
            0,
            LedScope::Local,
        );

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, inactive_widget);
        assert_eq!(statuses[0].request_id, 10);
        assert_eq!(statuses[0].status, LedRequestStatus::Accepted);
        assert_eq!(h.manager.applied_local, baseline_local);
        assert_eq!(h.manager.applied_global, baseline_global);
    }

    #[test]
    fn switching_scene_applies_latest_endless_for_new_scene() {
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget_a, scene_a)]));
        h.manager.on_scene_changed(scene_a);
        h.manager.widget_to_scene.insert(widget_b.clone(), scene_b);

        h.manager.on_endless(
            widget_b.clone(),
            1,
            ProtoLedEffect::Solid,
            rgb(10, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            widget_b.clone(),
            2,
            ProtoLedEffect::Solid,
            rgb(20, 0, 0),
            0,
            LedScope::Local,
        );

        h.manager.on_scene_changed(scene_b);

        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(20, 0, 0), 0, None))
        );
    }

    #[test]
    fn temp_dropped_on_scene_change_emits_expired() {
        // A local temp never ticks off-screen: navigating away from its
        // scene drops the in-flight temp and emits `Expired`. It does
        // not resume, and it no longer contributes a deadline.
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget_a, scene_a)]));
        h.manager.on_scene_changed(scene_a);

        h.manager.on_temporary(
            widget_a.clone(),
            11,
            ProtoLedEffect::Breathe,
            rgb(1, 1, 1),
            0,
            5_000,
            LedScope::Local,
        );
        h.drain_statuses();
        assert_eq!(active_temp_request_id(&h, scene_a), Some(11));
        assert!(h.manager.active_deadline().is_some());

        h.manager.on_scene_changed(scene_b);

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_a);
        assert_eq!(statuses[0].request_id, 11);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
        assert_eq!(active_temp_request_id(&h, scene_a), None);
        assert!(h.manager.active_deadline().is_none());
    }

    #[test]
    fn inactive_scene_temp_queue_does_not_advance() {
        // A local temp submitted for a scene that is not active waits in
        // its queue: no clock starts, so it adds no deadline and nothing
        // is promoted. It promotes only once its scene becomes active.
        let mut h = Harness::new();
        let scene_active = scene_id();
        let scene_inactive = scene_id();
        let active_widget = instance_id("active");
        let inactive_widget = instance_id("inactive");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&active_widget, scene_active)]));
        h.manager.on_scene_changed(scene_active);
        h.manager
            .widget_to_scene
            .insert(inactive_widget.clone(), scene_inactive);

        h.manager.on_temporary(
            inactive_widget.clone(),
            55,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            1_000,
            LedScope::Local,
        );
        h.drain_statuses();
        assert!(h.manager.active_deadline().is_none());
        assert_eq!(active_temp_request_id(&h, scene_inactive), None);

        // Activating the scene promotes the queued temp and starts its
        // clock.
        h.manager.on_scene_changed(scene_inactive);
        assert_eq!(active_temp_request_id(&h, scene_inactive), Some(55));
        assert!(h.manager.active_deadline().is_some());
    }

    #[test]
    fn widget_stop_only_cancels_own_requests() {
        // A on Local, B on Global so they coexist on separate tiers.
        // Stopping A must cancel only A's endless — B's slot stays.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget_a, scene), (&widget_b, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget_a.clone(),
            100,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            widget_b.clone(),
            200,
            ProtoLedEffect::Solid,
            rgb(0, 1, 0),
            0,
            LedScope::Global,
        );
        h.drain_statuses();

        h.manager.on_stop(&widget_a, LED_REQUEST_ID_ALL);

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_a);
        assert_eq!(statuses[0].request_id, 100);
        assert_eq!(statuses[0].status, LedRequestStatus::Superseded);
    }

    fn global_active_temp_request_id(h: &Harness) -> Option<LedRequestId> {
        h.manager
            .global_state
            .active_temp
            .as_ref()
            .map(|t| t.entry.request_id)
    }

    fn force_global_running_until(h: &mut Harness, until: Instant) {
        let active = h
            .manager
            .global_state
            .active_temp
            .as_mut()
            .expect("BUG: global active temp must exist");
        active.until = until;
    }

    #[test]
    fn local_and_global_temps_publish_to_separate_layers_and_tick_independently() {
        // Cross-tier priority is owned by `LedCoordinator`'s layer
        // ordering, not by `LedSceneManager`. We publish the local
        // winner and the global winner separately; the coordinator
        // picks the higher-priority one. Both temps tick on logical
        // time — nothing pauses on layer loss.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("dual");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_temporary(
            widget.clone(),
            500,
            ProtoLedEffect::KnightRider,
            rgb(8, 0, 0),
            500,
            5_000,
            LedScope::Global,
        );
        h.drain_statuses();
        assert_eq!(global_active_temp_request_id(&h), Some(500));
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(
                ProtoLedEffect::KnightRider,
                rgb(8, 0, 0),
                500,
                Some(5_000)
            ))
        );
        assert_eq!(h.manager.applied_local, None);

        h.manager.on_temporary(
            widget.clone(),
            600,
            ProtoLedEffect::Solid,
            rgb(0, 9, 0),
            0,
            1_000,
            LedScope::Local,
        );
        h.drain_statuses();
        // Both layers carry their respective active temp; the
        // coordinator picks LocalScene over GlobalAmbient. No
        // Superseded fires because both requests are alive.
        assert_eq!(active_temp_request_id(&h, scene), Some(600));
        assert_eq!(global_active_temp_request_id(&h), Some(500));
        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(
                ProtoLedEffect::Solid,
                rgb(0, 9, 0),
                0,
                Some(1_000)
            ))
        );
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(
                ProtoLedEffect::KnightRider,
                rgb(8, 0, 0),
                500,
                Some(5_000)
            ))
        );

        // Local completes; the global temp is still ticking and still
        // owns the GlobalAmbient layer.
        force_running_temp_until(&mut h, scene, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();
        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].request_id, 600);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
        assert_eq!(global_active_temp_request_id(&h), Some(500));
        assert_eq!(h.manager.applied_local, None);

        // Global eventually completes on its own logical-time schedule
        // under its original id.
        force_global_running_until(&mut h, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();
        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].request_id, 500);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
    }

    #[test]
    fn local_endless_and_global_endless_publish_to_their_own_layers() {
        // Both endlesses are live simultaneously, on separate layers.
        // No `Superseded` event fires across tiers — that's just
        // layer-priority arbitration in `LedCoordinator`. Stopping the
        // local one leaves the global one untouched on its own layer.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("layered");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            700,
            ProtoLedEffect::Solid,
            rgb(7, 0, 0),
            0,
            LedScope::Global,
        );
        h.drain_statuses();
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(7, 0, 0), 0, None))
        );
        assert_eq!(h.manager.applied_local, None);

        h.manager.on_endless(
            widget.clone(),
            800,
            ProtoLedEffect::Solid,
            rgb(0, 0, 7),
            0,
            LedScope::Local,
        );
        let statuses = h.drain_statuses();
        assert!(
            !statuses
                .iter()
                .any(|s| s.request_id == 700 && s.status == LedRequestStatus::Superseded),
            "global endless must not receive Superseded when a local one lands: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 0, 7), 0, None))
        );
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(7, 0, 0), 0, None))
        );

        h.manager.on_stop(&widget, 800);
        h.drain_statuses();
        assert_eq!(h.manager.applied_local, None);
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(7, 0, 0), 0, None))
        );
    }

    #[test]
    fn endless_replace_supersedes_with_no_fallback() {
        // Two endlesses on the same tier from the same widget: the
        // first gets `Superseded` and is gone. Stopping the second
        // clears the strip — no resurrection of the first.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("stack");

        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            901,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            LedScope::Global,
        );
        h.manager.on_endless(
            widget.clone(),
            902,
            ProtoLedEffect::Solid,
            rgb(2, 0, 0),
            0,
            LedScope::Global,
        );

        let statuses = h.drain_statuses();
        assert!(
            statuses
                .iter()
                .any(|s| s.request_id == 901 && s.status == LedRequestStatus::Superseded),
            "replacement must Supersede the prior endless: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(2, 0, 0), 0, None))
        );

        h.manager.on_stop(&widget, 902);
        let statuses = h.drain_statuses();
        assert!(
            statuses
                .iter()
                .any(|s| s.request_id == 902 && s.status == LedRequestStatus::Superseded),
            "stopping 902 must emit Superseded(902): {statuses:?}"
        );
        assert_eq!(h.manager.applied_global, None);
    }

    #[test]
    fn cross_widget_endless_last_writer_wins_and_supersedes_prior() {
        // Widget B's endless lands on a tier already held by widget A.
        // A is displaced with `Superseded` (not paused — there is no
        // suspend/resume any more); the strip shows B. Stopping B
        // clears the tier; A does not come back.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget_a.clone(),
            10,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            LedScope::Global,
        );
        h.drain_statuses();

        h.manager.on_endless(
            widget_b.clone(),
            20,
            ProtoLedEffect::Solid,
            rgb(0, 2, 0),
            0,
            LedScope::Global,
        );

        let statuses = h.drain_statuses();
        assert!(
            statuses.iter().any(|s| s.instance_id == widget_a
                && s.request_id == 10
                && s.status == LedRequestStatus::Superseded),
            "A's endless must be Superseded when B claims the tier: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 2, 0), 0, None))
        );

        h.manager.on_stop(&widget_b, 20);

        let statuses = h.drain_statuses();
        assert!(
            statuses
                .iter()
                .any(|s| s.request_id == 20 && s.status == LedRequestStatus::Superseded),
            "B's endless must be Superseded when stopped: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_global, None,
            "A must not resurrect: {statuses:?}"
        );
    }

    #[test]
    fn scene_change_drops_local_layer_keeps_global() {
        // Scene A has a local temp and the widget also published a
        // global endless. Switching to scene B (no local content) drops
        // the LocalScene layer to None and reports `Expired` for the
        // dropped temp; the GlobalAmbient layer stays untouched, so
        // `LedCoordinator` then picks the global one.
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget_a, scene_a)]));
        h.manager.on_scene_changed(scene_a);
        h.manager.widget_to_scene.insert(widget_b.clone(), scene_b);

        h.manager.on_temporary(
            widget_a.clone(),
            41,
            ProtoLedEffect::Solid,
            rgb(4, 0, 0),
            0,
            5_000,
            LedScope::Local,
        );
        h.manager.on_endless(
            widget_a.clone(),
            42,
            ProtoLedEffect::Solid,
            rgb(0, 4, 0),
            0,
            LedScope::Global,
        );
        h.drain_statuses();
        assert_eq!(active_temp_request_id(&h, scene_a), Some(41));
        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(
                ProtoLedEffect::Solid,
                rgb(4, 0, 0),
                0,
                Some(5_000)
            ))
        );
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 4, 0), 0, None))
        );

        h.manager.on_scene_changed(scene_b);
        // LocalScene goes None (scene B has nothing local) and the
        // dropped temp reports `Expired`; GlobalAmbient is unchanged.
        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_a);
        assert_eq!(statuses[0].request_id, 41);
        assert_eq!(statuses[0].status, LedRequestStatus::Expired);
        assert_eq!(h.manager.applied_local, None);
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 4, 0), 0, None))
        );
        assert_eq!(active_temp_request_id(&h, scene_a), None);
    }

    #[test]
    fn stop_led_zero_sweeps_both_tiers() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("sweep");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            201,
            ProtoLedEffect::Solid,
            rgb(2, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            widget.clone(),
            202,
            ProtoLedEffect::Solid,
            rgb(0, 2, 0),
            0,
            LedScope::Global,
        );
        h.drain_statuses();

        h.manager.on_stop(&widget, LED_REQUEST_ID_ALL);

        let statuses = h.drain_statuses();
        let mut superseded_ids: Vec<_> = statuses
            .iter()
            .filter(|s| s.status == LedRequestStatus::Superseded)
            .map(|s| s.request_id)
            .collect();
        superseded_ids.sort_unstable();
        assert_eq!(superseded_ids, vec![201, 202]);
        // Both layers cleared.
        assert_eq!(h.manager.applied_local, None);
        assert_eq!(h.manager.applied_global, None);
    }

    #[test]
    fn widget_disconnect_sweeps_both_tiers() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("disconnect-both");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget, scene)]));
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            301,
            ProtoLedEffect::Solid,
            rgb(3, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            widget.clone(),
            302,
            ProtoLedEffect::Solid,
            rgb(0, 3, 0),
            0,
            LedScope::Global,
        );
        h.drain_statuses();

        h.manager.reconcile_connected(&BTreeSet::new());

        let statuses = h.drain_statuses();
        let mut superseded_ids: Vec<_> = statuses
            .iter()
            .filter(|s| s.status == LedRequestStatus::Superseded)
            .map(|s| s.request_id)
            .collect();
        superseded_ids.sort_unstable();
        assert_eq!(superseded_ids, vec![301, 302]);
        assert_eq!(h.manager.applied_local, None);
        assert_eq!(h.manager.applied_global, None);
    }

    #[test]
    fn widget_disconnect_removes_effects_from_all_scenes() {
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager
            .on_config_snapshot(snapshot_of(&[(&widget_a, scene_a)]));
        h.manager.on_scene_changed(scene_a);
        h.manager.widget_to_scene.insert(widget_b.clone(), scene_b);

        h.manager.on_endless(
            widget_a.clone(),
            10,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            widget_b.clone(),
            20,
            ProtoLedEffect::Solid,
            rgb(0, 1, 0),
            0,
            LedScope::Local,
        );
        h.drain_statuses();

        // widget_b disconnects; widget_a stays connected.
        h.manager
            .reconcile_connected(&BTreeSet::from([widget_a.clone()]));

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_b);
        assert_eq!(statuses[0].request_id, 20);
        assert_eq!(statuses[0].status, LedRequestStatus::Superseded);
    }

    #[test]
    fn config_snapshot_adds_widget_to_scene_mapping() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("snap");

        let mut snapshot = WidgetSceneMap::new();
        snapshot.insert(widget.clone(), scene);
        h.manager.on_config_snapshot(snapshot);

        assert_eq!(h.manager.widget_to_scene.get(&widget), Some(&scene));
    }

    #[test]
    fn config_snapshot_dropping_widget_supersedes_its_requests() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("dropped");

        // Seed: widget is in config and active scene; submit an endless.
        let mut initial = WidgetSceneMap::new();
        initial.insert(widget.clone(), scene);
        h.manager.on_config_snapshot(initial);
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            77,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            LedScope::Local,
        );
        h.drain_statuses();

        // New snapshot: widget gone from config.
        h.manager.on_config_snapshot(WidgetSceneMap::new());

        let statuses = h.drain_statuses();
        let superseded: Vec<_> = statuses
            .iter()
            .filter(|s| s.status == LedRequestStatus::Superseded)
            .map(|s| (s.instance_id.clone(), s.request_id))
            .collect();
        assert_eq!(superseded, vec![(widget.clone(), 77)]);
        assert!(!h.manager.widget_to_scene.contains_key(&widget));
        assert_eq!(h.manager.applied_local, None);
    }

    #[test]
    fn config_snapshot_unchanged_does_not_sweep_existing_effects() {
        // Restart scenario: snapshot stays the same across a respawn.
        // The widget's effects must keep playing.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("restarter");

        let mut snapshot = WidgetSceneMap::new();
        snapshot.insert(widget.clone(), scene);
        h.manager.on_config_snapshot(snapshot.clone());
        h.manager.on_scene_changed(scene);
        h.manager.on_endless(
            widget.clone(),
            88,
            ProtoLedEffect::Solid,
            rgb(0, 1, 0),
            0,
            LedScope::Local,
        );
        h.drain_statuses();

        // Re-deliver the same snapshot — simulating a save that touched
        // an unrelated concern but still emitted a snapshot.
        h.manager.on_config_snapshot(snapshot);

        let statuses = h.drain_statuses();
        assert!(
            statuses.is_empty(),
            "no Superseded must fire on identical snapshot, got {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 1, 0), 0, None))
        );
    }

    #[test]
    fn widget_disconnect_keeps_mapping_so_respawned_widget_reroutes() {
        // A widget process restart (e.g. size change) drops it from the
        // connected set. Reconcile sweeps its effects but leaves the
        // config-derived scene mapping intact, so the respawned process
        // can re-issue LED requests and they route to the same scene.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("resize-survivor");

        let mut snapshot = WidgetSceneMap::new();
        snapshot.insert(widget.clone(), scene);
        h.manager.on_config_snapshot(snapshot);
        h.manager.on_scene_changed(scene);

        h.manager.on_endless(
            widget.clone(),
            91,
            ProtoLedEffect::Solid,
            rgb(7, 7, 7),
            0,
            LedScope::Local,
        );
        h.drain_statuses();

        // Wayland disconnect (the widget's process is being respawned):
        // it drops out of the connected set. The mapping must survive;
        // effects get swept since the respawned process won't remember them.
        h.manager.reconcile_connected(&BTreeSet::new());
        assert_eq!(h.manager.widget_to_scene.get(&widget), Some(&scene));
        assert_eq!(h.manager.applied_local, None);

        // Respawned widget re-issues its endless — must route to the
        // scene's slot via the preserved mapping.
        h.manager.on_endless(
            widget.clone(),
            92,
            ProtoLedEffect::Solid,
            rgb(7, 7, 7),
            0,
            LedScope::Local,
        );
        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(7, 7, 7), 0, None))
        );
    }
}

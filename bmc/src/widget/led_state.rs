// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use bmc_led::data::{LedEffect as HwLedEffect, LedScene, Rgb};
use bmc_widget_protocol::{
    LED_REQUEST_ID_ALL, LedEffect as ProtoLedEffect, LedRequestId, LedRequestStatus, LedScope,
    RgbColor,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::compositor::{InstanceId, LedRequestStatusEvent};
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
    /// mutated — temps tick in logical time, so the wall-clock
    /// deadline (`RunningTemp::until`) is derived once on promotion
    /// from the queue.
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

#[derive(Debug)]
struct PendingEndless {
    entry: EndlessEntry,
    seq: u64,
}

#[derive(Debug)]
struct PendingTemp {
    entry: TempEntry,
    seq: u64,
}

#[derive(Debug, Default)]
struct PendingState {
    /// Single parked endless slot per widget — a parked widget can
    /// only have one outstanding endless; later requests supersede the
    /// earlier one with `Superseded` even before the scene activates.
    endless: Option<PendingEndless>,
    temp_queue: VecDeque<PendingTemp>,
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
    pending: HashMap<InstanceId, PendingState>,
    pending_seq: u64,
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
            pending: HashMap::new(),
            pending_seq: 0,
        }
    }

    pub(crate) fn on_scene_changed(&mut self, scene_id: SceneId, widget_ids: Vec<InstanceId>) {
        self.active_scene = Some(scene_id);
        self.scenes.entry(scene_id).or_default();

        // Drain parked requests before `widget_ids` is consumed by the
        // mapping loop below. Sort by seq so cross-widget ordering matches
        // the order callers issued the requests in — the latest endless
        // wins the single slot.
        let mut parked_endless: Vec<PendingEndless> = Vec::new();
        let mut parked_temp: Vec<PendingTemp> = Vec::new();
        for widget_id in &widget_ids {
            if let Some(parked) = self.pending.remove(widget_id) {
                parked_endless.extend(parked.endless);
                parked_temp.extend(parked.temp_queue);
            }
        }
        parked_endless.sort_by_key(|p| p.seq);
        parked_temp.sort_by_key(|p| p.seq);

        let scene_state = self
            .scenes
            .get_mut(&scene_id)
            .expect("BUG: scene state was just initialised");
        let mut emissions: Vec<(InstanceId, LedRequestId, LedRequestStatus)> = Vec::new();
        for parked in parked_endless {
            apply_endless(&mut scene_state.endless, parked.entry, &mut emissions);
        }
        scene_state
            .temp_queue
            .extend(parked_temp.into_iter().map(|p| p.entry));

        for widget_id in widget_ids {
            self.widget_to_scene.insert(widget_id, scene_id);
        }
        for (id, rid, status) in emissions {
            self.emit(id, rid, status);
        }
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
                    self.pending_seq += 1;
                    let seq = self.pending_seq;
                    self.pending
                        .entry(instance_id.clone())
                        .or_default()
                        .temp_queue
                        .push_back(PendingTemp { entry, seq });
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
                    self.pending_seq += 1;
                    let seq = self.pending_seq;
                    let pending = self.pending.entry(instance_id.clone()).or_default();
                    apply_pending_endless(&mut pending.endless, new_entry, seq, &mut emissions);
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

        if let Some(state) = self.pending.get_mut(instance_id) {
            sweep_pending(state, &matches, &mut superseded);
            if state.endless.is_none() && state.temp_queue.is_empty() {
                self.pending.remove(instance_id);
            }
        }

        for (id, rid) in superseded {
            self.emit(id, rid, LedRequestStatus::Superseded);
        }

        self.refresh_active_scene_effect();
    }

    pub(crate) fn on_widget_disconnected(&mut self, instance_id: &str) {
        self.widget_to_scene
            .retain(|widget_id, _| widget_id != instance_id);
        self.on_stop(instance_id, LED_REQUEST_ID_ALL);
        self.evict_empty_scenes();
    }

    /// Drop scene entries that no widget still references and that hold
    /// no pending state. When a scene is removed from the cycling
    /// configuration, the compositor kills every widget on it; each
    /// `on_widget_disconnected` clears that widget's `widget_to_scene`
    /// entry and sweeps its requests, so the scene becomes orphaned and
    /// empty. Without eviction, `self.scenes` would accumulate dead
    /// entries forever across config reloads.
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

        for state in self
            .scenes
            .values_mut()
            .chain(std::iter::once(&mut self.global_state))
        {
            if let Some(done) = expire_state_if_due(state, now) {
                expired.push(done);
            }
        }

        if expired.is_empty() {
            return;
        }

        for (instance_id, request_id) in expired {
            self.emit(instance_id, request_id, LedRequestStatus::Expired);
        }
        self.refresh_active_scene_effect();
    }

    /// Earliest wall-clock deadline across every scene's and the
    /// global tier's `active_temp`. Temps tick in logical time
    /// regardless of which scene is currently active, so the deadline
    /// surface has to cover everything — otherwise a temp in an
    /// inactive scene would never fire its `Expired`.
    pub(crate) fn active_deadline(&self) -> Option<Instant> {
        self.scenes
            .values()
            .chain(std::iter::once(&self.global_state))
            .filter_map(|state| state.active_temp.as_ref().map(|t| t.until))
            .min()
    }

    /// Publish the winning scene for each widget layer.
    ///
    /// Cross-tier priority (local-wins-over-global) is owned by
    /// `LedCoordinator` via its layer ordering, not by this manager.
    /// Each layer is published independently; `LedCoordinator` picks
    /// the highest-priority filled one. Temps tick in logical time and
    /// do not pause on scene change or cross-layer loss — they
    /// complete on schedule regardless of whether they were ever on
    /// the strip.
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
        let _ = self.status_tx.send(LedRequestStatusEvent {
            instance_id,
            request_id,
            status,
        });
    }
}

/// What the given layer wants to show on the strip, promoting from the
/// temp queue if the active slot is empty. Returns the `LedScene` to
/// publish, or `None` if the layer has nothing.
///
/// Temps tick in logical time once promoted: `until` is set from
/// `Instant::now() + duration` exactly once and never adjusted. Scene
/// activity does not affect lifecycle — `Expired` will fire on the
/// schedule the widget asked for whether or not the strip ever showed
/// the effect.
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

/// Pending endless slots are keyed per-widget, so any push from the
/// same widget is a self-replace. Drop the prior slot (if any) with
/// `Superseded` and store the new entry under the latest seq.
fn apply_pending_endless(
    slot: &mut Option<PendingEndless>,
    new_entry: EndlessEntry,
    seq: u64,
    emissions: &mut Vec<(InstanceId, LedRequestId, LedRequestStatus)>,
) {
    if let Some(prev) = slot.take() {
        emissions.push((
            prev.entry.instance_id,
            prev.entry.request_id,
            LedRequestStatus::Superseded,
        ));
    }
    *slot = Some(PendingEndless {
        entry: new_entry,
        seq,
    });
}

fn sweep_pending(
    state: &mut PendingState,
    matches: &impl Fn(&str, LedRequestId) -> bool,
    superseded: &mut Vec<(InstanceId, LedRequestId)>,
) {
    if let Some(parked) = state.endless.take() {
        if matches(&parked.entry.instance_id, parked.entry.request_id) {
            superseded.push((parked.entry.instance_id, parked.entry.request_id));
        } else {
            state.endless = Some(parked);
        }
    }
    state.temp_queue.retain(|p| {
        if matches(&p.entry.instance_id, p.entry.request_id) {
            superseded.push((p.entry.instance_id.clone(), p.entry.request_id));
            false
        } else {
            true
        }
    });
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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
            .on_scene_changed(scene_active, vec![active_widget.clone()]);
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

        h.manager.on_scene_changed(scene_a, vec![widget_a.clone()]);
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

        h.manager.on_scene_changed(scene_b, vec![widget_b.clone()]);

        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(20, 0, 0), 0, None))
        );
    }

    #[test]
    fn temp_keeps_ticking_through_scene_change() {
        // Pause-on-scene-change is gone: a running temp ticks in logical
        // time regardless of whether the strip is showing it. Its
        // deadline stays scheduled through a scene change.
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager.on_scene_changed(scene_a, vec![widget_a.clone()]);
        h.manager.widget_to_scene.insert(widget_b.clone(), scene_b);

        h.manager.on_temporary(
            widget_a.clone(),
            11,
            ProtoLedEffect::Breathe,
            rgb(1, 1, 1),
            0,
            5_000,
            LedScope::Local,
        );
        let deadline_before = h
            .manager
            .active_deadline()
            .expect("BUG: temp should have set a deadline");

        h.manager.on_scene_changed(scene_b, vec![widget_b.clone()]);
        let deadline_after = h
            .manager
            .active_deadline()
            .expect("BUG: scene change must not clear the temp deadline");

        assert_eq!(
            deadline_before, deadline_after,
            "scene change must not move the deadline (no pause): before={deadline_before:?} after={deadline_after:?}"
        );
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
            .on_scene_changed(scene, vec![widget_a.clone(), widget_b.clone()]);
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager
            .on_scene_changed(scene, vec![widget_a.clone(), widget_b.clone()]);
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
    fn disconnect_of_last_widget_on_inactive_scene_evicts_scene_entry() {
        // Two scenes, one widget on each. Activate scene_a so scene_b
        // is non-active. Disconnect widget_b → scene_b's entry should
        // be evicted from `scenes` so it doesn't leak across config
        // reloads.
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager.on_scene_changed(scene_a, vec![widget_a.clone()]);
        h.manager.widget_to_scene.insert(widget_b.clone(), scene_b);
        // Force scene_b to materialise in `scenes` with an entry.
        h.manager.on_endless(
            widget_b.clone(),
            1,
            ProtoLedEffect::Solid,
            rgb(0, 1, 0),
            0,
            LedScope::Local,
        );
        assert!(h.manager.scenes.contains_key(&scene_b));

        h.manager.on_widget_disconnected(&widget_b);

        assert!(
            !h.manager.scenes.contains_key(&scene_b),
            "scene_b must be evicted once its last widget disconnects"
        );
        // Active scene stays even if its widgets all left, so we can
        // keep driving its tier state.
        assert!(h.manager.scenes.contains_key(&scene_a));
    }

    #[test]
    fn disconnect_keeps_scene_when_other_widgets_remain() {
        // Two widgets on the same scene; disconnecting one must not
        // evict the scene because the other widget still references it.
        let mut h = Harness::new();
        let scene = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager
            .on_scene_changed(scene, vec![widget_a.clone(), widget_b.clone()]);
        h.manager.on_endless(
            widget_a.clone(),
            1,
            ProtoLedEffect::Solid,
            rgb(1, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_widget_disconnected(&widget_b);
        assert!(h.manager.scenes.contains_key(&scene));
    }

    #[test]
    fn scene_change_drops_local_layer_keeps_global() {
        // Scene A has a local temp and the widget also published a
        // global endless. Switching to scene B (no local content)
        // drops the LocalScene layer to None; the GlobalAmbient layer
        // stays untouched. `LedCoordinator` will then pick the global
        // one because LocalScene is empty.
        //
        // The local temp keeps ticking in logical time off-screen
        // because pause-on-scene-change is gone.
        let mut h = Harness::new();
        let scene_a = scene_id();
        let scene_b = scene_id();
        let widget_a = instance_id("a");
        let widget_b = instance_id("b");

        h.manager.on_scene_changed(scene_a, vec![widget_a.clone()]);
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

        h.manager.on_scene_changed(scene_b, vec![widget_b.clone()]);
        // LocalScene goes None (scene B has nothing local); GlobalAmbient
        // is unchanged. Scene A's temp is still ticking in its inactive
        // scene's state.
        assert_eq!(h.manager.applied_local, None);
        assert_eq!(
            h.manager.applied_global,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 4, 0), 0, None))
        );
        assert_eq!(active_temp_request_id(&h, scene_a), Some(41));
    }

    #[test]
    fn stop_led_zero_sweeps_both_tiers() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("sweep");

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager.on_scene_changed(scene, vec![widget.clone()]);
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

        h.manager.on_widget_disconnected(&widget);

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

        h.manager.on_scene_changed(scene_a, vec![widget_a.clone()]);
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

        h.manager.on_widget_disconnected(&widget_b);

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_b);
        assert_eq!(statuses[0].request_id, 20);
        assert_eq!(statuses[0].status, LedRequestStatus::Superseded);
    }

    #[test]
    fn local_endless_from_unmapped_widget_parks_then_drains_on_scene_activation() {
        let mut h = Harness::new();
        let scene = scene_id();
        let widget = instance_id("parked");

        h.manager.on_endless(
            widget.clone(),
            401,
            ProtoLedEffect::Solid,
            rgb(255, 0, 0),
            0,
            LedScope::Local,
        );

        let statuses = h.drain_statuses();
        assert!(
            statuses
                .iter()
                .any(|s| s.request_id == 401 && s.status == LedRequestStatus::Accepted),
            "parked request still emits Accepted"
        );

        h.manager.on_scene_changed(scene, vec![widget.clone()]);

        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(255, 0, 0), 0, None)),
        );
    }

    #[test]
    fn parked_endlesses_drain_in_seq_order_across_widgets() {
        let mut h = Harness::new();
        let scene = scene_id();
        let w1 = instance_id("race-a");
        let w2 = instance_id("race-b");

        h.manager.on_endless(
            w1.clone(),
            501,
            ProtoLedEffect::Solid,
            rgb(255, 0, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            w2.clone(),
            502,
            ProtoLedEffect::Solid,
            rgb(0, 255, 0),
            0,
            LedScope::Local,
        );
        h.manager.on_endless(
            w1.clone(),
            503,
            ProtoLedEffect::Solid,
            rgb(0, 0, 255),
            0,
            LedScope::Local,
        );

        h.manager
            .on_scene_changed(scene, vec![w1.clone(), w2.clone()]);

        assert_eq!(
            h.manager.applied_local,
            Some(build_scene(ProtoLedEffect::Solid, rgb(0, 0, 255), 0, None)),
        );
    }
}

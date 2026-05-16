// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use bmc_led::data::{LedEffect as HwLedEffect, LedScene, Rgb};
use bmc_widget_protocol::{
    LED_REQUEST_ID_ALL, LedEffect as ProtoLedEffect, LedRequestId, LedRequestStatus, LedScope,
    RgbColor,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::compositor::{InstanceId, WidgetRequestStatus};
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
    remaining: Duration,
}

#[derive(Debug)]
enum ActiveTemp {
    Running { entry: TempEntry, until: Instant },
    Paused { entry: TempEntry },
}

#[derive(Debug, Default)]
struct SceneEffectState {
    endless_stack: Vec<EndlessEntry>,
    temp_queue: VecDeque<TempEntry>,
    active_temp: Option<ActiveTemp>,
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
    endless_stack: Vec<PendingEndless>,
    temp_queue: VecDeque<PendingTemp>,
}

pub(crate) struct LedSceneManager {
    coordinator: LedCoordinatorHandle,
    status_tx: mpsc::UnboundedSender<WidgetRequestStatus>,
    widget_to_scene: HashMap<InstanceId, SceneId>,
    active_scene: Option<SceneId>,
    scenes: HashMap<SceneId, SceneEffectState>,
    global_state: SceneEffectState,
    applied_scene: LedScene,
    pending: HashMap<InstanceId, PendingState>,
    pending_seq: u64,
}

impl LedSceneManager {
    pub(crate) fn new(
        coordinator: LedCoordinatorHandle,
        status_tx: mpsc::UnboundedSender<WidgetRequestStatus>,
    ) -> Self {
        Self {
            coordinator,
            status_tx,
            widget_to_scene: HashMap::new(),
            active_scene: None,
            scenes: HashMap::new(),
            global_state: SceneEffectState::default(),
            applied_scene: LedScene {
                effect: HwLedEffect::None,
                period: None,
                duration: None,
            },
            pending: HashMap::new(),
            pending_seq: 0,
        }
    }

    pub(crate) fn on_scene_changed(&mut self, scene_id: SceneId, widget_ids: Vec<InstanceId>) {
        if let Some(previous_scene_id) = self.active_scene
            && previous_scene_id != scene_id
            && let Some(state) = self.scenes.get_mut(&previous_scene_id)
        {
            pause_state_temporary(state);
        }

        self.active_scene = Some(scene_id);
        self.scenes.entry(scene_id).or_default();

        // Drain parked requests before `widget_ids` is consumed by the
        // mapping loop below. Sort by seq so cross-widget ordering matches
        // the order callers issued the requests in.
        let mut parked_endless: Vec<PendingEndless> = Vec::new();
        let mut parked_temp: Vec<PendingTemp> = Vec::new();
        for widget_id in &widget_ids {
            if let Some(parked) = self.pending.remove(widget_id) {
                parked_endless.extend(parked.endless_stack);
                parked_temp.extend(parked.temp_queue);
            }
        }
        parked_endless.sort_by_key(|p| p.seq);
        parked_temp.sort_by_key(|p| p.seq);

        let scene_state = self
            .scenes
            .get_mut(&scene_id)
            .expect("BUG: scene state was just initialised");
        scene_state
            .endless_stack
            .extend(parked_endless.into_iter().map(|p| p.entry));
        scene_state
            .temp_queue
            .extend(parked_temp.into_iter().map(|p| p.entry));

        for widget_id in widget_ids {
            self.widget_to_scene.insert(widget_id, scene_id);
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
            remaining: Duration::from_millis(u64::from(duration_ms)),
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

        let previous_top: Option<(InstanceId, LedRequestId)> = match scope {
            LedScope::Local => {
                if let Some(scene_id) = self.widget_to_scene.get(&instance_id).copied() {
                    let stack = &mut self.scenes.entry(scene_id).or_default().endless_stack;
                    let prev = stack
                        .last()
                        .map(|entry| (entry.instance_id.clone(), entry.request_id));
                    stack.push(new_entry);
                    prev
                } else {
                    self.pending_seq += 1;
                    let seq = self.pending_seq;
                    let stack = &mut self
                        .pending
                        .entry(instance_id.clone())
                        .or_default()
                        .endless_stack;
                    let prev = stack
                        .last()
                        .map(|p| (p.entry.instance_id.clone(), p.entry.request_id));
                    stack.push(PendingEndless {
                        entry: new_entry,
                        seq,
                    });
                    prev
                }
            }
            LedScope::Global => {
                let stack = &mut self.global_state.endless_stack;
                let prev = stack
                    .last()
                    .map(|entry| (entry.instance_id.clone(), entry.request_id));
                stack.push(new_entry);
                prev
            }
        };

        if let Some((old_instance_id, old_request_id)) = previous_top {
            self.emit(
                old_instance_id,
                old_request_id,
                LedRequestStatus::Superseded,
            );
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
            if state.endless_stack.is_empty() && state.temp_queue.is_empty() {
                self.pending.remove(instance_id);
            }
        }

        for (superseded_instance, superseded_request) in superseded {
            self.emit(
                superseded_instance,
                superseded_request,
                LedRequestStatus::Superseded,
            );
        }

        self.refresh_active_scene_effect();
    }

    pub(crate) fn on_widget_disconnected(&mut self, instance_id: &str) {
        self.widget_to_scene
            .retain(|widget_id, _| widget_id != instance_id);
        self.on_stop(instance_id, LED_REQUEST_ID_ALL);
    }

    pub(crate) fn on_active_expiry(&mut self) {
        let now = Instant::now();
        let mut expired: Vec<(InstanceId, LedRequestId)> = Vec::new();

        if let Some(scene_id) = self.active_scene
            && let Some(state) = self.scenes.get_mut(&scene_id)
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
            self.emit(instance_id, request_id, LedRequestStatus::Completed);
        }
        self.refresh_active_scene_effect();
    }

    pub(crate) fn active_deadline(&self) -> Option<Instant> {
        let scene_deadline = self
            .active_scene
            .and_then(|id| self.scenes.get(&id))
            .and_then(|state| match state.active_temp.as_ref()? {
                ActiveTemp::Running { until, .. } => Some(*until),
                ActiveTemp::Paused { .. } => None,
            });
        let global_deadline = match self.global_state.active_temp.as_ref() {
            Some(ActiveTemp::Running { until, .. }) => Some(*until),
            Some(ActiveTemp::Paused { .. }) | None => None,
        };
        match (scene_deadline, global_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(d), None) | (None, Some(d)) => Some(d),
            (None, None) => None,
        }
    }

    fn refresh_active_scene_effect(&mut self) {
        let Some(scene_id) = self.active_scene else {
            return;
        };
        self.scenes.entry(scene_id).or_default();

        // The active scene's local tier wins outright when it has something
        // to show; the global tier only feeds the strip when local is empty.
        // A Running temp that loses to the other tier transitions to Paused
        // (preserving its `remaining`) without emitting Superseded — it's a
        // display state, not a lifecycle state.
        let (winner_scene, winner_loser) = {
            let local_state = self.scenes.get_mut(&scene_id).expect("BUG: scene inserted");
            if let Some(scene) = pick_winner_scene(local_state) {
                (Some(scene), Tier::Global)
            } else if let Some(scene) = pick_winner_scene(&mut self.global_state) {
                (Some(scene), Tier::Local)
            } else {
                (None, Tier::None)
            }
        };

        match winner_loser {
            Tier::Global => pause_state_temporary(&mut self.global_state),
            Tier::Local => {
                if let Some(state) = self.scenes.get_mut(&scene_id) {
                    pause_state_temporary(state);
                }
            }
            Tier::None => {}
        }

        if let Some(scene) = winner_scene {
            self.apply_scene(scene);
        } else {
            self.apply_clear();
        }
    }

    fn apply_scene(&mut self, scene: LedScene) {
        if scene == self.applied_scene {
            return;
        }
        self.applied_scene = scene;
        self.coordinator.publish(Layer::Widgets, Some(scene));
    }

    fn apply_clear(&mut self) {
        let cleared = LedScene {
            effect: HwLedEffect::None,
            period: None,
            duration: None,
        };
        if cleared == self.applied_scene {
            return;
        }
        self.applied_scene = cleared;
        self.coordinator.publish(Layer::Widgets, None);
    }

    fn emit(&self, instance_id: InstanceId, request_id: LedRequestId, status: LedRequestStatus) {
        let _ = self.status_tx.send(WidgetRequestStatus {
            instance_id,
            request_id,
            status,
        });
    }
}

#[derive(Debug, Clone, Copy)]
enum Tier {
    Local,
    Global,
    None,
}

/// Pick what the given tier wants to show on the strip, mutating the tier
/// state in place (resume Paused, pop queued temp). Returns the
/// `LedScene` to apply, or `None` if the tier has nothing to play.
fn pick_winner_scene(state: &mut SceneEffectState) -> Option<LedScene> {
    match state.active_temp.take() {
        Some(ActiveTemp::Paused { mut entry }) => {
            entry.scene.duration = Some(entry.remaining);
            let scene = entry.scene;
            let until = Instant::now() + entry.remaining;
            state.active_temp = Some(ActiveTemp::Running { entry, until });
            return Some(scene);
        }
        Some(ActiveTemp::Running { entry, until }) => {
            let scene = entry.scene;
            state.active_temp = Some(ActiveTemp::Running { entry, until });
            return Some(scene);
        }
        None => {}
    }

    if let Some(mut entry) = state.temp_queue.pop_front() {
        entry.scene.duration = Some(entry.remaining);
        let scene = entry.scene;
        let until = Instant::now() + entry.remaining;
        state.active_temp = Some(ActiveTemp::Running { entry, until });
        return Some(scene);
    }

    state.endless_stack.last().map(|entry| entry.scene)
}

/// Transition a `Running` active temp to `Paused`, capturing remaining
/// time. No-op for `Paused` or absent temps. Used both when a scene
/// becomes inactive and when a tier loses to the other tier on refresh.
fn pause_state_temporary(state: &mut SceneEffectState) {
    let Some(active_temp) = state.active_temp.take() else {
        return;
    };
    state.active_temp = Some(match active_temp {
        ActiveTemp::Running { mut entry, until } => {
            entry.remaining = until.saturating_duration_since(Instant::now());
            ActiveTemp::Paused { entry }
        }
        ActiveTemp::Paused { entry } => ActiveTemp::Paused { entry },
    });
}

/// Remove every entry from a state that matches the given predicate,
/// collecting their `(instance_id, request_id)` into `superseded`.
fn sweep_state(
    state: &mut SceneEffectState,
    matches: &impl Fn(&str, LedRequestId) -> bool,
    superseded: &mut Vec<(InstanceId, LedRequestId)>,
) {
    if let Some(active) = state.active_temp.take() {
        match active {
            ActiveTemp::Running { entry, until } => {
                if matches(&entry.instance_id, entry.request_id) {
                    superseded.push((entry.instance_id, entry.request_id));
                } else {
                    state.active_temp = Some(ActiveTemp::Running { entry, until });
                }
            }
            ActiveTemp::Paused { entry } => {
                if matches(&entry.instance_id, entry.request_id) {
                    superseded.push((entry.instance_id, entry.request_id));
                } else {
                    state.active_temp = Some(ActiveTemp::Paused { entry });
                }
            }
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

    let kept_endless: Vec<_> = std::mem::take(&mut state.endless_stack)
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
    state.endless_stack = kept_endless;
}

fn sweep_pending(
    state: &mut PendingState,
    matches: &impl Fn(&str, LedRequestId) -> bool,
    superseded: &mut Vec<(InstanceId, LedRequestId)>,
) {
    state.endless_stack.retain(|p| {
        if matches(&p.entry.instance_id, p.entry.request_id) {
            superseded.push((p.entry.instance_id.clone(), p.entry.request_id));
            false
        } else {
            true
        }
    });
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
/// and return the request's identity for `Completed` emission.
fn expire_state_if_due(
    state: &mut SceneEffectState,
    now: Instant,
) -> Option<(InstanceId, LedRequestId)> {
    let active = state.active_temp.take()?;
    match active {
        ActiveTemp::Running { entry, until } if until <= now => {
            Some((entry.instance_id, entry.request_id))
        }
        running @ ActiveTemp::Running { .. } => {
            state.active_temp = Some(running);
            None
        }
        paused @ ActiveTemp::Paused { .. } => {
            state.active_temp = Some(paused);
            None
        }
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
        status_rx: mpsc::UnboundedReceiver<WidgetRequestStatus>,
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

        fn drain_statuses(&mut self) -> Vec<WidgetRequestStatus> {
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
        let state = h.manager.scenes.get(&scene)?;
        let active = state.active_temp.as_ref()?;
        match active {
            ActiveTemp::Running { entry, .. } | ActiveTemp::Paused { entry } => {
                Some(entry.request_id)
            }
        }
    }

    fn force_running_temp_until(h: &mut Harness, scene: SceneId, until: Instant) {
        let state = h
            .manager
            .scenes
            .get_mut(&scene)
            .expect("BUG: scene state must exist");
        let active = state
            .active_temp
            .take()
            .expect("BUG: active temp must exist");
        state.active_temp = Some(match active {
            ActiveTemp::Running { entry, .. } => ActiveTemp::Running { entry, until },
            ActiveTemp::Paused { entry } => ActiveTemp::Paused { entry },
        });
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
        assert_eq!(statuses[0].status, LedRequestStatus::Completed);
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
        assert_eq!(statuses[0].status, LedRequestStatus::Completed);
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
        assert_eq!(statuses[0].status, LedRequestStatus::Completed);
        assert_eq!(active_temp_request_id(&h, scene), None);
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(9, 9, 9), 0, None)
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

        let baseline = h.manager.applied_scene;
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
        assert_eq!(h.manager.applied_scene, baseline);
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
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(20, 0, 0), 0, None)
        );
    }

    #[test]
    fn temporary_effect_pauses_on_scene_hide_and_resumes_on_show() {
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
        assert!(h.manager.active_deadline().is_some());

        h.manager.on_scene_changed(scene_b, vec![widget_b.clone()]);
        assert!(h.manager.active_deadline().is_none());

        h.manager.on_scene_changed(scene_a, vec![widget_a.clone()]);
        assert!(h.manager.active_deadline().is_some());
    }

    #[test]
    fn widget_stop_only_cancels_own_requests() {
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
            LedScope::Local,
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
        let active = h.manager.global_state.active_temp.as_ref()?;
        match active {
            ActiveTemp::Running { entry, .. } | ActiveTemp::Paused { entry } => {
                Some(entry.request_id)
            }
        }
    }

    fn global_active_temp_is_paused(h: &Harness) -> bool {
        matches!(
            h.manager.global_state.active_temp.as_ref(),
            Some(ActiveTemp::Paused { .. })
        )
    }

    fn force_global_running_until(h: &mut Harness, until: Instant) {
        let active = h
            .manager
            .global_state
            .active_temp
            .take()
            .expect("BUG: global active temp must exist");
        h.manager.global_state.active_temp = Some(match active {
            ActiveTemp::Running { entry, .. } => ActiveTemp::Running { entry, until },
            ActiveTemp::Paused { entry } => ActiveTemp::Paused { entry },
        });
    }

    #[test]
    fn local_temp_preempts_global_temp_then_resumes() {
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
        // Global temp is the only request and the active scene has nothing
        // local, so global drives the strip.
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::KnightRider, rgb(8, 0, 0), 500, Some(5_000))
        );

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
        // Local wins; global pauses without emitting Superseded.
        assert_eq!(active_temp_request_id(&h, scene), Some(600));
        assert!(global_active_temp_is_paused(&h));
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(0, 9, 0), 0, Some(1_000))
        );

        // Local completes; global resumes.
        force_running_temp_until(&mut h, scene, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();
        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].request_id, 600);
        assert_eq!(statuses[0].status, LedRequestStatus::Completed);
        assert_eq!(global_active_temp_request_id(&h), Some(500));
        assert!(!global_active_temp_is_paused(&h));

        // Global eventually completes under its original id.
        force_global_running_until(&mut h, Instant::now() - Duration::from_millis(1));
        h.manager.on_active_expiry();
        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].request_id, 500);
        assert_eq!(statuses[0].status, LedRequestStatus::Completed);
    }

    #[test]
    fn local_endless_hides_global_endless_without_superseded() {
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
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(7, 0, 0), 0, None)
        );

        h.manager.on_endless(
            widget.clone(),
            800,
            ProtoLedEffect::Solid,
            rgb(0, 0, 7),
            0,
            LedScope::Local,
        );
        let statuses = h.drain_statuses();
        // Only Accepted(800) is emitted; the global endless is hidden, not
        // superseded.
        assert!(
            !statuses
                .iter()
                .any(|s| s.request_id == 700 && s.status == LedRequestStatus::Superseded),
            "global endless must not receive Superseded on cross-tier preemption: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(0, 0, 7), 0, None)
        );

        h.manager.on_stop(&widget, 800);
        h.drain_statuses();
        // Global endless re-applies.
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(7, 0, 0), 0, None)
        );
    }

    #[test]
    fn global_endless_stack_supersedes_within_tier() {
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
        // 901: Accepted; then 902 supersedes 901; then 902: Accepted.
        assert!(
            statuses
                .iter()
                .any(|s| s.request_id == 901 && s.status == LedRequestStatus::Superseded),
            "G1 must receive Superseded when G2 lands: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(2, 0, 0), 0, None)
        );

        h.manager.on_stop(&widget, 902);
        let statuses = h.drain_statuses();
        assert!(
            statuses
                .iter()
                .any(|s| s.request_id == 902 && s.status == LedRequestStatus::Superseded),
            "G2 must receive Superseded when stopped: {statuses:?}"
        );
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(1, 0, 0), 0, None)
        );
    }

    #[test]
    fn scene_change_falls_through_to_global() {
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
        // Active scene local is winning; global endless is on its stack but
        // not displayed.
        assert_eq!(active_temp_request_id(&h, scene_a), Some(41));
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(4, 0, 0), 0, Some(5_000))
        );

        h.manager.on_scene_changed(scene_b, vec![widget_b.clone()]);
        // Scene A's local temp paused; scene B has nothing local; global
        // endless wins.
        assert_eq!(
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(0, 4, 0), 0, None)
        );
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
        // Both tiers cleared, strip falls through to apply_clear.
        assert_eq!(h.manager.applied_scene.effect, HwLedEffect::None);
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
        assert_eq!(h.manager.applied_scene.effect, HwLedEffect::None);
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
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(255, 0, 0), 0, None),
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
            h.manager.applied_scene,
            build_scene(ProtoLedEffect::Solid, rgb(0, 0, 255), 0, None),
        );
    }
}

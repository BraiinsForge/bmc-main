// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use bmc_led::data::{LedCommand, LedEffect as HwLedEffect, LedScene, Rgb};
use bmc_widget_protocol::{
    LED_REQUEST_ID_ALL, LedEffect as ProtoLedEffect, LedRequestId, LedRequestStatus, RgbColor,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::compositor::{InstanceId, LedRequestStatusEvent};
use crate::led::LedController;
use crate::scene::SceneId;
use tracing::warn;

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

pub(crate) struct LedSceneManager<T: crate::BmcManager> {
    led_controller: LedController<T>,
    status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    widget_to_scene: HashMap<InstanceId, SceneId>,
    active_scene: Option<SceneId>,
    scenes: HashMap<SceneId, SceneEffectState>,
    applied_scene: LedScene,
}

impl<T: crate::BmcManager> LedSceneManager<T> {
    pub(crate) fn new(
        led_controller: LedController<T>,
        status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    ) -> Self {
        Self {
            led_controller,
            status_tx,
            widget_to_scene: HashMap::new(),
            active_scene: None,
            scenes: HashMap::new(),
            applied_scene: LedScene {
                effect: HwLedEffect::None,
                period: None,
                duration: None,
            },
        }
    }

    pub(crate) fn on_scene_changed(&mut self, scene_id: SceneId, widget_ids: Vec<InstanceId>) {
        if let Some(previous_scene_id) = self.active_scene
            && previous_scene_id != scene_id
        {
            self.pause_scene_temporary(previous_scene_id);
        }

        self.active_scene = Some(scene_id);
        for widget_id in widget_ids {
            self.widget_to_scene.insert(widget_id, scene_id);
        }
        self.scenes.entry(scene_id).or_default();
        self.refresh_active_scene_effect();
    }

    pub(crate) fn on_temporary(
        &mut self,
        instance_id: InstanceId,
        request_id: LedRequestId,
        effect: ProtoLedEffect,
        color: RgbColor,
        period_ms: u32,
        duration_ms: u32,
    ) {
        let Some(scene_id) = self.widget_to_scene.get(&instance_id).copied() else {
            warn!(%instance_id, "ignoring led temporary without widget scene mapping");
            return;
        };

        let entry = TempEntry {
            instance_id: instance_id.clone(),
            request_id,
            scene: build_scene(effect, color, period_ms, Some(u64::from(duration_ms))),
            remaining: Duration::from_millis(u64::from(duration_ms)),
        };
        self.emit(instance_id, request_id, LedRequestStatus::Accepted);

        let can_start_now = self.active_scene == Some(scene_id)
            && !self.scene_has_active_temp(scene_id)
            && self
                .scenes
                .get(&scene_id)
                .is_none_or(|state| state.temp_queue.is_empty());
        if can_start_now {
            self.start_temporary(scene_id, entry);
        } else {
            self.scenes
                .entry(scene_id)
                .or_default()
                .temp_queue
                .push_back(entry);
        }
    }

    pub(crate) fn on_endless(
        &mut self,
        instance_id: InstanceId,
        request_id: LedRequestId,
        effect: ProtoLedEffect,
        color: RgbColor,
        period_ms: u32,
    ) {
        let Some(scene_id) = self.widget_to_scene.get(&instance_id).copied() else {
            warn!(%instance_id, "ignoring led endless without widget scene mapping");
            return;
        };

        let new_entry = EndlessEntry {
            instance_id: instance_id.clone(),
            request_id,
            scene: build_scene(effect, color, period_ms, None),
        };
        let previous_top = self
            .scenes
            .entry(scene_id)
            .or_default()
            .endless_stack
            .last()
            .map(|entry| (entry.instance_id.clone(), entry.request_id));
        self.scenes
            .entry(scene_id)
            .or_default()
            .endless_stack
            .push(new_entry.clone());

        if let Some((old_instance_id, old_request_id)) = previous_top {
            self.emit(
                old_instance_id,
                old_request_id,
                LedRequestStatus::Superseded,
            );
        }
        self.emit(instance_id, request_id, LedRequestStatus::Accepted);

        if self.active_scene == Some(scene_id) && !self.scene_has_active_temp(scene_id) {
            self.apply_scene(new_entry.scene);
        }
    }

    pub(crate) fn on_stop(&mut self, instance_id: &str, request_id: LedRequestId) {
        let cancel_all = request_id == LED_REQUEST_ID_ALL;
        let matches = |stored_instance: &str, stored_id: LedRequestId| -> bool {
            stored_instance == instance_id && (cancel_all || stored_id == request_id)
        };

        let mut superseded: Vec<(InstanceId, LedRequestId)> = Vec::new();

        for state in self.scenes.values_mut() {
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
        let Some(scene_id) = self.active_scene else {
            return;
        };

        let expired = {
            let state = self.scenes.entry(scene_id).or_default();
            let Some(active_temp) = state.active_temp.take() else {
                return;
            };
            match active_temp {
                ActiveTemp::Running { entry, until } => {
                    if Instant::now() < until {
                        state.active_temp = Some(ActiveTemp::Running { entry, until });
                        return;
                    }
                    Some((entry.instance_id, entry.request_id))
                }
                ActiveTemp::Paused { entry } => {
                    state.active_temp = Some(ActiveTemp::Paused { entry });
                    None
                }
            }
        };

        if let Some((instance_id, request_id)) = expired {
            self.emit(instance_id, request_id, LedRequestStatus::Completed);
            self.refresh_active_scene_effect();
        }
    }

    pub(crate) fn active_deadline(&self) -> Option<Instant> {
        let scene_id = self.active_scene?;
        let state = self.scenes.get(&scene_id)?;
        match state.active_temp.as_ref()? {
            ActiveTemp::Running { until, .. } => Some(*until),
            ActiveTemp::Paused { .. } => None,
        }
    }

    fn pause_scene_temporary(&mut self, scene_id: SceneId) {
        let Some(state) = self.scenes.get_mut(&scene_id) else {
            return;
        };
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

    fn refresh_active_scene_effect(&mut self) {
        let Some(scene_id) = self.active_scene else {
            return;
        };
        self.scenes.entry(scene_id).or_default();

        let paused_entry = {
            let state = self.scenes.get_mut(&scene_id).expect("BUG: scene inserted");
            match state.active_temp.take() {
                Some(ActiveTemp::Paused { entry }) => Some(entry),
                Some(running @ ActiveTemp::Running { .. }) => {
                    state.active_temp = Some(running);
                    None
                }
                None => None,
            }
        };
        if let Some(entry) = paused_entry {
            self.start_temporary(scene_id, entry);
            return;
        }

        if self.scene_has_running_temp(scene_id) {
            return;
        }

        let next = self
            .scenes
            .get_mut(&scene_id)
            .expect("BUG: scene inserted")
            .temp_queue
            .pop_front();
        if let Some(entry) = next {
            self.start_temporary(scene_id, entry);
            return;
        }

        let endless = self
            .scenes
            .get(&scene_id)
            .expect("BUG: scene inserted")
            .endless_stack
            .last()
            .map(|entry| entry.scene);
        if let Some(scene) = endless {
            self.apply_scene(scene);
        } else {
            self.apply_clear();
        }
    }

    fn start_temporary(&mut self, scene_id: SceneId, mut entry: TempEntry) {
        entry.scene.duration = Some(entry.remaining);
        let until = Instant::now() + entry.remaining;
        let scene = entry.scene;

        self.scenes
            .entry(scene_id)
            .or_default()
            .active_temp
            .replace(ActiveTemp::Running { entry, until });
        self.apply_scene(scene);
    }

    fn scene_has_running_temp(&self, scene_id: SceneId) -> bool {
        self.scenes.get(&scene_id).is_some_and(|state| {
            state
                .active_temp
                .as_ref()
                .is_some_and(|active| matches!(active, ActiveTemp::Running { .. }))
        })
    }

    fn scene_has_active_temp(&self, scene_id: SceneId) -> bool {
        self.scenes
            .get(&scene_id)
            .is_some_and(|state| state.active_temp.is_some())
    }

    fn apply_scene(&mut self, scene: LedScene) {
        self.applied_scene = scene;
        self.led_controller
            .send_command(LedCommand::SetEffect(scene));
    }

    fn apply_clear(&mut self) {
        self.apply_scene(LedScene {
            effect: HwLedEffect::None,
            period: None,
            duration: None,
        });
    }

    fn emit(&self, instance_id: InstanceId, request_id: LedRequestId, status: LedRequestStatus) {
        let _ = self.status_tx.send(LedRequestStatusEvent {
            instance_id,
            request_id,
            status,
        });
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
    use std::net::IpAddr;
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::anyhow;
    use axum_extra::extract::cookie::Cookie;
    use bmc_platform::BmcPlatform;
    use bmc_shared_ii_net::wifi::{EncryptionType, WifiScanItem, WifiStatus};
    use bmc_shared_time::time::Timezone;
    use bmc_support::SupportArchiveFormat;
    use uuid::Uuid;

    use super::*;
    use crate::alarm::AlarmBus;
    use crate::bootloader_config::BootloaderConfig;
    use crate::manager::{
        BmcState, IfaceData, InitialSetupError, NetworkProtocolConfig, WifiData, WifiEvent,
        WifiNetworkConfig,
    };
    use crate::session;
    use crate::system_upgrade::StateService;

    #[derive(Clone, Debug)]
    struct DummySessionHandle;

    impl session::Handle for DummySessionHandle {
        fn is_valid(&self) -> bool {
            true
        }

        fn id(&self) -> String {
            "dummy-session".to_owned()
        }
    }

    #[derive(Clone, Debug, Default)]
    struct DummySessionManager;

    #[derive(Debug, thiserror::Error)]
    #[error("dummy session error")]
    struct DummySessionError;

    #[async_trait::async_trait]
    impl session::Manager for DummySessionManager {
        type Error = DummySessionError;
        type Session = DummySessionHandle;

        const SESSION_TIMEOUT: u32 = 1;

        async fn login(&self, _password: &str) -> Result<Cookie<'static>, Self::Error> {
            Err(DummySessionError)
        }

        async fn logout(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
            Err(DummySessionError)
        }

        async fn logout_all_related(&self, _session: Self::Session) -> Result<(), Self::Error> {
            Err(DummySessionError)
        }

        async fn extend(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
            Err(DummySessionError)
        }

        async fn find(&self, _cookies: &[Cookie<'_>]) -> Result<Self::Session, Self::Error> {
            Err(DummySessionError)
        }
    }

    #[derive(Clone, Debug)]
    struct DummyManager {
        timezone_tx: tokio::sync::watch::Sender<Timezone>,
    }

    impl DummyManager {
        fn new() -> Self {
            let (timezone_tx, _timezone_rx) = tokio::sync::watch::channel(Timezone::default());
            Self { timezone_tx }
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("dummy manager error")]
    struct DummyManagerError;

    #[async_trait::async_trait]
    impl crate::BmcManager for DummyManager {
        type SessionManager = DummySessionManager;
        type Error = DummyManagerError;

        async fn version(&self) -> Option<bmc_platform::BosVersion> {
            None
        }

        fn platform(&self) -> BmcPlatform {
            BmcPlatform::BraiinsBmc
        }

        async fn upgrade(
            &self,
            _keep_settings: bool,
            _upgrade_image_path: &Path,
        ) -> anyhow::Result<()> {
            Err(anyhow!("not implemented"))
        }

        async fn check_and_remove_upgrade_marker(&self) -> bool {
            false
        }

        fn session_manager(&self) -> Self::SessionManager {
            DummySessionManager
        }

        async fn check_password(&self, _password: Option<&str>) -> Result<bool, Self::Error> {
            Err(DummyManagerError)
        }

        async fn set_password(&self, _password: Option<String>) -> Result<(), Self::Error> {
            Err(DummyManagerError)
        }

        fn timezone(&self) -> Timezone {
            Timezone::default()
        }

        async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()> {
            let _ = self.timezone_tx.send(timezone);
            Ok(())
        }

        fn watch_timezone_updates(&self) -> tokio::sync::watch::Receiver<Timezone> {
            self.timezone_tx.subscribe()
        }

        async fn is_factory_default(&self) -> bool {
            false
        }

        async fn factory_reset(&self, _hard: bool) -> Result<(), Self::Error> {
            Err(DummyManagerError)
        }

        async fn is_setup_pending(&self) -> bool {
            false
        }

        async fn is_wifi_reconfig(&self) -> bool {
            false
        }

        async fn enter_wifi_reconfig(&self) -> Result<(), InitialSetupError> {
            Err(InitialSetupError::NotSupported)
        }

        async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
            Err(InitialSetupError::NotSupported)
        }

        async fn hostname(&self) -> Option<String> {
            None
        }

        fn mac_address(&self) -> Option<String> {
            None
        }

        async fn ip_address(&self) -> Option<IpAddr> {
            None
        }

        async fn network_config(&self) -> Option<NetworkProtocolConfig> {
            None
        }

        async fn set_network_config(&self, _config: NetworkProtocolConfig) -> anyhow::Result<()> {
            Err(anyhow!("not implemented"))
        }

        async fn captive_portal_redirect_host(&self) -> Option<String> {
            None
        }

        async fn wifi_initial_setup(
            &self,
            _config: WifiNetworkConfig,
        ) -> Result<(), InitialSetupError> {
            Err(InitialSetupError::NotSupported)
        }

        async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError> {
            Err(InitialSetupError::NotSupported)
        }

        async fn wifi_scan(&self) -> anyhow::Result<Vec<WifiScanItem>> {
            Ok(Vec::new())
        }

        fn subscribe_wifi_events(&self) -> tokio::sync::broadcast::Receiver<WifiEvent> {
            let (_tx, rx) = tokio::sync::broadcast::channel(1);
            rx
        }

        async fn reboot(&self) -> anyhow::Result<()> {
            Err(anyhow!("not implemented"))
        }

        async fn device_state(&self) -> BmcState {
            BmcState::Operational
        }

        async fn update_device_state(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn wifi_ssid(&self) -> anyhow::Result<String> {
            Err(anyhow!("not implemented"))
        }

        async fn init_wifi_ap(&self) -> Result<(), Self::Error> {
            Err(DummyManagerError)
        }

        async fn wifi_save_and_connect(
            &self,
            _ssid: String,
            _password: Option<String>,
            _encryption: EncryptionType,
        ) -> Result<(), Self::Error> {
            Err(DummyManagerError)
        }

        async fn wifi_status(&self) -> anyhow::Result<WifiData> {
            Ok(WifiData {
                iface: IfaceData::default(),
                status: WifiStatus::default(),
            })
        }

        async fn wifi_saved_networks(&self) -> anyhow::Result<Vec<WifiStatus>> {
            Ok(Vec::new())
        }

        async fn handle_graceful_shutdown(&self) {}

        async fn support_archive(
            &self,
            _format: SupportArchiveFormat,
        ) -> Result<Vec<u8>, Self::Error> {
            Err(DummyManagerError)
        }

        async fn sync_boot_environment(
            &self,
            _config: &BootloaderConfig,
        ) -> Result<(), Self::Error> {
            Err(DummyManagerError)
        }
    }

    struct Harness {
        manager: LedSceneManager<DummyManager>,
        status_rx: mpsc::UnboundedReceiver<LedRequestStatusEvent>,
    }

    impl Harness {
        fn new() -> Self {
            let state_service = StateService::new();
            let manager = Arc::new(DummyManager::new());
            let (_price_tx, price_rx) = tokio::sync::watch::channel(0.0_f32);
            let alarm_bus = AlarmBus::new();
            let (status_tx, status_rx) = mpsc::unbounded_channel();
            let (led_controller, _state_tx) =
                LedController::new(&state_service, manager, price_rx, true, alarm_bus);
            let manager = LedSceneManager::new(led_controller, status_tx);
            Self { manager, status_rx }
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
        );
        h.manager.on_temporary(
            widget.clone(),
            22,
            ProtoLedEffect::Solid,
            rgb(3, 0, 0),
            0,
            2_000,
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
        h.manager
            .on_endless(widget.clone(), 31, ProtoLedEffect::Solid, rgb(9, 9, 9), 0);
        h.manager.on_temporary(
            widget.clone(),
            32,
            ProtoLedEffect::Solid,
            rgb(4, 0, 0),
            0,
            1_000,
        );
        h.manager.on_temporary(
            widget.clone(),
            33,
            ProtoLedEffect::Solid,
            rgb(5, 0, 0),
            0,
            1_000,
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

        h.manager
            .on_endless(widget_b.clone(), 1, ProtoLedEffect::Solid, rgb(10, 0, 0), 0);
        h.manager
            .on_endless(widget_b.clone(), 2, ProtoLedEffect::Solid, rgb(20, 0, 0), 0);

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
        );
        h.manager.on_endless(
            widget_b.clone(),
            200,
            ProtoLedEffect::Solid,
            rgb(0, 1, 0),
            0,
        );
        h.drain_statuses();

        h.manager.on_stop(&widget_a, LED_REQUEST_ID_ALL);

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_a);
        assert_eq!(statuses[0].request_id, 100);
        assert_eq!(statuses[0].status, LedRequestStatus::Superseded);
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

        h.manager
            .on_endless(widget_a.clone(), 10, ProtoLedEffect::Solid, rgb(1, 0, 0), 0);
        h.manager
            .on_endless(widget_b.clone(), 20, ProtoLedEffect::Solid, rgb(0, 1, 0), 0);
        h.drain_statuses();

        h.manager.on_widget_disconnected(&widget_b);

        let statuses = h.drain_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].instance_id, widget_b);
        assert_eq!(statuses[0].request_id, 20);
        assert_eq!(statuses[0].status, LedRequestStatus::Superseded);
    }
}

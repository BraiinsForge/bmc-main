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

//! Mock compositor for x86 development without Wayland.
//!
//! Logs all compositor operations instead of rendering to a display.

use std::collections::BTreeSet;

use bmc::compositor::{
    ActiveScene, AlarmCommand, Compositor, CompositorError, CompositorEvent, CredentialSecrets,
    HardwareCapabilities, InstanceId, LedRequestStatusEvent, Position, SceneCycling, SceneLayout,
    SettingUpdate, SettingsCommand, Size, UpgradeDisplaySnapshot, WidgetAction,
    WidgetInitialConfig,
};
use bmc_platform::{HardwareProfile, Product};
use tokio::sync::{broadcast, mpsc, watch};

#[derive(Debug)]
struct MockSceneState {
    scenes: Vec<SceneLayout>,
    current_index: usize,
}

impl Default for MockSceneState {
    fn default() -> Self {
        Self {
            scenes: vec![SceneLayout::default()],
            current_index: 0,
        }
    }
}

impl MockSceneState {
    fn active_scene(&self) -> &SceneLayout {
        &self.scenes[self.current_index]
    }

    fn active_snapshot(&self) -> (Option<bmc::scene::SceneId>, Vec<InstanceId>) {
        (
            self.active_scene().scene_id,
            self.active_scene()
                .widgets
                .iter()
                .filter(|widget| widget.visible)
                .map(|widget| widget.instance_id.clone())
                .collect(),
        )
    }

    fn set_active_scene(&mut self, layout: SceneLayout) {
        let index = layout.scene_id.and_then(|id| {
            self.scenes
                .iter()
                .position(|scene| scene.scene_id == Some(id))
        });
        if let Some(index) = index {
            self.scenes[index] = layout;
            self.current_index = index;
        } else {
            self.scenes = vec![layout];
            self.current_index = 0;
        }
    }

    fn set_scene_cycling(&mut self, scenes: Vec<SceneLayout>) {
        let active_id = self.active_scene().scene_id;
        if scenes.is_empty() {
            self.scenes = vec![SceneLayout::default()];
            self.current_index = 0;
            return;
        }

        self.current_index = active_id
            .and_then(|id| scenes.iter().position(|scene| scene.scene_id == Some(id)))
            .unwrap_or(0);
        self.scenes = scenes;
    }
}

#[derive(Debug)]
pub struct MockCompositor {
    action_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<WidgetAction>>>,
    settings_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<SettingsCommand>>>,
    alarm_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<AlarmCommand>>>,
    event_tx: broadcast::Sender<CompositorEvent>,
    status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    active_scene_tx: watch::Sender<Option<ActiveScene>>,
    connected_widgets_tx: watch::Sender<BTreeSet<InstanceId>>,
    display_name: std::sync::Mutex<Option<String>>,
    product: Product,
    scene_state: std::sync::Mutex<MockSceneState>,
    upgrade_state: std::sync::Mutex<Option<UpgradeDisplaySnapshot>>,
}

impl MockCompositor {
    #[must_use]
    pub fn new(product: Product) -> Self {
        let (_action_tx, action_rx) = mpsc::unbounded_channel();
        let (_settings_tx, settings_rx) = mpsc::unbounded_channel();
        let (_alarm_tx, alarm_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(64);
        let (status_tx, mut status_rx) = mpsc::unbounded_channel::<LedRequestStatusEvent>();
        tokio::spawn(async move {
            while let Some(status) = status_rx.recv().await {
                tracing::info!(
                    "MockCompositor: led_request_status widget={} req={} status={:?}",
                    status.instance_id,
                    status.request_id,
                    status.status
                );
            }
        });
        let (active_scene_tx, _) = watch::channel(None);
        let (connected_widgets_tx, _) = watch::channel(BTreeSet::new());
        Self {
            action_rx: std::sync::Mutex::new(Some(action_rx)),
            settings_rx: std::sync::Mutex::new(Some(settings_rx)),
            alarm_rx: std::sync::Mutex::new(Some(alarm_rx)),
            event_tx,
            status_tx,
            active_scene_tx,
            connected_widgets_tx,
            display_name: std::sync::Mutex::new(None),
            product,
            scene_state: std::sync::Mutex::new(MockSceneState::default()),
            upgrade_state: std::sync::Mutex::new(None),
        }
    }

    fn emit_active_scene_changed(
        &self,
        scene_id: bmc::scene::SceneId,
        widget_ids: Vec<InstanceId>,
    ) {
        let _ = self.active_scene_tx.send(Some(ActiveScene {
            scene_id,
            widget_ids,
        }));
    }

    fn emit_active_scene_changed_if_changed<F>(&self, update_scene_state: F)
    where
        F: FnOnce(&mut MockSceneState),
    {
        let active_scene_after = {
            let mut scene_state = self
                .scene_state
                .lock()
                .expect("BUG: scene_state lock poisoned");
            let active_scene_before = scene_state.active_snapshot();
            update_scene_state(&mut scene_state);
            let active_scene_after = scene_state.active_snapshot();
            (active_scene_before != active_scene_after).then_some(active_scene_after)
        };
        if let Some((Some(scene_id), widget_ids)) = active_scene_after {
            self.emit_active_scene_changed(scene_id, widget_ids);
        }
    }

    #[must_use]
    pub fn upgrade_state(&self) -> Option<UpgradeDisplaySnapshot> {
        self.upgrade_state
            .lock()
            .expect("BUG: upgrade_state lock poisoned")
            .clone()
    }
}

impl Compositor for MockCompositor {
    fn start(&self) -> Result<String, CompositorError> {
        let name = "wayland-mock-0".to_owned();
        tracing::info!("MockCompositor: started (display={})", name);
        *self
            .display_name
            .lock()
            .expect("BUG: display_name lock poisoned") = Some(name.clone());
        Ok(name)
    }

    fn wayland_display(&self) -> Option<String> {
        self.display_name
            .lock()
            .expect("BUG: display_name lock poisoned")
            .clone()
    }

    fn hardware_capabilities(&self) -> HardwareCapabilities {
        HardwareProfile::for_product(self.product).capabilities()
    }

    fn set_upgrade_state(&self, state: UpgradeDisplaySnapshot) -> Result<(), CompositorError> {
        tracing::info!(?state, "MockCompositor: set upgrade state");
        *self
            .upgrade_state
            .lock()
            .expect("BUG: upgrade_state lock poisoned") = Some(state);
        Ok(())
    }

    fn register_widget(
        &self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        initial_config: WidgetInitialConfig,
    ) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: register widget '{}' at ({},{}) size {}x{} initial={:?}",
            instance_id,
            position.x,
            position.y,
            size.width,
            size.height,
            initial_config,
        );
        self.connected_widgets_tx.send_modify(|set| {
            set.insert(instance_id.clone());
        });
        // Immediately signal that the widget is ready
        let _ = self
            .event_tx
            .send(CompositorEvent::WidgetReady { instance_id });
        Ok(())
    }

    fn set_widget_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: set_widget_pid '{}' pid={}",
            instance_id,
            pid
        );
        Ok(())
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: unregister widget '{}'", instance_id);
        self.connected_widgets_tx.send_modify(|set| {
            set.remove(instance_id);
        });
        Ok(())
    }

    fn clear_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: clear_pid instance='{}' pid={}",
            instance_id,
            pid
        );
        Ok(())
    }

    fn set_active_scene(&self, layout: SceneLayout) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: set active scene with {} widgets",
            layout.widgets.len()
        );
        for w in &layout.widgets {
            tracing::debug!(
                "  widget '{}': ({},{}) {}x{} visible={}",
                w.instance_id,
                w.position.x,
                w.position.y,
                w.size.width,
                w.size.height,
                w.visible,
            );
        }
        self.emit_active_scene_changed_if_changed(|scene_state| {
            scene_state.set_active_scene(layout);
        });
        Ok(())
    }

    fn set_scene_cycling(&self, scenes: Vec<SceneLayout>) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: set scene cycling with {} scenes",
            scenes.len()
        );
        self.emit_active_scene_changed_if_changed(|scene_state| {
            scene_state.set_scene_cycling(scenes);
        });
        Ok(())
    }

    fn set_scene_cycling_config(&self, config: SceneCycling) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: set scene cycling config {:?}", config);
        Ok(())
    }

    fn set_scene_cycling_suspended(&self, suspended: bool) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: set scene cycling suspended={suspended}");
        Ok(())
    }

    fn reset_to_first_scene(&self) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: reset to first scene");
        Ok(())
    }

    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: broadcast setting {:?}", setting);
        Ok(())
    }

    fn update_widget_params(
        &self,
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError> {
        tracing::info!(
            "MockCompositor: update_widget_params {instance_id}: {}",
            serde_json::Value::Object(params)
        );
        Ok(())
    }

    fn update_widget_credentials(
        &self,
        instance_id: &InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        _secrets: CredentialSecrets,
    ) -> Result<(), CompositorError> {
        // The view names accounts but carries no secret, so logging it is safe.
        tracing::info!(
            "MockCompositor: update_widget_credentials {instance_id}: {}",
            serde_json::Value::Object(credentials)
        );
        Ok(())
    }

    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction> {
        self.action_rx
            .lock()
            .expect("BUG: action_rx lock poisoned")
            .take()
            .expect("BUG: action_receiver already taken")
    }

    fn settings_receiver(&self) -> mpsc::UnboundedReceiver<SettingsCommand> {
        self.settings_rx
            .lock()
            .expect("BUG: settings_rx lock poisoned")
            .take()
            .expect("BUG: settings_receiver already taken")
    }

    fn alarm_receiver(&self) -> mpsc::UnboundedReceiver<AlarmCommand> {
        self.alarm_rx
            .lock()
            .expect("BUG: alarm_rx lock poisoned")
            .take()
            .expect("BUG: alarm_receiver already taken")
    }

    fn request_status_sender(&self) -> mpsc::UnboundedSender<LedRequestStatusEvent> {
        self.status_tx.clone()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<CompositorEvent> {
        self.event_tx.subscribe()
    }

    fn active_scene_watch(&self) -> watch::Receiver<Option<ActiveScene>> {
        self.active_scene_tx.subscribe()
    }

    fn connected_widgets_watch(&self) -> watch::Receiver<BTreeSet<InstanceId>> {
        self.connected_widgets_tx.subscribe()
    }

    fn shutdown(&self) -> Result<(), CompositorError> {
        tracing::info!("MockCompositor: shutdown");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bmc::compositor::{
        ActiveScene, Compositor, Position, SceneLayout, Size, UpgradeDisplaySnapshot,
        UpgradeDisplayState, UpgradeGeneration, UpgradeKind, WidgetPlacement,
    };
    use bmc::scene::SceneId;
    use bmc_platform::{DisplayShape, Product};

    use super::MockCompositor;

    #[tokio::test]
    async fn mock_retains_the_latest_upgrade_snapshot() {
        let compositor = MockCompositor::new(Product::Bmc100);
        let snapshot = UpgradeDisplaySnapshot {
            generation: UpgradeGeneration::new(4),
            state: UpgradeDisplayState::Failed {
                kind: UpgradeKind::Packages,
            },
        };
        compositor
            .set_upgrade_state(snapshot.clone())
            .expect("BUG: mock accepts upgrade state");
        assert_eq!(compositor.upgrade_state(), Some(snapshot));
    }

    fn scene_with_id_and_widgets(id: SceneId, widgets: &[(&str, bool)]) -> SceneLayout {
        SceneLayout {
            scene_id: Some(id),
            cycle_duration: None,
            combined: false,
            widgets: widgets
                .iter()
                .map(|(instance_id, visible)| WidgetPlacement {
                    instance_id: (*instance_id).to_owned(),
                    position: Position { x: 0, y: 0 },
                    size: Size {
                        width: 100,
                        height: 100,
                    },
                    visible: *visible,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn mock_reports_bmc100_capabilities() {
        let caps = MockCompositor::new(Product::Bmc100).hardware_capabilities();
        assert_eq!((caps.display.width, caps.display.height), (1_280, 480));
        assert_eq!(caps.display.shape, DisplayShape::Rectangular);
        assert_eq!(caps.display.dpi, 217);
        assert_eq!(caps.slot_grid.map(|g| (g.columns, g.rows)), Some((4, 2)));
    }

    #[tokio::test]
    async fn set_scene_cycling_updates_active_scene_when_active_scene_falls_back() {
        let id_a = SceneId::generate();
        let id_b = SceneId::generate();
        let compositor = MockCompositor::new(Product::Bmc100);
        compositor
            .set_scene_cycling(vec![
                scene_with_id_and_widgets(id_a, &[("a-visible", true), ("a-hidden", false)]),
                scene_with_id_and_widgets(id_b, &[("b-visible", true)]),
            ])
            .expect("BUG: initial set_scene_cycling should succeed");
        compositor
            .set_active_scene(scene_with_id_and_widgets(id_b, &[("b-visible", true)]))
            .expect("BUG: set_active_scene should succeed");

        let mut active = compositor.active_scene_watch();
        active.borrow_and_update();
        compositor
            .set_scene_cycling(vec![scene_with_id_and_widgets(
                id_a,
                &[("a-visible", true), ("a-hidden", false)],
            )])
            .expect("BUG: set_scene_cycling should succeed");

        assert!(
            active
                .has_changed()
                .expect("BUG: watch sender must be live"),
            "active scene must update on fallback"
        );
        match &*active.borrow_and_update() {
            Some(ActiveScene {
                scene_id,
                widget_ids,
            }) => {
                assert_eq!(*scene_id, id_a);
                assert_eq!(*widget_ids, vec![String::from("a-visible")]);
            }
            None => panic!("BUG: expected an active scene after fallback"),
        }
    }
}

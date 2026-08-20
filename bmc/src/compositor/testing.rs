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

//! A recording [`Compositor`] double, shared by the tests in this crate.

use std::sync::Mutex;

use bmc_platform::{HardwareProfile, Product};

use super::*;

pub(crate) type CredentialPush = (
    InstanceId,
    serde_json::Map<String, serde_json::Value>,
    CredentialSecrets,
);

#[derive(Default)]
pub(crate) struct RecordingCompositor {
    pub(crate) scene_cycling_configs: Mutex<Vec<SceneCycling>>,
    pub(crate) scene_cycling_lists: Mutex<Vec<Vec<SceneLayout>>>,
    pub(crate) credential_pushes: Mutex<Vec<CredentialPush>>,
    pub(crate) connected: Mutex<BTreeSet<InstanceId>>,
    widget_calls: Mutex<Vec<String>>,
}

impl RecordingCompositor {
    fn record(&self, call: String) {
        self.widget_calls
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .push(call);
    }

    /// The widget pid and teardown calls, in the order they arrived.
    pub(crate) fn widget_calls(&self) -> Vec<String> {
        self.widget_calls
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .clone()
    }
}

impl Compositor for RecordingCompositor {
    fn start(&self) -> Result<String, CompositorError> {
        Ok("test-display".to_owned())
    }

    fn wayland_display(&self) -> Option<String> {
        Some("test-display".to_owned())
    }

    fn hardware_capabilities(&self) -> HardwareCapabilities {
        HardwareProfile::for_product(Product::Bmc100).capabilities()
    }

    fn register_widget(
        &self,
        _instance_id: InstanceId,
        _position: Position,
        _size: Size,
        _initial_config: WidgetInitialConfig,
    ) -> Result<(), CompositorError> {
        Ok(())
    }

    fn set_widget_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        self.record(format!("set_pid {instance_id} {pid}"));
        Ok(())
    }

    fn bind_respawned_pid(
        &self,
        instance_id: &InstanceId,
        pid: u32,
    ) -> Result<(), CompositorError> {
        self.record(format!("bind_respawned {instance_id} {pid}"));
        Ok(())
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        self.record(format!("unregister {instance_id}"));
        Ok(())
    }

    fn unregister_abandoned(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        self.record(format!("unregister_abandoned {instance_id}"));
        Ok(())
    }

    fn clear_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        self.record(format!("clear_pid {instance_id} {pid}"));
        Ok(())
    }

    fn set_active_scene(&self, _layout: SceneLayout) -> Result<(), CompositorError> {
        Ok(())
    }

    fn set_scene_cycling(&self, scenes: Vec<SceneLayout>) -> Result<(), CompositorError> {
        self.scene_cycling_lists
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .push(scenes);
        Ok(())
    }

    fn set_scene_cycling_config(&self, config: SceneCycling) -> Result<(), CompositorError> {
        self.scene_cycling_configs
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .push(config);
        Ok(())
    }

    fn set_scene_cycling_suspended(&self, _suspended: bool) -> Result<(), CompositorError> {
        Ok(())
    }

    fn reset_to_first_scene(&self) -> Result<(), CompositorError> {
        Ok(())
    }

    fn broadcast_setting(&self, _setting: SettingUpdate) -> Result<(), CompositorError> {
        Ok(())
    }

    fn update_widget_params(
        &self,
        _instance_id: &InstanceId,
        _params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError> {
        Ok(())
    }

    fn update_widget_credentials(
        &self,
        instance_id: &InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: CredentialSecrets,
    ) -> Result<bool, CompositorError> {
        let mut pushes = self
            .credential_pushes
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned");
        let changed = pushes
            .iter()
            .rev()
            .find(|(id, _, _)| id == instance_id)
            .is_none_or(|(_, view, stored)| view != &credentials || stored != &secrets);
        pushes.push((instance_id.clone(), credentials, secrets));
        Ok(changed)
    }

    fn action_receiver(&self) -> tokio::sync::mpsc::UnboundedReceiver<WidgetAction> {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        rx
    }

    fn settings_receiver(&self) -> tokio::sync::mpsc::UnboundedReceiver<SettingsCommand> {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        rx
    }

    fn alarm_receiver(&self) -> tokio::sync::mpsc::UnboundedReceiver<AlarmCommand> {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        rx
    }

    fn request_status_sender(&self) -> tokio::sync::mpsc::UnboundedSender<LedRequestStatusEvent> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        tx
    }

    fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<CompositorEvent> {
        let (_tx, rx) = tokio::sync::broadcast::channel(16);
        rx
    }

    fn active_scene_watch(&self) -> tokio::sync::watch::Receiver<Option<ActiveScene>> {
        let (_tx, rx) = tokio::sync::watch::channel(None);
        rx
    }

    fn connected_widgets_watch(&self) -> tokio::sync::watch::Receiver<BTreeSet<InstanceId>> {
        let connected = self
            .connected
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .clone();
        let (_tx, rx) = tokio::sync::watch::channel(connected);
        rx
    }

    fn shutdown(&self) -> Result<(), CompositorError> {
        Ok(())
    }
}

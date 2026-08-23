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

use std::{collections::BTreeMap, sync::Mutex};

use bmc_platform::{HardwareProfile, Product};

use super::*;

pub(crate) type CredentialPush = (
    InstanceId,
    serde_json::Map<String, serde_json::Value>,
    CredentialSecrets,
);
type RegistrationObserver = Box<dyn FnOnce() + Send>;

#[derive(Default)]
pub(crate) struct RecordingCompositor {
    pub(crate) scene_cycling_configs: Mutex<Vec<SceneCycling>>,
    pub(crate) scene_cycling_lists: Mutex<Vec<Vec<SceneLayout>>>,
    pub(crate) credential_pushes: Mutex<Vec<CredentialPush>>,
    pub(crate) connected: Mutex<BTreeSet<InstanceId>>,
    widget_calls: Mutex<Vec<String>>,
    retained_modes: Mutex<BTreeMap<WidgetInstanceKey, WidgetConnectionMode>>,
    retained_sizes: Mutex<BTreeMap<WidgetInstanceKey, Size>>,
    retained_params: Mutex<BTreeMap<WidgetInstanceKey, serde_json::Map<String, serde_json::Value>>>,
    held_widget_receipts: Mutex<Option<Vec<tokio::sync::oneshot::Sender<()>>>>,
    next_registration_observer: Mutex<Option<RegistrationObserver>>,
}

impl RecordingCompositor {
    fn record(&self, call: String) {
        self.widget_calls
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .push(call);
    }

    pub(crate) fn widget_calls(&self) -> Vec<String> {
        self.widget_calls
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .clone()
    }

    pub(crate) fn retained_mode(&self, key: WidgetInstanceKey) -> Option<WidgetConnectionMode> {
        self.retained_modes
            .lock()
            .expect("BUG: retained-mode lock must not be poisoned")
            .get(&key)
            .copied()
    }

    pub(crate) fn retained_params(
        &self,
        key: WidgetInstanceKey,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        self.retained_params
            .lock()
            .expect("BUG: retained-params lock must not be poisoned")
            .get(&key)
            .cloned()
    }

    pub(crate) fn retained_size(&self, key: WidgetInstanceKey) -> Option<Size> {
        self.retained_sizes
            .lock()
            .expect("BUG: retained-size lock must not be poisoned")
            .get(&key)
            .copied()
    }

    pub(crate) fn hold_widget_receipts(&self) {
        *self
            .held_widget_receipts
            .lock()
            .expect("BUG: receipt gate lock must not be poisoned") = Some(Vec::new());
    }

    pub(crate) fn observe_next_registration(&self, observer: impl FnOnce() + Send + 'static) {
        *self
            .next_registration_observer
            .lock()
            .expect("BUG: registration observer lock must not be poisoned") =
            Some(Box::new(observer));
    }

    pub(crate) fn release_widget_receipts(&self) {
        let receipts = self
            .held_widget_receipts
            .lock()
            .expect("BUG: receipt gate lock must not be poisoned")
            .take()
            .expect("BUG: widget receipts were not held");
        for receipt in receipts {
            let _ = receipt.send(());
        }
    }

    fn widget_receipt(&self, operation: &'static str) -> CompositorReceipt {
        let mut held = self
            .held_widget_receipts
            .lock()
            .expect("BUG: receipt gate lock must not be poisoned");
        let Some(receipts) = held.as_mut() else {
            return CompositorReceipt::completed(operation);
        };
        let (applied, receipt) = CompositorReceipt::pending(operation);
        receipts.push(applied);
        receipt
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

    fn enqueue_register_widget(
        &self,
        registration: WidgetRegistration,
    ) -> Result<CompositorReceipt, CompositorError> {
        if let Some(observer) = self
            .next_registration_observer
            .lock()
            .expect("BUG: registration observer lock must not be poisoned")
            .take()
        {
            observer();
        }
        self.retained_sizes
            .lock()
            .expect("BUG: retained-size lock must not be poisoned")
            .insert(
                registration.key,
                Size {
                    width: registration.initial_config.width,
                    height: registration.initial_config.height,
                },
            );
        self.retained_params
            .lock()
            .expect("BUG: retained-params lock must not be poisoned")
            .insert(registration.key, registration.initial_config.params.clone());
        self.retained_modes
            .lock()
            .expect("BUG: retained-mode lock must not be poisoned")
            .insert(registration.key, registration.connection_mode);
        self.record(format!("register_retained {}", registration.key));
        Ok(self.widget_receipt("register widget"))
    }

    fn enqueue_activate_widget(
        &self,
        key: WidgetInstanceKey,
    ) -> Result<CompositorReceipt, CompositorError> {
        if let Some(mode) = self
            .retained_modes
            .lock()
            .expect("BUG: retained-mode lock must not be poisoned")
            .get_mut(&key)
        {
            *mode = WidgetConnectionMode::Accepting;
        }
        self.record(format!("activate {key}"));
        Ok(self.widget_receipt("activate widget"))
    }

    fn enqueue_deactivate_widget(
        &self,
        key: WidgetInstanceKey,
    ) -> Result<CompositorReceipt, CompositorError> {
        if let Some(mode) = self
            .retained_modes
            .lock()
            .expect("BUG: retained-mode lock must not be poisoned")
            .get_mut(&key)
        {
            *mode = WidgetConnectionMode::Inactive;
        }
        self.record(format!("deactivate {key}"));
        Ok(self.widget_receipt("deactivate widget"))
    }

    fn enqueue_unregister_widget(
        &self,
        key: WidgetInstanceKey,
    ) -> Result<CompositorReceipt, CompositorError> {
        self.retained_modes
            .lock()
            .expect("BUG: retained-mode lock must not be poisoned")
            .remove(&key);
        self.record(format!("unregister_retained {key}"));
        Ok(self.widget_receipt("unregister widget"))
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
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError> {
        let key = WidgetInstanceKey::new(
            instance_id
                .parse()
                .expect("BUG: coordinator instance IDs must be UUIDs"),
        );
        self.retained_params
            .lock()
            .expect("BUG: retained-params lock must not be poisoned")
            .insert(key, params);
        Ok(())
    }

    fn update_widget_credentials(
        &self,
        instance_id: &InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: CredentialSecrets,
    ) -> Result<bool, CompositorError> {
        self.record(format!("credentials {instance_id}"));
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

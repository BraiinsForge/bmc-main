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
pub(crate) type ParameterPush = (InstanceId, serde_json::Map<String, serde_json::Value>);
type RetainedCredentials = (
    serde_json::Map<String, serde_json::Value>,
    CredentialSecrets,
);
type HeldCredentialReceipt = (tokio::sync::oneshot::Sender<bool>, bool);

#[derive(Default)]
pub(crate) struct RecordingCompositor {
    pub(crate) scene_cycling_configs: Mutex<Vec<SceneCycling>>,
    pub(crate) scene_cycling_lists: Mutex<Vec<Vec<SceneLayout>>>,
    pub(crate) credential_pushes: Mutex<Vec<CredentialPush>>,
    pub(crate) parameter_pushes: Mutex<Vec<ParameterPush>>,
    pub(crate) connected: Mutex<BTreeSet<InstanceId>>,
    widget_calls: Mutex<Vec<String>>,
    retained_modes: Mutex<BTreeMap<WidgetInstanceKey, WidgetConnectionMode>>,
    retained_sizes: Mutex<BTreeMap<WidgetInstanceKey, Size>>,
    retained_params: Mutex<BTreeMap<WidgetInstanceKey, serde_json::Map<String, serde_json::Value>>>,
    retained_credentials: Mutex<BTreeMap<WidgetInstanceKey, RetainedCredentials>>,
    held_widget_receipts: Mutex<Option<Vec<tokio::sync::oneshot::Sender<()>>>>,
    next_registration_observer: Mutex<Option<RegistrationObserver>>,
    held_credential_receipts: Mutex<Option<Vec<HeldCredentialReceipt>>>,
    next_credential_error: Mutex<Option<CompositorError>>,
    next_parameter_error: Mutex<Option<CompositorError>>,
    credential_update_attempts: AtomicUsize,
    shutdown_calls: AtomicUsize,
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

    pub(crate) fn shutdown_call_count(&self) -> usize {
        self.shutdown_calls.load(Ordering::Relaxed)
    }

    pub(crate) fn credential_update_attempt_count(&self) -> usize {
        self.credential_update_attempts.load(Ordering::Relaxed)
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

    pub(crate) fn retained_credentials(
        &self,
        key: WidgetInstanceKey,
    ) -> Option<RetainedCredentials> {
        self.retained_credentials
            .lock()
            .expect("BUG: retained-credentials lock must not be poisoned")
            .get(&key)
            .cloned()
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

    pub(crate) fn hold_credential_receipts(&self) {
        *self
            .held_credential_receipts
            .lock()
            .expect("BUG: credential receipt gate lock must not be poisoned") = Some(Vec::new());
    }

    pub(crate) fn release_credential_receipts(&self) {
        let receipts = self
            .held_credential_receipts
            .lock()
            .expect("BUG: credential receipt gate lock must not be poisoned")
            .take()
            .expect("BUG: credential receipts were not held");
        for (receipt, changed) in receipts {
            let _ = receipt.send(changed);
        }
    }

    pub(crate) fn drop_credential_receipts(&self) {
        self.held_credential_receipts
            .lock()
            .expect("BUG: credential receipt gate lock must not be poisoned")
            .take()
            .expect("BUG: credential receipts were not held");
    }

    pub(crate) fn fail_next_credential_update(&self) {
        *self
            .next_credential_error
            .lock()
            .expect("BUG: credential failure lock must not be poisoned") = Some(
            CompositorError::SendError("injected credential update failure".to_owned()),
        );
    }

    pub(crate) fn fail_next_parameter_update(&self) {
        *self
            .next_parameter_error
            .lock()
            .expect("BUG: parameter failure lock must not be poisoned") = Some(
            CompositorError::SendError("injected parameter update failure".to_owned()),
        );
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

    fn credential_receipt(&self, changed: bool) -> CredentialUpdateReceipt {
        let mut held = self
            .held_credential_receipts
            .lock()
            .expect("BUG: credential receipt gate lock must not be poisoned");
        let Some(receipts) = held.as_mut() else {
            return CredentialUpdateReceipt::completed(changed);
        };
        let (applied, receipt) = CredentialUpdateReceipt::pending();
        receipts.push((applied, changed));
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
        self.retained_credentials
            .lock()
            .expect("BUG: retained-credentials lock must not be poisoned")
            .insert(
                registration.key,
                (
                    registration.initial_config.credentials.clone(),
                    registration.initial_config.credential_secrets.clone(),
                ),
            );
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
        let mut modes = self
            .retained_modes
            .lock()
            .expect("BUG: retained-mode lock must not be poisoned");
        let Some(mode) = modes.get_mut(&key) else {
            return Ok(CompositorReceipt::not_applied("activate widget"));
        };
        *mode = WidgetConnectionMode::Accepting;
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
        self.retained_params
            .lock()
            .expect("BUG: retained-params lock must not be poisoned")
            .remove(&key);
        self.retained_sizes
            .lock()
            .expect("BUG: retained-size lock must not be poisoned")
            .remove(&key);
        self.retained_credentials
            .lock()
            .expect("BUG: retained-credentials lock must not be poisoned")
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
        key: WidgetInstanceKey,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError> {
        if let Some(error) = self
            .next_parameter_error
            .lock()
            .expect("BUG: parameter failure lock must not be poisoned")
            .take()
        {
            return Err(error);
        }
        if let Some(stored) = self
            .retained_params
            .lock()
            .expect("BUG: retained-params lock must not be poisoned")
            .get_mut(&key)
        {
            *stored = params.clone();
        }
        self.parameter_pushes
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .push((key.to_string(), params));
        Ok(())
    }

    fn enqueue_update_widget_credentials(
        &self,
        key: WidgetInstanceKey,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: CredentialSecrets,
    ) -> Result<CredentialUpdateReceipt, CompositorError> {
        self.credential_update_attempts
            .fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self
            .next_credential_error
            .lock()
            .expect("BUG: credential failure lock must not be poisoned")
            .take()
        {
            return Err(error);
        }
        self.record(format!("credentials {key}"));
        let changed = {
            let mut retained = self
                .retained_credentials
                .lock()
                .expect("BUG: retained-credentials lock must not be poisoned");
            retained
                .get_mut(&key)
                .is_some_and(|(stored_credentials, stored_secrets)| {
                    let changed = *stored_credentials != credentials || *stored_secrets != secrets;
                    *stored_credentials = credentials.clone();
                    *stored_secrets = secrets.clone();
                    changed
                })
        };
        self.credential_pushes
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .push((key.to_string(), credentials, secrets));
        Ok(self.credential_receipt(changed))
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

// Copyright (C) 2025  Braiins Systems s.r.o.
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

use crate::BmcManager;
use crate::alarm::AlarmController;
use crate::backlight::DisplayBacklightDriver;
use crate::config::ConfigHandle;
use crate::i18n::I18nMarker;
use crate::initial_setup::InitialSetup;
use crate::led::LedController;
use crate::led_coordinator::LedCoordinatorHandle;
use crate::secret_store::SecretStoreHandle;
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::system_upgrade::boser::UpgradeRoute;
use crate::web::SessionManager;
use crate::web::session::extract_session;
use crate::widget::{Coordinator, WidgetRegistry};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use bmc_grpc::web;
use bmc_platform::HardwareCapabilities;
use bmc_upgrade::firmware::FirmwareIndex;
use connectrpc::{ConnectError, ConnectRpcService, Router};
use std::fmt::Display;
use std::sync::Arc;
use strum::EnumMessage;
use tokio::sync::RwLock;

mod account_management;
pub mod authentication;
mod credential_management;
mod language;
mod logging;
mod metadata;
mod system;
use super::SystemUpgradeService;
mod alarm;
mod configuration_service;
mod initial_setup;
mod led_test;
mod network;
mod scene_management;
mod shared;
mod upgrade_service;

use logging::{RpcLog, log_refusal};

const BOSER_MANAGED_STATUS_MESSAGE: &str = "This operation is managed by Boser on this platform";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedRpcOwner {
    Bmc,
    Boser,
}

fn split_procedure(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix('/')?.split_once('/')
}

fn managed_rpc_owner(path: &str) -> Option<ManagedRpcOwner> {
    let (service, method) = split_procedure(path)?;

    match (service, method) {
        (
            web::NETWORK_SERVICE_SERVICE_NAME,
            "GetNetworkInfo"
            | "GetNetworkConfig"
            | "GetWifiStatus"
            | "GetWifiSavedNetworks"
            | "ScanWifi",
        )
        | (web::SYSTEM_SERVICE_SERVICE_NAME, "HasPassword" | "GetTimezone" | "GetTimezoneList")
        | (
            web::UPGRADE_SERVICE_SERVICE_NAME,
            "CheckForUpgrade" | "GetInstallableWidgets" | "StartUpgrade",
        ) => Some(ManagedRpcOwner::Bmc),
        (web::NETWORK_SERVICE_SERVICE_NAME, "SetNetworkConfig" | "SetWifi")
        | (
            web::SYSTEM_SERVICE_SERVICE_NAME,
            "CreatePassword" | "ChangePassword" | "RemovePassword" | "SetTimezone" | "FactoryReset"
            | "Reboot",
        )
        | (web::UPGRADE_SERVICE_SERVICE_NAME, "SetAutoUpgrade" | "GetAutoUpgrade") => {
            Some(ManagedRpcOwner::Boser)
        }
        _ => None,
    }
}

/// The services a client reaches before it has a session.
fn is_public(service: &str) -> bool {
    matches!(
        service,
        web::AUTHENTICATION_SERVICE_SERVICE_NAME
            | web::METADATA_SERVICE_SERVICE_NAME
            | web::INITIAL_SETUP_SERVICE_SERVICE_NAME
            | web::LANGUAGE_SERVICE_SERVICE_NAME
    )
}

/// The procedures a device without the [`I18nMarker`] does not serve.
fn needs_i18n(path: &str) -> bool {
    matches!(
        split_procedure(path),
        Some(
            (web::LANGUAGE_SERVICE_SERVICE_NAME, _)
                | (web::CONFIGURATION_SERVICE_SERVICE_NAME, "SetLanguage")
        )
    )
}

/// What refuses a call before its body is read: a missing session on a
/// non-public service, and Boser's ownership of a method on its platforms.
pub(crate) struct AccessGate<S> {
    router: Arc<Router>,
    session_manager: Arc<S>,
    hardware_capabilities: HardwareCapabilities,
    i18n: I18nMarker,
}

impl<S> Clone for AccessGate<S> {
    fn clone(&self) -> Self {
        Self {
            router: self.router.clone(),
            session_manager: self.session_manager.clone(),
            hardware_capabilities: self.hardware_capabilities,
            i18n: self.i18n.clone(),
        }
    }
}

impl<S: SessionManager> AccessGate<S> {
    fn check(&self, path: &str, extensions: &http::Extensions) -> Result<(), ConnectError> {
        // A procedure nobody serves is the dispatcher's to refuse as unimplemented.
        // That holds only while the gate and the dispatcher read the same path;
        // nothing may rewrite the URI between them.
        if !self.router.has_method(path) {
            return Ok(());
        }
        // Before the session, and worded as the dispatcher refuses a procedure it lacks,
        // so the answer is the BMC application's from before them.
        if needs_i18n(path) && !self.i18n.is_set() {
            let procedure = path.strip_prefix('/').unwrap_or(path);
            return Err(ConnectError::unimplemented(format!(
                "method not found: {procedure}"
            )));
        }
        if !split_procedure(path).is_some_and(|(service, _)| is_public(service)) {
            extract_session::<S>(extensions)?;
        }
        if self.hardware_capabilities.boser_managed
            && managed_rpc_owner(path) == Some(ManagedRpcOwner::Boser)
        {
            return Err(ConnectError::unimplemented(BOSER_MANAGED_STATUS_MESSAGE));
        }
        Ok(())
    }
}

/// Middleware: answer a refused call in the caller's protocol without reading its body.
pub(crate) async fn access_gate<S: SessionManager>(
    State(gate): State<AccessGate<S>>,
    request: Request,
    next: Next,
) -> Response {
    if let Err(refusal) = gate.check(request.uri().path(), request.extensions()) {
        log_refusal(request.extensions(), request.uri().path(), &refusal);
        return refusal.into_http_response(request.headers()).map(Body::new);
    }
    next.run(request).await
}

pub(crate) struct GrpcWeb<
    T: BmcManager,
    S: SessionManager,
    U: FirmwareIndex,
    V: DisplayBacklightDriver,
> {
    manager: Arc<T>,
    session_manager: Arc<S>,
    system_upgrade_service: SystemUpgradeService<U, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    secret_store: Arc<RwLock<SecretStoreHandle>>,
    initial_setup: InitialSetup<T, U>,
    led_controller: LedController<T>,
    widget_registry: Arc<WidgetRegistry>,
    widget_coordinator: Arc<Coordinator>,
    led_coordinator: LedCoordinatorHandle,
    system_manager: SystemManager<V>,
    sound_controller: SoundController,
    alarm_controller: Option<AlarmController>,
    hardware_capabilities: HardwareCapabilities,
    upgrade_route: UpgradeRoute,
    i18n: I18nMarker,
}

impl<T: BmcManager, S: SessionManager, U: FirmwareIndex, V: DisplayBacklightDriver>
    GrpcWeb<T, S, U, V>
{
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        manager: Arc<T>,
        session_manager: Arc<S>,
        system_upgrade_service: SystemUpgradeService<U, T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        secret_store: Arc<RwLock<SecretStoreHandle>>,
        initial_setup: InitialSetup<T, U>,
        led_controller: LedController<T>,
        widget_registry: Arc<WidgetRegistry>,
        widget_coordinator: Arc<Coordinator>,
        led_coordinator: LedCoordinatorHandle,
        system_manager: SystemManager<V>,
        sound_controller: SoundController,
        alarm_controller: Option<AlarmController>,
        hardware_capabilities: HardwareCapabilities,
        upgrade_route: UpgradeRoute,
        i18n: I18nMarker,
    ) -> Self {
        Self {
            manager,
            session_manager,
            system_upgrade_service,
            config_handle,
            secret_store,
            initial_setup,
            led_controller,
            widget_registry,
            widget_coordinator,
            led_coordinator,
            system_manager,
            sound_controller,
            alarm_controller,
            hardware_capabilities,
            upgrade_route,
            i18n,
        }
    }

    /// The services behind the access gate; the session and cookie layers go on top.
    pub(crate) fn build(self) -> axum::Router {
        let wifi_joins = self.initial_setup.wifi_joins();
        let mut router = Router::new()
            .add_service(Arc::new(authentication::AuthenticationService::new(
                self.session_manager.clone(),
            )))
            .add_service(Arc::new(metadata::MetadataService::new(
                self.manager.clone(),
            )))
            .add_service(Arc::new(language::LanguageService))
            .add_service(Arc::new(initial_setup::InitialSetupService::new(
                self.manager.clone(),
                self.initial_setup,
            )))
            .add_service(Arc::new(network::NetworkService::new(
                self.manager.clone(),
                wifi_joins,
            )))
            .add_service(Arc::new(system::SystemService::new(
                self.manager,
                self.session_manager.clone(),
            )))
            .add_service(Arc::new(upgrade_service::UpgradeService::<_, _, S>::new(
                self.system_upgrade_service,
                self.config_handle.clone(),
                &self.hardware_capabilities,
                self.upgrade_route,
            )))
            .add_service(Arc::new(configuration_service::ConfigurationService::new(
                self.system_manager,
                self.sound_controller,
                self.config_handle.clone(),
                self.i18n.clone(),
            )))
            .add_service(Arc::new(scene_management::SceneManagementService::new(
                self.widget_registry,
                self.config_handle.clone(),
                self.secret_store.clone(),
                self.widget_coordinator,
                self.hardware_capabilities,
                self.led_coordinator,
            )))
            .add_service(Arc::new(account_management::AccountManagementService::new(
                self.config_handle,
                self.secret_store,
            )))
            .add_service(Arc::new(credential_management::CredentialManagementService))
            .add_service(Arc::new(led_test::LedTestService::new(self.led_controller)));
        if let Some(alarm_controller) = self.alarm_controller {
            router = router.add_service(Arc::new(alarm::AlarmService::new(alarm_controller)));
        }

        gated(
            router,
            self.session_manager,
            self.hardware_capabilities,
            self.i18n,
        )
    }
}

/// `router` as an axum router, answering through [`RpcLog`] and behind [`access_gate`].
fn gated<S: SessionManager>(
    router: Router,
    session_manager: Arc<S>,
    hardware_capabilities: HardwareCapabilities,
    i18n: I18nMarker,
) -> axum::Router {
    let router = Arc::new(router);
    let gate = AccessGate {
        router: router.clone(),
        session_manager,
        hardware_capabilities,
        i18n,
    };
    axum::Router::new()
        .fallback_service(ConnectRpcService::from_arc(router).with_interceptor(RpcLog))
        .layer(axum::middleware::from_fn_with_state(gate, access_gate::<S>))
}

#[derive(EnumMessage)]
pub(crate) enum GrpcError {
    #[strum(serialize = "Some of the fields are invalid")]
    BadRequest,
    #[strum(serialize = "Could not verify the credential")]
    CredentialUnverified,
}

impl Display for GrpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Write first serialization, if available (should be always true).
        // In an unlikely case of no serialization, report the platform as unknown.
        write!(
            f,
            "{}",
            self.get_serializations().first().unwrap_or(&"unknown")
        )
    }
}

#[cfg(test)]
mod tests;

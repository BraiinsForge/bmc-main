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

// gRPC handlers inherently return `Result<_, tonic::Status>` and `Status` is 176 bytes.
// Boxing it would be non-idiomatic — this is tonic's API, not a design choice we control.
#![expect(
    clippy::result_large_err,
    reason = "tonic::Status is inherent to the gRPC handler surface"
)]

use crate::BmcManager;
use crate::alarm::AlarmController;
use crate::backlight::DisplayBacklightDriver;
use crate::config::ConfigHandle;
use crate::initial_setup::InitialSetup;
use crate::led::LedController;
use crate::led_coordinator::LedCoordinatorHandle;
use crate::secret_store::SecretStoreHandle;
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::web::SessionManager;
use crate::web::session::extract_session;
use crate::widget::{Coordinator, WidgetRegistry};
use bmc_grpc::web;
use bmc_platform::HardwareCapabilities;
use bmc_upgrade::firmware::FirmwareIndex;
use std::fmt::Display;
use std::sync::Arc;
use strum::EnumMessage;
use tokio::sync::RwLock;
use tonic::service::Routes;
use tonic::{Status, body::Body, codegen::http::Request};
use tonic_middleware::InterceptorFor;
use tonic_middleware::RequestInterceptor;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower::Layer;
use tracing::debug;

mod account_management;
pub mod authentication;
mod credential_management;
mod logging;
mod metadata;
mod system;
use super::SystemUpgradeService;
mod alarm;
mod configuration_service;
mod hardware;
mod initial_setup;
mod led_test;
mod network;
mod scene_management;
mod shared;
mod upgrade_service;

use logging::GrpcLoggingLayer;

struct AuthInterceptor<S: SessionManager> {
    pub session_manager: Arc<S>,
}

impl<S: SessionManager> Clone for AuthInterceptor<S> {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
        }
    }
}

#[async_trait::async_trait]
impl<S: SessionManager> RequestInterceptor for AuthInterceptor<S> {
    async fn intercept(&self, req: Request<Body>) -> Result<Request<Body>, Status> {
        debug!("Intercepting request: {:?}", req);

        let _ = extract_session::<S>(req.extensions())?;

        Ok(req)
    }
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
    alarm_controller: AlarmController,
    hardware_capabilities: HardwareCapabilities,
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
        alarm_controller: AlarmController,
        hardware_capabilities: HardwareCapabilities,
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
        }
    }

    #[expect(clippy::too_many_lines)]
    pub(crate) fn build(self) -> Routes {
        let auth_interceptor = AuthInterceptor {
            session_manager: self.session_manager.clone(),
        };

        let upgrade_service = web::upgrade_service_server::UpgradeServiceServer::new(
            upgrade_service::UpgradeService::new(
                self.system_upgrade_service,
                self.config_handle.clone(),
                &self.hardware_capabilities,
            ),
        );

        let reflection_service = Builder::configure()
            .register_encoded_file_descriptor_set(web::FILE_DESCRIPTOR_SET)
            .build_v1alpha()
            .expect("BUG: Unable to decode reflection descriptor");

        let authentication_service =
            web::authentication_service_server::AuthenticationServiceServer::new(
                authentication::AuthenticationService::new(self.session_manager.clone()),
            );

        let metadata_service = web::metadata_service_server::MetadataServiceServer::new(
            metadata::MetadataService::new(self.manager.clone()),
        );

        let hardware_service = web::hardware_service_server::HardwareServiceServer::new(
            hardware::HardwareCapabilitiesService::new(self.hardware_capabilities),
        );

        let initial_setup_service =
            web::initial_setup_service_server::InitialSetupServiceServer::new(
                initial_setup::InitialSetupService::new(self.manager.clone(), self.initial_setup),
            );

        let network_service = web::network_service_server::NetworkServiceServer::new(
            network::NetworkService::new(self.manager.clone()),
        );

        let system_service = web::system_service_server::SystemServiceServer::new(
            system::SystemService::new(self.manager, self.session_manager),
        );

        let configuration_service =
            web::configuration_service_server::ConfigurationServiceServer::new(
                configuration_service::ConfigurationService::new(
                    self.system_manager,
                    self.sound_controller,
                    self.config_handle.clone(),
                ),
            );

        let scene_management_service =
            web::scene_management_service_server::SceneManagementServiceServer::new(
                scene_management::SceneManagementService::new(
                    self.widget_registry,
                    self.config_handle.clone(),
                    self.secret_store.clone(),
                    self.widget_coordinator.clone(),
                    self.hardware_capabilities,
                    self.led_coordinator,
                ),
            );

        let account_management_service =
            web::account_management_service_server::AccountManagementServiceServer::new(
                account_management::AccountManagementService::new(
                    self.config_handle,
                    self.secret_store,
                ),
            );

        let credential_management_service =
            web::credential_management_service_server::CredentialManagementServiceServer::new(
                credential_management::CredentialManagementService,
            );

        let alarm_service = web::alarm_service_server::AlarmServiceServer::new(
            alarm::AlarmService::new(self.alarm_controller),
        );

        let led_test_service = web::led_test_service_server::LedTestServiceServer::new(
            led_test::LedTestService::new(self.led_controller),
        );

        let logging_layer = GrpcLoggingLayer::new();

        // GrpcWebLayer is badly named, it's not a "layer", it's re-wrapper for other Services
        // All services requiring authentication have to be wrapped in GrpcWebLayer and use "InterceptorFor"
        Routes::new(GrpcWebLayer::new().layer(reflection_service))
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(authentication_service)),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(metadata_service)),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(hardware_service)),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        scene_management_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        configuration_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        account_management_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        credential_management_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        system_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        upgrade_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(initial_setup_service)),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(GrpcWebLayer::new().layer(InterceptorFor::new(
                        network_service,
                        auth_interceptor.clone(),
                    ))),
            )
            .add_service(
                tower::ServiceBuilder::new()
                    .layer(logging_layer.clone())
                    .service(
                        GrpcWebLayer::new()
                            .layer(InterceptorFor::new(alarm_service, auth_interceptor.clone())),
                    ),
            )
            .add_service(tower::ServiceBuilder::new().layer(logging_layer).service(
                GrpcWebLayer::new().layer(InterceptorFor::new(led_test_service, auth_interceptor)),
            ))
    }
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

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::config::ConfigHandle;
use crate::initial_setup::InitialSetup;
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::web::SessionManager;
use crate::web::session::extract_session;
use crate::widget_tasks::WidgetTasks;
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_grpc::web;
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

pub mod authentication;
mod metadata;
mod system;
use super::{AlarmController, SystemUpgradeService};
mod alarm;
mod configuration_service;
mod initial_setup;
mod network;
mod scene_management;
mod shared;
mod upgrade_service;

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
    display_controller: DisplayController,
    widget_tasks: WidgetTasks,
    initial_setup: InitialSetup<T, U>,
    system_manager: SystemManager<V>,
    sound_controller: SoundController,
    alarm_controller: AlarmController,
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
        display_controller: DisplayController,
        widget_tasks: WidgetTasks,
        initial_setup: InitialSetup<T, U>,
        system_manager: SystemManager<V>,
        sound_controller: SoundController,
        alarm_controller: AlarmController,
    ) -> Self {
        Self {
            manager,
            session_manager,
            system_upgrade_service,
            config_handle,
            display_controller,
            widget_tasks,
            initial_setup,
            system_manager,
            sound_controller,
            alarm_controller,
        }
    }

    pub(crate) fn build(self) -> Routes {
        let auth_interceptor = AuthInterceptor {
            session_manager: self.session_manager.clone(),
        };

        let upgrade_service = web::upgrade_service_server::UpgradeServiceServer::new(
            upgrade_service::UpgradeService::new(
                self.manager.clone(),
                self.system_upgrade_service,
                self.config_handle.clone(),
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
                    self.config_handle,
                    self.display_controller,
                    self.widget_tasks,
                ),
            );

        let alarm_service = web::alarm_service_server::AlarmServiceServer::new(
            alarm::AlarmService::new(self.alarm_controller),
        );

        // GrpcWebLayer is badly named, it's not a "layer", it's re-wrapper for other Services
        // All services requiring authentication have to be wrapped in GrpcWebLayer and use "InterceptorFor"
        Routes::new(GrpcWebLayer::new().layer(reflection_service))
            .add_service(GrpcWebLayer::new().layer(authentication_service))
            .add_service(GrpcWebLayer::new().layer(metadata_service))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                configuration_service,
                auth_interceptor.clone(),
            )))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                scene_management_service,
                auth_interceptor.clone(),
            )))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                system_service,
                auth_interceptor.clone(),
            )))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                network_service,
                auth_interceptor.clone(),
            )))
            .add_service(GrpcWebLayer::new().layer(InterceptorFor::new(
                upgrade_service,
                auth_interceptor.clone(),
            )))
            .add_service(GrpcWebLayer::new().layer(initial_setup_service))
            .add_service(
                GrpcWebLayer::new()
                    .layer(InterceptorFor::new(alarm_service, auth_interceptor.clone())),
            )
    }
}

#[derive(EnumMessage)]
pub(crate) enum GrpcError {
    #[strum(serialize = "Some of the fields are invalid")]
    BadRequest,
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

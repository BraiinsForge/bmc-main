// Copyright (C) 2025  Braiins Systems s.r.o.

mod captive_portal;
mod grpc;
mod http_server;
mod no_password;
mod session;

// TODO: display refactor — re-enable AlarmController/SoundController/SystemManager
// imports when display-dependent services are restored.
// use crate::alarm::AlarmController;
// use crate::sound::SoundController;
// use crate::system_manager::SystemManager;
use crate::backlight::DisplayBacklightDriver;
use crate::config::ConfigHandle;
use crate::initial_setup::InitialSetup;
use crate::led::LedController;
use crate::session::Manager as SessionManager;
use crate::widget::{Coordinator, WidgetRegistry};
// TODO: display refactor — re-enable once a replacement display layer ships
// use crate::widget_tasks::WidgetTasks;
use crate::{BmcManager, system_upgrade::SystemUpgradeService};
use anyhow::Result;
use axum::{ServiceExt, extract::Request, http::header::CONTENT_TYPE};
// TODO: display refactor
// use bmc_display::display_controller::DisplayController;
use bmc_upgrade::firmware::FirmwareIndex;
use std::{
    marker::PhantomData,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower::{Layer, steer::Steer};
use tower_http::normalize_path::NormalizePathLayer;

pub(crate) struct WebService<
    T: BmcManager,
    S: SessionManager,
    U: FirmwareIndex,
    V: DisplayBacklightDriver,
> {
    manager: Arc<T>,
    session_manager: Arc<S>,
    config: ServerConfig,
    system_upgrade_service: SystemUpgradeService<U, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    // TODO: display refactor
    // display_controller: DisplayController,
    // widget_tasks: WidgetTasks,
    initial_setup: InitialSetup<T, U>,
    led_controller: LedController<T>,
    widget_registry: Arc<WidgetRegistry>,
    widget_coordinator: Arc<Coordinator>,
    // TODO: display refactor — re-enable once display services are available
    // and remove _phantom_v.
    // system_manager: SystemManager<V>,
    // sound_controller: SoundController,
    // alarm_controller: AlarmController,
    _phantom_v: PhantomData<V>,
}

impl<T: BmcManager, S: SessionManager, U: FirmwareIndex, V: DisplayBacklightDriver>
    WebService<T, S, U, V>
{
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        manager: Arc<T>,
        session_manager: Arc<S>,
        config: ServerConfig,
        system_upgrade_service: SystemUpgradeService<U, T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        // TODO: display refactor
        // display_controller: DisplayController,
        // widget_tasks: WidgetTasks,
        initial_setup: InitialSetup<T, U>,
        led_controller: LedController<T>,
        widget_registry: Arc<WidgetRegistry>,
        widget_coordinator: Arc<Coordinator>,
        // TODO: display refactor — re-enable when display services are available
        // system_manager: SystemManager<V>,
        // sound_controller: SoundController,
        // alarm_controller: AlarmController,
    ) -> Self {
        Self {
            manager,
            session_manager,
            config,
            system_upgrade_service,
            config_handle,
            // TODO: display refactor
            // display_controller,
            // widget_tasks,
            initial_setup,
            led_controller,
            widget_registry,
            widget_coordinator,
            // system_manager,
            // sound_controller,
            // alarm_controller,
            _phantom_v: PhantomData,
        }
    }

    pub(crate) async fn run(self, listener: TcpListener) -> Result<()> {
        let http_router = http_server::HttpServer::new(self.config, self.manager.clone()).build();
        let grpc_router = grpc::GrpcWeb::<_, _, _, V>::new(
            self.manager.clone(),
            self.session_manager.clone(),
            self.system_upgrade_service,
            self.config_handle,
            // TODO: display refactor
            // self.display_controller,
            // self.widget_tasks,
            self.initial_setup,
            self.led_controller,
            self.widget_registry,
            self.widget_coordinator,
            // TODO: display refactor — re-enable when display services are available
            // self.system_manager,
            // self.sound_controller,
            // self.alarm_controller,
        )
        .build()
        .into_axum_router()
        .layer(session::SessionLayer::new(self.session_manager.clone()))
        .layer(no_password::NoPasswordLayer::new(self.session_manager))
        .layer(tower_cookies::CookieManagerLayer::new());

        // combine grpc and http router into one service
        let service = Steer::new(
            vec![http_router, grpc_router],
            |req: &Request, _services: &[_]| {
                // grpc service -> 1
                // http service -> 0
                usize::from(
                    req.headers()
                        .get(CONTENT_TYPE)
                        .map(axum::http::HeaderValue::as_bytes)
                        .filter(|content_type| content_type.starts_with(b"application/grpc"))
                        .is_some(),
                )
            },
        );

        let service = NormalizePathLayer::trim_trailing_slash().layer(service);
        let service =
            ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(service);

        axum::serve(listener, service)
            .with_graceful_shutdown(async move { self.manager.handle_graceful_shutdown().await })
            .await?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub www_root_path: PathBuf,
    pub www_assets_path: PathBuf,
    pub www_var_path: PathBuf,
    pub grpc_address: std::net::SocketAddr,
}

impl ServerConfig {
    pub const WWW_ROOT_PATH: &str = "/www/bmc";

    #[must_use]
    pub fn set_www_root_path(mut self, www_root_path: PathBuf) -> Self {
        self.www_root_path = www_root_path;
        self
    }

    #[must_use]
    pub fn set_www_assets_path(mut self, www_assets_path: PathBuf) -> Self {
        self.www_assets_path = www_assets_path;
        self
    }

    #[must_use]
    pub fn set_www_var_path(mut self, www_var_path: PathBuf) -> Self {
        self.www_var_path = www_var_path;
        self
    }

    #[must_use]
    pub fn set_grpc_address(mut self, address: std::net::SocketAddr) -> Self {
        self.grpc_address = address;
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            www_root_path: Self::WWW_ROOT_PATH.into(),
            www_assets_path: PathBuf::from(Self::WWW_ROOT_PATH).join("assets"),
            www_var_path: PathBuf::from(Self::WWW_ROOT_PATH).join("var"),
            grpc_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50051),
        }
    }
}

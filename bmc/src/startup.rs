// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::config::DisplayConfigHandle;
use crate::display_tasks::DisplayTasks;
use crate::initial_setup::InitialSetup;
use crate::system_upgrade::{StateService, SystemUpgradeService};
use anyhow::Result;
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::info;

use crate::manager::BmcManager;
use crate::web::{ServerConfig, WebService};

#[derive(Debug)]
pub struct App<T, V>
where
    T: BmcManager,
    V: FirmwareIndex,
{
    listener: TcpListener,
    manager: Arc<T>,
    session_manager: Arc<T::SessionManager>,
    config: Configuration,
    display_tasks: DisplayTasks,
    system_upgrade_service: SystemUpgradeService<V, T>,
    display_config_handle: Arc<RwLock<DisplayConfigHandle>>,
    display_controller: DisplayController,
    initial_setup: InitialSetup<T>,
}

impl<T, V> App<T, V>
where
    T: BmcManager,
    V: FirmwareIndex,
{
    pub async fn init<U: DisplayBacklightDriver>(
        config: Configuration,
        manager: Arc<T>,
        session_manager: T::SessionManager,
        display_driver: DisplayDriver<U>,
        firmware_resolver: FirmwareResolver<V>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        let state_service = StateService::new();

        let system_upgrade_service = SystemUpgradeService::new(
            firmware_resolver,
            &config.upgrade_image_path,
            manager.clone(),
            state_service.clone(),
        );

        let mut display_config_handle =
            DisplayConfigHandle::new(config.display_config_path.clone());
        display_config_handle.init().await;

        let display_controller = display_driver.display_controller.clone();
        display_controller.populate_widgets(display_config_handle.scenes());

        let initial_setup = InitialSetup::new(manager.clone(), Arc::new(AtomicBool::new(false)));

        let display_tasks = DisplayTasks::new(
            display_controller.clone(),
            state_service.subscribe(),
            manager.watch_timezone_updates(),
            initial_setup.subscribe(),
        );

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            display_tasks,
            system_upgrade_service,
            display_config_handle: Arc::new(RwLock::new(display_config_handle)),
            display_controller,
            initial_setup,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        self.display_tasks.spawn();
        self.system_upgrade_service.init().await;

        WebService::new(
            self.manager.clone(),
            self.session_manager.clone(),
            self.config.server_config,
            self.system_upgrade_service,
            self.display_config_handle.clone(),
            self.display_controller,
            self.initial_setup,
        )
        .run(self.listener)
        .await?;

        Ok(())
    }

    pub fn port(&self) -> Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }
}

#[derive(Debug)]
pub struct Configuration {
    pub address: SocketAddr,
    pub server_config: ServerConfig,
    pub upgrade_image_path: PathBuf,
    pub display_config_path: PathBuf,
}

impl Configuration {
    const UPGRADE_IMAGE_PATH: &'static str = "/tmp/firmware.tar";
    const DISPLAY_CONFIG_PATH: &'static str = "/etc/bmc_display.json";
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 9090),
            server_config: ServerConfig::default(),
            upgrade_image_path: PathBuf::from(Self::UPGRADE_IMAGE_PATH),
            display_config_path: Self::DISPLAY_CONFIG_PATH.into(),
        }
    }
}

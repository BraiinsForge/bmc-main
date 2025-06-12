// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use crate::display::DisplayTasks;
use crate::system_upgrade::{StateService, SystemUpgradeService};
use anyhow::Result;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use tokio::net::TcpListener;
use tracing::info;

use crate::manager::BmcManager;
use crate::web::{ServerConfig, WebService};

#[derive(Debug)]
pub struct App<T, U, V>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
    V: FirmwareIndex,
{
    listener: TcpListener,
    manager: Arc<T>,
    session_manager: Arc<T::SessionManager>,
    config: Configuration,
    display_tasks: DisplayTasks<U>,
    system_upgrade_service: SystemUpgradeService<V, T>,
}

impl<T, U, V> App<T, U, V>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
    V: FirmwareIndex,
{
    pub async fn init(
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

        let display_tasks = DisplayTasks::init(
            display_driver,
            state_service,
            manager.watch_timezone_updates(),
        )?;

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            display_tasks,
            system_upgrade_service,
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
}

impl Configuration {
    const UPGRADE_IMAGE_PATH: &'static str = "/tmp/firmware.tar";
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 9090),
            server_config: ServerConfig::default(),
            upgrade_image_path: PathBuf::from(Self::UPGRADE_IMAGE_PATH),
        }
    }
}

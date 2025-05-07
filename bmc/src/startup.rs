// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Result;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use tokio::net::TcpListener;
use tracing::info;

use crate::manager::BmcManager;
use crate::web::{ServerConfig, WebService};

#[derive(Debug)]
pub struct App<T, U>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
{
    listener: TcpListener,
    manager: Arc<T>,
    session_manager: Arc<T::SessionManager>,
    config: Configuration,
    display_handle: Arc<U>,
}

impl<T, U> App<T, U>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
{
    pub async fn init(
        config: Configuration,
        manager: Arc<T>,
        session_manager: T::SessionManager,
        display_handle: Arc<U>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            display_handle,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        self.display_handle.init()?;

        WebService::new(
            self.manager.clone(),
            self.session_manager.clone(),
            self.config.server_config,
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

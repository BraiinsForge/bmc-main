// Copyright (C) 2025  Braiins Systems s.r.o.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Result;
use bmc_display::display_driver::DisplayHandle;
use tokio::net::TcpListener;
use tracing::info;

use crate::manager::BmcManager;
use crate::web::{ServerConfig, WebService};

#[derive(Debug)]
pub struct App<T, U>
where
    T: BmcManager,
    U: DisplayHandle,
{
    listener: TcpListener,
    manager: Arc<T>,
    config: Configuration,
    display_handle: Arc<U>,
}

impl<T, U> App<T, U>
where
    T: BmcManager,
    U: DisplayHandle,
{
    pub async fn init(
        config: Configuration,
        manager: Arc<T>,
        display_handle: Arc<U>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        Ok(Self {
            listener,
            manager,
            config,
            display_handle,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        self.display_handle.init()?;

        WebService::new(self.manager.clone(), self.config.server_config)
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
}

impl Configuration {}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 9090),
            server_config: ServerConfig::default(),
        }
    }
}

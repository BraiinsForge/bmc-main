// Copyright (C) 2025  Braiins Systems s.r.o.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use crate::manager::BmcManager;
use crate::web::{ServerConfig, WebService};

pub struct App<T>
where
    T: BmcManager,
{
    listener: TcpListener,
    manager: Arc<T>,
    config: Configuration,
}

impl<T> App<T>
where
    T: BmcManager,
{
    pub async fn init(config: Configuration, manager: Arc<T>) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        Ok(Self {
            listener,
            manager,
            config,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        WebService::new(self.manager.clone(), self.config.server_config)
            .run(self.listener)
            .await?;

        Ok(())
    }

    pub fn port(&self) -> Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }
}

pub struct Configuration {
    pub address: SocketAddr,
    pub server_config: ServerConfig,
}

impl Configuration {}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6060),
            server_config: Default::default(),
        }
    }
}

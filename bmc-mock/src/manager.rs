// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::anyhow;
use bmc::manager::NetworkProtocolConfig;
use bmc_platform::BmcPlatform;
use bmc_shared_time::time::Timezone;
use std::{
    net::IpAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::info;

use crate::{MockSessionManager, mockfs::MockFs};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Manager {
    mockfs: MockFs,
    pub session_manager: MockSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    password: Arc<Mutex<Option<String>>>,
    mac_address: String,
    ip_address: IpAddr,
    hostname: String,
    network_config: Arc<Mutex<NetworkProtocolConfig>>,
}

impl Manager {
    #[must_use]
    pub fn new(
        mockfs: MockFs,
        session_manager: MockSessionManager,
        password: Arc<Mutex<Option<String>>>,
        hostname: String,
        mac_address: String,
        ip_address: IpAddr,
    ) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(Timezone::default());
        Self {
            mockfs,
            session_manager,
            timezone_sender,
            password,
            hostname,
            mac_address,
            network_config: Arc::new(Mutex::new(NetworkProtocolConfig::Dhcp)),
            ip_address,
        }
    }
}

#[async_trait::async_trait]
impl bmc::BmcManager for Manager {
    type Error = Error;
    type SessionManager = MockSessionManager;

    fn version(&self) -> String {
        "0.1.0".to_owned()
    }

    fn platform(&self) -> BmcPlatform {
        BmcPlatform::BraiinsBmc
    }

    async fn upgrade(&self, keep_settings: bool, _upgrade_image_path: &Path) -> anyhow::Result<()> {
        info!(
            "Performing system upgrade (keep_settings={})...",
            keep_settings
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(())
    }

    async fn check_and_remove_upgrade_marker(&self) -> bool {
        self.mockfs.upgrade_result().exists()
    }

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error> {
        let current_password = self.password.lock().expect("BUG: cannot lock password");

        let matches = match (password, current_password.as_deref()) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(password), Some(current_password)) => password == current_password,
        };

        Ok(matches)
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Setting password to {:?}", password);

        let mut guard = self.password.lock().expect("BUG: cannot lock password");
        *guard = password;

        Ok(())
    }

    fn timezone(&self) -> Timezone {
        self.timezone_sender.borrow().clone()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()> {
        self.timezone_sender.send_if_modified(|current| {
            if *current != timezone {
                *current = timezone;
                return true;
            }
            false
        });

        Ok(())
    }

    fn watch_timezone_updates(&self) -> tokio::sync::watch::Receiver<Timezone> {
        self.timezone_sender.subscribe()
    }

    async fn is_factory_default(&self) -> bool {
        let result = self.mockfs.factory_default().exists();
        info!("Checking if factory default... {result}");
        result
    }

    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error> {
        info!(hard, "Performing factory reset...");
        Ok(())
    }

    async fn hostname(&self) -> Option<String> {
        Some(self.hostname.clone())
    }

    fn mac_address(&self) -> Option<String> {
        Some(self.mac_address.to_ascii_lowercase().clone())
    }

    fn ip_address(&self) -> Option<IpAddr> {
        Some(self.ip_address)
    }

    async fn network_config(&self) -> Option<NetworkProtocolConfig> {
        self.network_config.lock().ok().map(|config| config.clone())
    }

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> anyhow::Result<()> {
        let mut network_config = self
            .network_config
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock for network config"))?;

        *network_config = config;
        Ok(())
    }
}

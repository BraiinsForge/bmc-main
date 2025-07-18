// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{MockSessionManager, mockfs::MockFs};
use anyhow::anyhow;
use bmc::manager::{BmcState, InitialSetupError, NetworkProtocolConfig, WifiNetworkConfig};
use bmc_platform::{BmcPlatform, BosVersion};
use bmc_shared_ii_net::wifi::{EncryptionType, WifiScanItem};
use bmc_shared_time::time::Timezone;
use rand::Rng;
use std::{
    net::IpAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::info;
use tracing::log::warn;

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
    port: u16,
    connected_wifi: Arc<tokio::sync::Mutex<Option<WifiNetworkConfig>>>,
}

impl Manager {
    const WIFI_SSID: &str = "BMC 5a200d";

    #[must_use]
    pub fn new(
        mockfs: MockFs,
        session_manager: MockSessionManager,
        password: Arc<Mutex<Option<String>>>,
        hostname: String,
        mac_address: String,
        ip_address: IpAddr,
        port: u16,
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
            port,
            connected_wifi: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl bmc::BmcManager for Manager {
    type Error = Error;
    type SessionManager = MockSessionManager;

    async fn version(&self) -> BosVersion {
        BosVersion::new(&25, &7)
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

    async fn ip_address(&self) -> Option<IpAddr> {
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

    async fn captive_portal_redirect_host(&self) -> Option<String> {
        let port = self.port;
        Some(format!("localhost:{port}"))
    }

    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError> {
        if self.device_state().await != BmcState::FactoryDefault {
            return Err(InitialSetupError::NotSupported);
        }

        // Simulate connecting to WiFi
        tokio::time::sleep(Duration::from_secs(5)).await;

        info!("Setting up WiFi");
        let mut wifi = self.connected_wifi.lock().await;

        self.update_device_state()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;

        *wifi = Some(config);

        Ok(())
    }

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError> {
        info!("Reverting WiFi setup...");
        *self.connected_wifi.lock().await = None;
        Ok(())
    }

    async fn wifi_scan(&self) -> anyhow::Result<Vec<WifiScanItem>> {
        info!("Scanning WiFi...");

        let mut rng = rand::rng();
        let mut signal_strength = || rng.random_range(-90..=-50);

        let mut wifi_list = vec![
            WifiScanItem {
                ssid: "braiins".to_owned(),
                encryption_type: EncryptionType::Wpa2,
                signal_level: signal_strength(),
            },
            WifiScanItem {
                ssid: "dummy-wep".to_owned(),
                encryption_type: EncryptionType::Wep,
                signal_level: signal_strength(),
            },
            WifiScanItem {
                ssid: "dummy-wpa".to_owned(),
                encryption_type: EncryptionType::Wpa,
                signal_level: signal_strength(),
            },
            WifiScanItem {
                ssid: "dummy-none".to_owned(),
                encryption_type: EncryptionType::None,
                signal_level: signal_strength(),
            },
        ];

        wifi_list.sort_by(|a, b| {
            b.signal_level
                .cmp(&a.signal_level)
                .then_with(|| a.ssid.cmp(&b.ssid))
        });

        // return random number of elements
        let count = rng.random_range(0..=3);
        Ok(wifi_list.split_off(count))
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        info!("Performing reboot...");
        Ok(())
    }

    async fn device_state(&self) -> BmcState {
        if self.mockfs.factory_default().exists() {
            BmcState::FactoryDefault
        } else if self.mockfs.pending_setup().exists() {
            BmcState::SetupPending
        } else {
            BmcState::Operational
        }
    }

    async fn update_device_state(&self) -> anyhow::Result<()> {
        match self.device_state().await {
            BmcState::FactoryDefault => {
                self.mockfs.add_or_remove_factory_default_flag(false)?;
                self.mockfs.add_or_remove_setup_pending_flag(true)?;
            }
            BmcState::SetupPending => {
                self.mockfs.add_or_remove_setup_pending_flag(false)?;
            }
            BmcState::Operational => (),
        }

        Ok(())
    }

    fn wifi_ssid(&self) -> String {
        Self::WIFI_SSID.to_owned()
    }

    async fn init_wifi_ap(&self) -> Result<(), Self::Error> {
        warn!("Wifi init not implemented");
        Ok(())
    }

    async fn wifi_save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<(), Self::Error> {
        info!(
            "Connect to wifi network {ssid}:{:?}:{:?}",
            password, encryption
        );
        Ok(())
    }
}

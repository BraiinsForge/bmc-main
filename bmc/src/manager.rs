// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    fmt::Debug,
    net::{IpAddr, Ipv4Addr},
    path::Path,
};

use bmc_platform::BmcPlatform;
use strum::{Display, EnumString};
use thiserror::Error;
use tokio::sync::watch;

use bmc_shared_time::time::Timezone;

#[async_trait::async_trait]
pub trait BmcManager: Sync + Send + 'static + Debug {
    type SessionManager: crate::session::Manager;
    type Error: std::error::Error + Send + Sync;

    fn version(&self) -> String;

    fn platform(&self) -> BmcPlatform;

    async fn upgrade(&self, keep_settings: bool, upgrade_image_path: &Path) -> anyhow::Result<()>;

    // Checks if a system upgrade was performed
    async fn check_and_remove_upgrade_marker(&self) -> bool;

    fn session_manager(&self) -> Self::SessionManager;

    async fn has_password(&self) -> Result<bool, Self::Error> {
        self.check_password(None)
            .await
            .map(|has_no_password| !has_no_password)
    }

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error>;

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error>;

    fn timezone(&self) -> Timezone;

    fn timezone_list(&self) -> impl Iterator<Item = Timezone> {
        Timezone::timezone_list()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()>;

    fn watch_timezone_updates(&self) -> watch::Receiver<Timezone>;

    // Checks if the system is in factory default state
    async fn is_factory_default(&self) -> bool;

    /// Execute factory reset and reboot
    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error>;

    async fn hostname(&self) -> Option<String>;

    fn mac_address(&self) -> Option<String>;

    fn ip_address(&self) -> Option<IpAddr>;

    async fn network_config(&self) -> Option<NetworkProtocolConfig>;

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> anyhow::Result<()>;

    async fn captive_portal_redirect_host(&self) -> Option<String>;

    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError>;

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError>;

    async fn wifi_scan(&self) -> anyhow::Result<Vec<WifiScanItem>>;

    async fn reboot(&self) -> anyhow::Result<()>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkProtocolConfig {
    Dhcp,
    Static(NetworkProtocolConfigStatic),
}

impl Default for NetworkProtocolConfig {
    fn default() -> Self {
        Self::Dhcp
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkProtocolConfigStatic {
    pub address: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns_servers: Vec<Ipv4Addr>,
}

#[derive(Debug, Display, Eq, PartialEq)]
pub enum NetworkProtocol {
    Dhcp,
    Static,
}

impl From<NetworkProtocolConfig> for NetworkProtocol {
    fn from(network_config: NetworkProtocolConfig) -> Self {
        match network_config {
            NetworkProtocolConfig::Dhcp => NetworkProtocol::Dhcp,
            NetworkProtocolConfig::Static(_) => NetworkProtocol::Static,
        }
    }
}

#[derive(Error, Debug)]
pub enum InitialSetupError {
    #[error("Initial setup is not supported")]
    NotSupported,
    #[error("Unexpected error during initial setup. {0}")]
    UnexpectedFailure(String),
    #[error("Connection to wifi was not successful. {0}")]
    WifiConnectionFailure(String),
}

#[derive(Debug)]
pub struct WifiNetworkConfig {
    pub ssid: String,
    pub password: Option<String>,
    pub encryption: EncryptionType,
}

#[derive(Debug, EnumString, PartialEq, Eq, PartialOrd, Ord, Default, Clone, Copy, Display)]
pub enum EncryptionType {
    #[default]
    None,
    Wep,
    WepShared,
    Wpa,
    Wpa1_2,
    Wpa2,
    Wpa2_3,
}

#[derive(Debug, Clone)]
pub struct WifiScanItem {
    pub ssid: String,
    pub signal_level: i32,
    pub encryption_type: EncryptionType,
}

impl WifiScanItem {
    #[must_use]
    pub fn new(ssid: String, signal_level: i32, encryption_type: EncryptionType) -> Self {
        Self {
            ssid,
            signal_level,
            encryption_type,
        }
    }

    #[must_use]
    pub fn signal_strength(&self) -> SignalStrength {
        SignalStrength::new(self.signal_level)
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Default, Display, PartialOrd, Ord)]
pub enum SignalStrength {
    #[default]
    Offline,
    Low,
    Fair,
    Excellent,
}

impl SignalStrength {
    #[must_use]
    pub fn new(signal: i32) -> Self {
        match signal {
            0 => SignalStrength::Offline,
            x if x >= -60 => SignalStrength::Excellent,
            x if x >= -75 => SignalStrength::Fair,
            _ => SignalStrength::Low,
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use crate::bootloader_config::BootloaderConfig;
use anyhow::anyhow;
pub use bmc_nix::service_orchestrator::SERVICE_NAME_ENV;
use bmc_nix::service_orchestrator::upgraded_service_marker;
use bmc_platform::{BosPlatform, BosVersion};
use bmc_shared_ii_net::MacAddr;
use bmc_shared_ii_net::wifi::{EncryptionType, WifiScanItem, WifiStatus};
use bmc_shared_time::time::Timezone;
use bmc_support::SupportArchiveFormat;
use std::time::Duration;
use std::{
    fmt::Debug,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};
use strum::Display;
use thiserror::Error;
use tokio::sync::watch;
use tracing::{error, info};

/// Failure handing a firmware image off to the platform upgrade
/// mechanism. `InvalidImage` is permanent — the image is incompatible,
/// unsigned, or signed with the wrong key — so the UI can tell the user
/// to pick a different image instead of retrying the same one; every other
/// handoff failure is transient-ish and carries its own detail.
#[derive(Debug, Error)]
pub enum UpgradeError {
    #[error("Invalid firmware image")]
    InvalidImage,
    #[error("{0}")]
    Failed(String),
}

/// Marker the service orchestrator publishes when an activation
/// upgrades this process in place, or `None` when nothing started it
/// as a service — a hand-run binary has no activation to hear from.
#[must_use]
pub fn service_upgrade_marker_path() -> Option<PathBuf> {
    std::env::var(SERVICE_NAME_ENV)
        .ok()
        .map(|service| upgraded_service_marker(&service))
}

/// Outcome of consuming the one-shot post-upgrade marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeMarker {
    Absent,
    Consumed,
    RemovalFailed,
}

/// Consume a one-shot upgrade marker without conflating absence and failure.
pub async fn consume_upgrade_marker(path: &Path) -> UpgradeMarker {
    match tokio::fs::remove_file(path).await {
        Ok(()) => UpgradeMarker::Consumed,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => UpgradeMarker::Absent,
        Err(err) => {
            error!(
                error = %err,
                path = %path.display(),
                "failed to remove upgrade marker file"
            );
            UpgradeMarker::RemovalFailed
        }
    }
}

#[async_trait::async_trait]
pub trait BmcManager: Sync + Send + 'static + Debug {
    type SessionManager: crate::session::Manager;
    type Error: std::error::Error + Send + Sync;

    async fn version(&self) -> Option<BosVersion>;

    fn platform(&self) -> BosPlatform;

    /// Hand the image off to the platform upgrade mechanism. `Ok(())` means
    /// the handoff was accepted, not that the upgrade completed. Progress
    /// lines from the upgrade process are forwarded through `progress` when
    /// provided.
    async fn upgrade(
        &self,
        keep_settings: bool,
        upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError>;

    /// Consume the post-upgrade marker without conflating absence and failure.
    async fn consume_upgrade_marker(&self) -> UpgradeMarker;

    /// Consume the marker announcing that activation upgraded this service
    /// in place. A package upgrade never reboots,
    /// so the restart is the only signal that its run finished.
    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker;

    fn session_manager(&self) -> Self::SessionManager;

    async fn has_password(&self) -> Result<bool, Self::Error> {
        self.check_password(None)
            .await
            .map(|has_no_password| !has_no_password)
    }

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error>;

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error>;

    fn timezone(&self) -> Timezone;

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()>;

    fn watch_timezone_updates(&self) -> watch::Receiver<Timezone>;

    /// Watch WiFi-reconfiguration (setup) mode. `true` while setup mode is
    /// active. Used to drive the settings-tray `wifi_ap` broadcast.
    fn watch_wifi_reconfig(&self) -> watch::Receiver<bool>;

    // Checks if the system is in factory default state
    async fn is_factory_default(&self) -> bool;

    /// Execute factory reset and reboot
    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error>;

    // Checks if the system is in setup pending state
    async fn is_setup_pending(&self) -> bool;

    // Checks if the system is in wifi reconfiguration state
    async fn is_wifi_reconfig(&self) -> bool;

    // Enters wifi reconfiguration mode (enables AP + captive portal without factory reset)
    async fn enter_wifi_reconfig(&self) -> Result<(), InitialSetupError>;

    // Exits wifi reconfiguration mode and returns to operational
    async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError>;

    async fn hostname(&self) -> Option<String>;

    fn mac_address(&self) -> Option<String>;

    async fn ip_address(&self) -> Option<IpAddr>;

    async fn network_config(&self) -> Option<NetworkProtocolConfig>;

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> anyhow::Result<()>;

    async fn captive_portal_redirect_host(&self) -> Option<String>;

    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError>;

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError>;

    async fn wifi_scan(&self) -> anyhow::Result<Vec<WifiScanItem>>;

    fn subscribe_wifi_events(&self) -> tokio::sync::broadcast::Receiver<WifiEvent>;

    async fn reboot(&self) -> anyhow::Result<()>;

    async fn device_state(&self) -> BmcState;

    async fn update_device_state(&self) -> anyhow::Result<()>;

    async fn wait_for_wifi_ssid(
        &self,
        max_retry: usize,
        retry_delay: Duration,
    ) -> anyhow::Result<String> {
        for _ in 1..=max_retry {
            match self.wifi_ssid().await {
                Ok(wifi_ssid) => return Ok(wifi_ssid),
                Err(err) => {
                    info!(
                        "Wi-Fi interface not initialized yet: {err}, retrying in {} seconds",
                        retry_delay.as_secs()
                    );
                    tokio::time::sleep(retry_delay).await;
                }
            }
        }

        Err(anyhow!("Timeout waiting for Wi-Fi SSID."))
    }

    async fn wifi_ssid(&self) -> anyhow::Result<String>;

    async fn init_wifi_ap(&self) -> Result<(), Self::Error>;

    async fn wifi_save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<(), Self::Error>;

    async fn wifi_status(&self) -> anyhow::Result<WifiData>;

    async fn wifi_saved_networks(&self) -> anyhow::Result<Vec<WifiStatus>>;

    // Executes the function once bmc is shutting down
    async fn handle_graceful_shutdown(&self);

    async fn support_archive(&self, format: SupportArchiveFormat) -> Result<Vec<u8>, Self::Error>;

    /// Sync bootloader configuration to persistent storage (e.g., U-Boot environment).
    ///
    /// Platform-specific implementations (e.g., OpenWrt) write to U-Boot env,
    /// while other platforms (e.g., mock) may implement this as a no-op.
    async fn sync_boot_environment(&self, config: &BootloaderConfig) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
pub enum WifiEvent {
    ScanStarted,
    ScanEnded,
}

#[derive(Debug, Display, PartialEq)]
pub enum BmcState {
    #[strum(serialize = "factory default")]
    FactoryDefault,
    #[strum(serialize = "device setup")]
    SetupPending,
    #[strum(serialize = "operational")]
    Operational,
    #[strum(serialize = "wifi reconfiguration")]
    WifiReconfiguration,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub enum NetworkProtocolConfig {
    #[default]
    Dhcp,
    Static(NetworkProtocolConfigStatic),
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

#[derive(Debug)]
pub struct NetworkInfo {
    pub interface_name: String,
    pub mac_address: Option<String>,
    pub hostname: Option<String>,
    pub protocol: Option<NetworkProtocol>,
    pub dns_servers: Vec<Ipv4Addr>,
    pub networks: Vec<IpNetwork>,
    pub default_gateway: Option<Ipv4Addr>,
}

#[derive(Debug, Clone)]
pub struct IpNetwork {
    pub address: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IfaceData {
    pub ip: Option<IpAddr>,
    pub mac: Option<MacAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiData {
    pub iface: IfaceData,
    pub status: WifiStatus,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{UpgradeMarker, consume_upgrade_marker};

    #[tokio::test]
    async fn consuming_upgrade_marker_distinguishes_all_outcomes() {
        let dir = tempfile::tempdir().expect("BUG: create temporary marker directory");
        let marker = dir.path().join("upgrade_result");
        fs::write(&marker, "success").expect("BUG: create upgrade marker");

        assert_eq!(
            consume_upgrade_marker(&marker).await,
            UpgradeMarker::Consumed
        );
        assert_eq!(consume_upgrade_marker(&marker).await, UpgradeMarker::Absent);
        assert_eq!(
            consume_upgrade_marker(dir.path()).await,
            UpgradeMarker::RemovalFailed
        );
    }
}

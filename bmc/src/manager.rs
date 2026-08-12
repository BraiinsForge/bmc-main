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
use bmc_net::NetworkManager;
pub use bmc_nix::service_orchestrator::SERVICE_NAME_ENV;
use bmc_nix::service_orchestrator::upgraded_service_marker;
use bmc_platform::{BosPlatform, BosVersion};
use bmc_shared_time::time::Timezone;
use bmc_support::SupportArchiveFormat;
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};
use thiserror::Error;
use tokio::sync::watch;
use tracing::error;

pub use bmc_net_types::network::{
    BmcState, IfaceData, InitialSetupError, IpNetwork, NetworkInfo, NetworkProtocol,
    NetworkProtocolConfig, NetworkProtocolConfigStatic, WifiData, WifiEvent, WifiNetworkConfig,
};

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

    /// The platform network facade: ethernet config, optional WiFi (via
    /// [`NetworkManager::wifi`]), and the provisioning state machine.
    fn network_manager(&self) -> &dyn NetworkManager;

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

    /// Execute factory reset and reboot
    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error>;

    async fn reboot(&self) -> anyhow::Result<()>;

    // Executes the function once bmc is shutting down
    async fn handle_graceful_shutdown(&self);

    async fn support_archive(&self, format: SupportArchiveFormat) -> Result<Vec<u8>, Self::Error>;

    /// Sync bootloader configuration to persistent storage (e.g., U-Boot environment).
    ///
    /// Platform-specific implementations (e.g., OpenWrt) write to U-Boot env,
    /// while other platforms (e.g., mock) may implement this as a no-op.
    async fn sync_boot_environment(&self, config: &BootloaderConfig) -> Result<(), Self::Error>;
}

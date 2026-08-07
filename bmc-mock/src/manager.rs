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

use crate::pacing::UpgradePacing;
use crate::{MockSessionManager, mockfs::MockFs};
use anyhow::anyhow;
use bmc::bootloader_config::BootloaderConfig;
use bmc::manager::{
    BmcState, IfaceData, InitialSetupError, NetworkProtocolConfig, UpgradeError, UpgradeMarker,
    WifiData, WifiEvent, WifiNetworkConfig,
};
use bmc_nix::progress::{ActiveDownload, ProgressEvent};
use bmc_platform::{BosPlatform, BosVersion};
use bmc_shared_ii_net::MacAddr;
use bmc_shared_ii_net::wifi::{
    EncryptionType, WifiConfiguration, WifiLinkState, WifiMode, WifiScanItem, WifiStatus,
};
use bmc_shared_time::time::Timezone;
use bmc_support::SupportArchiveFormat;
use rand::Rng;
use std::io::{Cursor, Write};
use std::{
    net::IpAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::signal;
use tracing::info;
use tracing::log::warn;
use zip::write::SimpleFileOptions;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}

#[derive(Debug)]
pub struct Manager {
    mockfs: MockFs,
    platform: BosPlatform,
    pub session_manager: MockSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    password: Arc<Mutex<Option<String>>>,
    mac_address: String,
    ip_address: IpAddr,
    hostname: String,
    network_config: Arc<Mutex<NetworkProtocolConfig>>,
    port: u16,
    connected_wifi: Arc<tokio::sync::Mutex<Option<WifiNetworkConfig>>>,
    wifi_event_sender: tokio::sync::broadcast::Sender<WifiEvent>,
    wifi_reconfig_sender: tokio::sync::watch::Sender<bool>,
    pacing: UpgradePacing,
    stop: Arc<tokio::sync::Notify>,
}

impl Manager {
    const WIFI_SSID: &str = "BMC 5a200d";
    const WIFI_EVENTS_CAPACITY: usize = 10;
    const DUMMY_SUPPORT_FILE_NAME: &'static str = "hello_deck.txt";
    const DUMMY_SUPPORT_FILE_CONTENT: &'static str = "wake up Neo";

    #[must_use]
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        mockfs: MockFs,
        session_manager: MockSessionManager,
        password: Arc<Mutex<Option<String>>>,
        hostname: String,
        mac_address: String,
        ip_address: IpAddr,
        port: u16,
        platform: BosPlatform,
        pacing: UpgradePacing,
        stop: Arc<tokio::sync::Notify>,
    ) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(Timezone::default());
        let (wifi_event_sender, _) = tokio::sync::broadcast::channel(Self::WIFI_EVENTS_CAPACITY);
        let (wifi_reconfig_sender, _) = tokio::sync::watch::channel(false);
        Self {
            mockfs,
            platform,
            session_manager,
            timezone_sender,
            password,
            hostname,
            mac_address,
            network_config: Arc::new(Mutex::new(NetworkProtocolConfig::Dhcp)),
            ip_address,
            port,
            connected_wifi: Arc::new(tokio::sync::Mutex::new(None)),
            wifi_event_sender,
            wifi_reconfig_sender,
            pacing,
            stop,
        }
    }
}

fn consume_upgrade_marker(marker: &Path) -> UpgradeMarker {
    match std::fs::remove_file(marker) {
        Ok(()) => UpgradeMarker::Consumed,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => UpgradeMarker::Absent,
        Err(err) => {
            warn!("failed to remove mock upgrade marker: {err}");
            UpgradeMarker::RemovalFailed
        }
    }
}

#[async_trait::async_trait]
impl bmc::BmcManager for Manager {
    type Error = Error;
    type SessionManager = MockSessionManager;

    async fn version(&self) -> Option<BosVersion> {
        Some(BosVersion::new(&25, &7))
    }

    fn platform(&self) -> BosPlatform {
        self.platform
    }

    async fn upgrade(
        &self,
        keep_settings: bool,
        _upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        info!(
            "Performing system upgrade (keep_settings={})...",
            keep_settings
        );
        if let Some(progress) = progress {
            let total_bytes = 4_000_000;
            let mut lines = vec![
                ProgressEvent::Phase {
                    phase: "realizing".to_owned(),
                }
                .to_bmc_line(),
                ProgressEvent::RealizationStarted { total_paths: 3 }.to_bmc_line(),
            ];
            // Walk the download across chunks so the client sees real
            // progression toward total_bytes, not a single frozen frame.
            for downloaded_bytes in (1_000_000..=total_bytes).step_by(1_000_000) {
                lines.push(
                    ProgressEvent::Download {
                        downloaded_bytes,
                        total_bytes: Some(total_bytes),
                        remaining_bytes: Some(total_bytes - downloaded_bytes),
                        active: vec![ActiveDownload {
                            store_path: Some("/nix/store/mock-core".to_owned()),
                            source: Some("mock://packages/core".to_owned()),
                            downloaded_bytes,
                            total_bytes: Some(total_bytes),
                        }],
                    }
                    .to_bmc_line(),
                );
            }
            // The firmware-time package run stages next-boot activation, so it
            // stops at building; activation happens after the reboot.
            lines.extend([
                ProgressEvent::RealizationFinished.to_bmc_line(),
                ProgressEvent::Phase {
                    phase: "verifying".to_owned(),
                }
                .to_bmc_line(),
                ProgressEvent::Phase {
                    phase: "building".to_owned(),
                }
                .to_bmc_line(),
            ]);
            for line in lines {
                _ = progress.send(line);
                tokio::time::sleep(self.pacing.progress_step()).await;
            }
        }
        if crate::scenario::read(&self.mockfs.upgrade_scenario()).run
            == crate::scenario::RunScenario::ApplyFail
        {
            return Err(UpgradeError::Failed(
                "mock: firmware apply failed".to_owned(),
            ));
        }

        tokio::time::sleep(self.pacing.sysupgrade_duration()).await;

        crate::scenario::consume_pending_install(
            &self.mockfs.pending_install(),
            &self.mockfs.upgrade_scenario(),
        );

        let reboot_delay = self.pacing.shutdown_delay();
        tokio::spawn(async move {
            tokio::time::sleep(reboot_delay).await;
            info!("Mock sysupgrade: exiting to simulate the reboot");
            std::process::exit(0);
        });

        Ok(())
    }

    async fn consume_upgrade_marker(&self) -> UpgradeMarker {
        consume_upgrade_marker(&self.mockfs.upgrade_result())
    }

    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
        consume_upgrade_marker(&self.mockfs.service_upgrade_marker())
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

    fn watch_wifi_reconfig(&self) -> tokio::sync::watch::Receiver<bool> {
        self.wifi_reconfig_sender.subscribe()
    }

    async fn is_factory_default(&self) -> bool {
        self.mockfs.factory_default().exists()
    }

    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error> {
        info!(hard, "Performing factory reset...");
        Ok(())
    }

    async fn is_setup_pending(&self) -> bool {
        self.mockfs.setup_pending().exists()
    }

    async fn is_wifi_reconfig(&self) -> bool {
        self.mockfs.wifi_reconfig().exists()
    }

    async fn enter_wifi_reconfig(&self) -> Result<(), InitialSetupError> {
        info!("Entering WiFi reconfiguration mode (mock)");
        self.mockfs
            .add_or_remove_flag(true, &self.mockfs.wifi_reconfig())
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        Ok(())
    }

    async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
        info!("Exiting WiFi reconfiguration mode (mock)");
        self.mockfs
            .add_or_remove_flag(false, &self.mockfs.wifi_reconfig())
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
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

        // NOTE: Mock wifi scan by sleeping for some time. On production board scan takes between 5-10s
        tokio::time::sleep(Duration::from_secs(5)).await;
        _ = self.wifi_event_sender.send(WifiEvent::ScanStarted);

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

        _ = self.wifi_event_sender.send(WifiEvent::ScanEnded);

        Ok(wifi_list.split_off(count))
    }

    fn subscribe_wifi_events(&self) -> tokio::sync::broadcast::Receiver<WifiEvent> {
        self.wifi_event_sender.subscribe()
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        info!("Performing reboot...");
        Ok(())
    }

    async fn device_state(&self) -> BmcState {
        if self.is_wifi_reconfig().await {
            BmcState::WifiReconfiguration
        } else if self.is_factory_default().await {
            BmcState::FactoryDefault
        } else if self.is_setup_pending().await {
            BmcState::SetupPending
        } else {
            BmcState::Operational
        }
    }

    async fn update_device_state(&self) -> anyhow::Result<()> {
        match self.device_state().await {
            BmcState::FactoryDefault => {
                self.mockfs
                    .add_or_remove_flag(false, &self.mockfs.factory_default())?;
                self.mockfs
                    .add_or_remove_flag(true, &self.mockfs.setup_pending())?;
            }
            BmcState::SetupPending => {
                self.mockfs
                    .add_or_remove_flag(false, &self.mockfs.setup_pending())?;
            }
            BmcState::WifiReconfiguration => {
                self.mockfs
                    .add_or_remove_flag(false, &self.mockfs.wifi_reconfig())?;
            }
            BmcState::Operational => (),
        }

        Ok(())
    }

    async fn wifi_ssid(&self) -> anyhow::Result<String> {
        Ok(Self::WIFI_SSID.to_owned())
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

    async fn wifi_status(&self) -> anyhow::Result<WifiData> {
        let status = WifiStatus {
            enabled: true,
            configuration: Some(WifiConfiguration {
                mode: WifiMode::Station,
                ssid: "MockWiFi".to_owned(),
                encryption_type: EncryptionType::Wpa2,
            }),
            sta_link_state: Some(WifiLinkState::new("MockWiFi", -45)),
        };

        let iface_data = IfaceData {
            ip: Some(
                "192.168.1.100"
                    .parse()
                    .expect("BUG: hardcoded IP should always parse"),
            ),
            mac: Some(MacAddr::new(0x00, 0x11, 0x22, 0x33, 0x44, 0x55)),
        };

        Ok(WifiData {
            iface: iface_data,
            status,
        })
    }

    async fn wifi_saved_networks(&self) -> anyhow::Result<Vec<WifiStatus>> {
        Ok(vec![
            WifiStatus {
                enabled: true,
                configuration: Some(WifiConfiguration {
                    mode: WifiMode::Station,
                    ssid: "MockWiFi".to_owned(),
                    encryption_type: EncryptionType::Wpa2,
                }),
                sta_link_state: Some(WifiLinkState::new("MockWiFi", -45)),
            },
            WifiStatus {
                enabled: false,
                configuration: Some(WifiConfiguration {
                    mode: WifiMode::Station,
                    ssid: "MockWiFiDisabled".to_owned(),
                    encryption_type: EncryptionType::Wpa1_2,
                }),
                sta_link_state: Some(WifiLinkState::new("MockWiFiDisabled", -5)),
            },
        ])
    }

    async fn handle_graceful_shutdown(&self) {
        // The notifier models the point where bmc-openwrt receives procd's
        // SIGTERM from the external service orchestrator; the mock never
        // signals itself, it just runs the same graceful Axum shutdown path.
        tokio::select! {
            result = signal::ctrl_c() => {
                _ = result;
                info!("Shutdown signal received");
            }
            () = self.stop.notified() => {
                info!("Mock application stop requested");
            }
        }
    }

    async fn support_archive(&self, _format: SupportArchiveFormat) -> Result<Vec<u8>, Error> {
        info!("Support archive");
        let mut buf = [0; 256];
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf[..]));
        zip.start_file(
            Self::DUMMY_SUPPORT_FILE_NAME,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )?;
        zip.write_all(Self::DUMMY_SUPPORT_FILE_CONTENT.as_bytes())?;
        zip.finish()?;
        Ok(buf.to_vec())
    }

    async fn sync_boot_environment(&self, config: &BootloaderConfig) -> Result<(), Self::Error> {
        info!(?config, "Bootloader config sync (no-op in mock)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bmc::UpgradeMarker;

    use super::consume_upgrade_marker;

    #[test]
    fn consuming_mock_upgrade_marker_removes_it_once() {
        let dir = tempfile::tempdir().expect("BUG: create temporary marker directory");
        let marker = dir.path().join("upgrade_result");
        fs::write(&marker, "success").expect("BUG: create mock upgrade marker");

        assert_eq!(consume_upgrade_marker(&marker), UpgradeMarker::Consumed);
        assert!(
            !marker.exists(),
            "consumed marker must not replay after restart"
        );
        assert_eq!(consume_upgrade_marker(&marker), UpgradeMarker::Absent);
    }

    #[test]
    fn consuming_mock_upgrade_marker_reports_removal_failure() {
        let dir = tempfile::tempdir().expect("BUG: create temporary marker directory");

        assert_eq!(
            consume_upgrade_marker(dir.path()),
            UpgradeMarker::RemovalFailed
        );
    }
}

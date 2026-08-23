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

use crate::pwd::{PasswordHashType, SHADOW_FILE_MODE, SHADOW_PATH, ShadowFile};
use crate::session::OpenwrtSessionManager;
use crate::uboot_env::UbootEnvManager;
use crate::unix::system_reboot;
use crate::unix::{
    call_command, call_command_stdin, call_command_to_string, get_hostname, get_ip_address,
};
use crate::{ROOT_USERNAME, pwd, unix};
use anyhow::{anyhow, bail};
use bmc::bootloader_config::BootloaderConfig;
use bmc::manager::{
    BmcState, IfaceData, InitialSetupError, SERVICE_NAME_ENV, UpgradeError, UpgradeMarker,
    WifiData, WifiEvent, WifiNetworkConfig, consume_upgrade_marker, service_upgrade_marker_path,
};
use bmc::{
    BmcManager,
    manager::{NetworkProtocol, NetworkProtocolConfig, NetworkProtocolConfigStatic},
};
use bmc_platform::serial_number::BoardSerial;
use bmc_platform::{BmcInfo, BosPlatform, BosVersion};
use bmc_shared_ii_net::MacAddr;
use bmc_shared_ii_net::wifi::{EncryptionType, WifiMode, WifiScanItem, WifiStatus};
use bmc_shared_ii_net_drv::wifi::OpenwrtWifiManager;
use bmc_shared_ii_net_drv::{NetworkInterface, get_primary_interface};
use bmc_shared_time::time::Timezone;
use bmc_support::SupportArchiveFormat;
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    net::{IpAddr, Ipv4Addr},
    path::Path,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info, trace, warn};

#[derive(Debug)]
#[expect(clippy::struct_field_names)]
pub struct Manager {
    bmc_info: Arc<Option<BmcInfo>>,
    /// Factory-burned OTP serial, `None` when it is unavailable or invalid.
    board_serial: Option<BoardSerial>,
    platform_override: Option<BosPlatform>,
    pub session_manager: OpenwrtSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    wifi_manager: Arc<OpenwrtWifiManager>,
    /// Resolved WiFi network interface name (e.g. "wlan0", "phy0-sta0").
    wifi_iface_name: String,
    wifi_event_sender: tokio::sync::broadcast::Sender<WifiEvent>,
    wifi_reconfig_sender: tokio::sync::watch::Sender<bool>,
    uboot_env_manager: UbootEnvManager,
    upgrade_in_progress: AtomicBool,
}

impl Manager {
    const SYSUPGRADE_BIN: &str = "/sbin/sysupgrade";
    const SYSUPGRADE_ARG_NO_SAVE: &str = "-n";
    const UPGRADE_RESULT_FILE_PATH: &str = "/etc/upgrade_result";
    const DEFAULT_INTERFACE: &str = "wlan0";

    const UCI_SYSTEM_ZONENAME: &str = "system.@system[0].zonename";
    const UCI_SYSTEM_TIMEZONE: &str = "system.@system[0].timezone";
    const UCI_SYSTEM_HOSTNAME: &str = "system.@system[0].hostname";
    const UCI_NET_LAN: &str = "network.wifi_sta";
    const UCI_NET_LAN_PROTO_DHCP_VARIANT: &str = "dhcp";
    const UCI_NET_LAN_PROTO_STATIC_VARIANT: &str = "static";
    const UCI_NET_LAN_PROTO: &str = "network.wifi_sta.proto";
    const UCI_NET_LAN_IPADDR: &str = "network.wifi_sta.ipaddr";
    const UCI_NET_LAN_NETMASK: &str = "network.wifi_sta.netmask";
    const UCI_NET_LAN_GATEWAY: &str = "network.wifi_sta.gateway";
    const UCI_NET_LAN_DNS: &str = "network.wifi_sta.dns";
    const WIFI_EVENTS_CAPACITY: usize = 10;

    #[must_use]
    pub async fn new(
        session_manager: OpenwrtSessionManager,
        timezone: Timezone,
        wifi_manager: Arc<OpenwrtWifiManager>,
        platform_override: Option<BosPlatform>,
        board_serial: Option<BoardSerial>,
    ) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(timezone);
        let (wifi_event_sender, _) = tokio::sync::broadcast::channel(Self::WIFI_EVENTS_CAPACITY);
        let (wifi_reconfig_sender, _) = tokio::sync::watch::channel(false);

        let bmc_info = match BmcInfo::load() {
            Ok(bmc_info) => Some(bmc_info),
            Err(err) => {
                error!(error = ?err, "Failed to load BMC info");
                None
            }
        };

        // Resolve WiFi interface name once from wifi_manager (reads sysfs net/ dir).
        // Falls back to DEFAULT_INTERFACE if the device isn't ready yet.
        let wifi_iface_name = wifi_manager
            .get_wifi_device_name()
            .await
            .unwrap_or_else(|err| {
                error!(?err, "failed to resolve WiFi interface name, using default");
                Self::DEFAULT_INTERFACE.to_owned()
            });

        let manager = Self {
            bmc_info: Arc::new(bmc_info),
            board_serial,
            platform_override,
            session_manager,
            timezone_sender,
            wifi_manager,
            wifi_iface_name,
            wifi_event_sender,
            wifi_reconfig_sender,
            uboot_env_manager: UbootEnvManager::new(),
            upgrade_in_progress: AtomicBool::new(false),
        };

        // Seed the WiFi-reconfig watch from real state so the settings tray
        // reflects setup mode correctly if bmc starts while it is already active.
        // Both FactoryDefault and WifiReconfiguration run the setup AP.
        let setup_ap_active = matches!(
            manager.device_state().await,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        );
        let _ = manager.wifi_reconfig_sender.send(setup_ap_active);

        manager
    }

    #[must_use]
    pub fn board_serial(&self) -> Option<BoardSerial> {
        self.board_serial
    }

    async fn run_sysupgrade(
        keep_settings: bool,
        upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        let mut sysupgrade = Command::new(Self::SYSUPGRADE_BIN);
        if !keep_settings {
            sysupgrade.arg(Self::SYSUPGRADE_ARG_NO_SAVE);
        }
        sysupgrade.arg(upgrade_image_path.as_os_str());
        sysupgrade.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut handle = sysupgrade
            .spawn()
            .map_err(|e| UpgradeError::Failed(format!("failed to spawn sysupgrade: {e}")))?;
        let stdout = handle
            .stdout
            .take()
            .expect("BUG: stdout was piped but is missing");
        let stderr = handle
            .stderr
            .take()
            .expect("BUG: stderr was piped but is missing");

        // Drain both pipes concurrently with wait(): if either drain lagged
        // behind, the 64 KB pipe buffer could fill and block sysupgrade.
        let stdout_drain = drain_lines(stdout, |line| trace!(line, "sysupgrade stdout"));
        let stderr_drain = drain_lines(stderr, |line| {
            debug!(line, "sysupgrade stderr");
            if let Some(progress) = &progress {
                _ = progress.send(line.to_owned());
            }
        });
        let (status, (), ()) = tokio::join!(handle.wait(), stdout_drain, stderr_drain);

        let status = status
            .inspect_err(|err| error!(error = %err, "Upgrade process wait failed"))
            .map_err(|e| UpgradeError::Failed(format!("upgrade process wait failed: {e}")))?;

        interpret_sysupgrade_exit(status.code())
    }

    async fn restart_system_service(&self) -> anyhow::Result<()> {
        call_command("uci", &["commit", "system"]).await?;
        call_command("/etc/init.d/system", &["restart"]).await?;
        Ok(())
    }

    async fn get_network_protocol(&self) -> Option<NetworkProtocol> {
        let output = Command::new("uci")
            .arg("get")
            .arg(Self::UCI_NET_LAN_PROTO)
            .output()
            .await
            .ok()?;
        if output.status.success() {
            match String::from_utf8(output.stdout).as_deref().map(str::trim) {
                Ok(Self::UCI_NET_LAN_PROTO_DHCP_VARIANT) => Some(NetworkProtocol::Dhcp),
                Ok(Self::UCI_NET_LAN_PROTO_STATIC_VARIANT) => Some(NetworkProtocol::Static),
                _ => None,
            }
        } else {
            None
        }
    }

    fn get_mac_address(&self) -> Option<String> {
        NetworkInterface::get_by_name(&self.wifi_iface_name)
            .and_then(|network| network.mac_address().map(|mac| mac.to_string()))
    }

    fn get_network_ip_by_name(name: &str) -> Option<IpAddr> {
        NetworkInterface::get_by_name(name).and_then(|network| network.ipv4_address())
    }

    fn get_mac_short_id(mac: &str) -> String {
        let mac = mac.replace(MacAddr::DELIMITER, "");
        let mac = mac.as_bytes();

        mac.len()
            .checked_sub(3)
            .map_or("UNK".to_owned(), |start_idx| {
                String::from_utf8_lossy(&mac[start_idx..]).to_string()
            })
    }

    async fn configure_wifi_ap(&self) -> Result<(), InitialSetupError> {
        let ssid = self
            .calculate_wifi_ssid()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        info!(ssid = %ssid, "Configuring WiFi AP for initial setup");
        let wifi_manager = self.wifi_manager.as_ref();

        wifi_manager
            .reset_config()
            .await
            .inspect_err(|err| error!(error = %err, "Failed to reset WiFi config"))
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        wifi_manager
            .configure_ap_mode(ssid, None, EncryptionType::None)
            .await
            .inspect_err(|err| error!(error = %err, "Failed to configure WiFi AP mode"))
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        wifi_manager
            .enable_radio(true)
            .await
            .inspect_err(|err| error!(error = %err, "Failed to enable WiFi radio"))
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;

        info!("WiFi AP configured successfully");
        Ok(())
    }

    async fn disable_captive_portal(&self) -> Result<(), InitialSetupError> {
        debug!("Disabling captive portal configuration");

        call_command(
            "sh",
            &[
                "-c",
                ". /lib/functions/bos-factory-default.sh && disable_captive_portal && /etc/init.d/dnsmasq restart",
            ],
        )
        .await
        .inspect_err(|err| error!(error = %err, "Failed to disable captive portal"))
        .map_err(|err| InitialSetupError::UnexpectedFailure(format!("Failed to disable captive portal: {err}")))?;

        info!("Captive portal disabled successfully");
        Ok(())
    }

    async fn enable_captive_portal(&self) -> Result<(), InitialSetupError> {
        debug!("Enabling captive portal configuration");

        call_command(
            "sh",
            &[
                "-c",
                ". /lib/functions/bos-factory-default.sh && enable_captive_portal $FACTORY_DEFAULT_AP_IP_ADDR && /etc/init.d/dnsmasq restart",
            ],
        )
        .await
        .inspect_err(|err| error!(error = %err, "Failed to enable captive portal"))
        .map_err(|err| InitialSetupError::UnexpectedFailure(format!("Failed to enable captive portal: {err}")))?;

        info!("Captive portal enabled successfully");
        Ok(())
    }

    async fn set_wifi_reconfig_flag(&self) -> Result<(), InitialSetupError> {
        call_command(
            "sh",
            &[
                "-c",
                ". /lib/functions/bos-defaults.sh && set_wifi_reconfig",
            ],
        )
        .await
        .inspect_err(|err| error!(error = %err, "Failed to set wifi reconfig flag"))
        .map_err(|err| {
            InitialSetupError::UnexpectedFailure(format!("Failed to set wifi reconfig flag: {err}"))
        })?;

        Ok(())
    }

    async fn unset_wifi_reconfig_flag(&self) -> Result<(), InitialSetupError> {
        call_command(
            "sh",
            &[
                "-c",
                ". /lib/functions/bos-defaults.sh && unset_wifi_reconfig",
            ],
        )
        .await
        .inspect_err(|err| error!(error = %err, "Failed to unset wifi reconfig flag"))
        .map_err(|err| {
            InitialSetupError::UnexpectedFailure(format!(
                "Failed to unset wifi reconfig flag: {err}"
            ))
        })?;

        Ok(())
    }

    fn make_wifi_ssid_for_mac(&self, mac_short_id: &str) -> String {
        format!(
            "{} {mac_short_id}",
            self.platform().product().display_name()
        )
    }

    async fn calculate_wifi_ssid(&self) -> anyhow::Result<String> {
        let mac = call_command_to_string(
            "sh",
            &["-c", ". /lib/functions/bos-defaults.sh && wifi_mac"],
        )
        .await
        .inspect_err(|err| error!(error = %err, "Failed to read Wi-Fi MAC address"))?;

        let mac_id = Self::get_mac_short_id(mac.trim());
        Ok(self.make_wifi_ssid_for_mac(&mac_id))
    }
}

#[async_trait::async_trait]
impl BmcManager for Manager {
    type SessionManager = OpenwrtSessionManager;
    type Error = Error;

    async fn version(&self) -> Option<BosVersion> {
        self.bmc_info
            .as_ref()
            .as_ref()
            .map(|bmc_info| bmc_info.bos_version.clone())
    }

    fn platform(&self) -> BosPlatform {
        if let Some(platform) = self.platform_override {
            return platform;
        }
        self.bmc_info
            .as_ref()
            .as_ref()
            .map(|info| info.bmc_platform)
            .expect(
                "BUG: bmc-openwrt requires /etc/bos_platform; \
                 use --hardware-profile to override during development",
            )
    }

    async fn upgrade(
        &self,
        keep_settings: bool,
        upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        info!(
            keep_settings = keep_settings,
            path = %upgrade_image_path.display(),
            "Starting system upgrade"
        );

        self.upgrade_in_progress.store(true, Ordering::SeqCst);
        let result = Self::run_sysupgrade(keep_settings, upgrade_image_path, progress).await;
        if result.is_err() {
            self.upgrade_in_progress.store(false, Ordering::SeqCst);
        }
        result
    }

    async fn consume_upgrade_marker(&self) -> UpgradeMarker {
        consume_upgrade_marker(Path::new(Self::UPGRADE_RESULT_FILE_PATH)).await
    }

    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
        let Some(marker) = service_upgrade_marker_path() else {
            warn!("{SERVICE_NAME_ENV} is unset, so an in-place upgrade cannot be reported");
            return UpgradeMarker::Absent;
        };
        consume_upgrade_marker(&marker).await
    }

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error> {
        let shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        let matches = shadow_file.check_credentials(ROOT_USERNAME, password);

        Ok(matches)
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        let mut shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        shadow_file.set_password(ROOT_USERNAME, password, PasswordHashType::Md5)?;

        bmc::utils::replace_file_with_mode(
            SHADOW_PATH,
            shadow_file.to_string().as_bytes(),
            Some(SHADOW_FILE_MODE),
        )
        .await?;

        info!(username = ROOT_USERNAME, "System password updated");

        Ok(())
    }

    fn timezone(&self) -> Timezone {
        self.timezone_sender.borrow().clone()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()> {
        let zonename_cmd = format!("{}={}", Self::UCI_SYSTEM_ZONENAME, timezone.iana());
        call_command("uci", &["set", &zonename_cmd]).await?;

        let timezone_cmd: String = format!("{}={}", Self::UCI_SYSTEM_TIMEZONE, timezone.posix());
        call_command("uci", &["set", &timezone_cmd]).await?;

        self.restart_system_service().await?;

        let timezone_for_log = timezone.clone();
        self.timezone_sender.send_if_modified(|current| {
            if *current != timezone {
                *current = timezone;
                return true;
            }
            false
        });

        info!(timezone = %timezone_for_log, "System timezone updated");

        Ok(())
    }

    fn watch_timezone_updates(&self) -> tokio::sync::watch::Receiver<Timezone> {
        self.timezone_sender.subscribe()
    }

    fn watch_wifi_reconfig(&self) -> tokio::sync::watch::Receiver<bool> {
        self.wifi_reconfig_sender.subscribe()
    }

    async fn is_factory_default(&self) -> bool {
        call_command(
            "sh",
            &[
                "-c",
                ". /lib/functions/bos-defaults.sh && is_factory_default",
            ],
        )
        .await
        .is_ok()
    }

    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error> {
        let mut args = vec!["factory_reset"];
        if hard {
            args.push("--hard");
        }
        call_command("bos", &args).await?;
        Ok(())
    }

    async fn is_setup_pending(&self) -> bool {
        call_command(
            "sh",
            &["-c", ". /lib/functions/bos-defaults.sh && is_setup_pending"],
        )
        .await
        .is_ok()
    }

    async fn is_wifi_reconfig(&self) -> bool {
        call_command(
            "sh",
            &["-c", ". /lib/functions/bos-defaults.sh && is_wifi_reconfig"],
        )
        .await
        .is_ok()
    }

    async fn enter_wifi_reconfig(&self) -> Result<(), InitialSetupError> {
        info!("Entering WiFi reconfiguration mode");

        self.set_wifi_reconfig_flag().await?;
        self.configure_wifi_ap().await?;
        self.enable_captive_portal().await?;

        let _ = self.wifi_reconfig_sender.send(true);

        info!("WiFi reconfiguration mode enabled");
        Ok(())
    }

    async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
        if !self.is_wifi_reconfig().await {
            debug!("Not in WiFi reconfiguration mode, nothing to exit");
            return Ok(());
        }

        info!("Exiting WiFi reconfiguration mode");

        self.disable_captive_portal().await?;
        self.unset_wifi_reconfig_flag().await?;

        let _ = self.wifi_reconfig_sender.send(false);

        info!("WiFi reconfiguration mode disabled");
        Ok(())
    }

    async fn hostname(&self) -> Option<String> {
        match uci_get_opt(Self::UCI_SYSTEM_HOSTNAME).await {
            None => get_hostname().await,
            hostname => hostname,
        }
    }

    fn mac_address(&self) -> Option<String> {
        self.get_mac_address()
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        if let Some(ip) = Self::get_network_ip_by_name(&self.wifi_iface_name) {
            return Some(ip);
        }
        get_ip_address()
    }

    async fn network_config(&self) -> Option<NetworkProtocolConfig> {
        let protocol = match self.get_network_protocol().await? {
            NetworkProtocol::Dhcp => NetworkProtocolConfig::Dhcp,
            NetworkProtocol::Static => NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
                address: uci_get_opt(Self::UCI_NET_LAN_IPADDR).await?.parse().ok()?,
                netmask: uci_get_opt(Self::UCI_NET_LAN_NETMASK).await?.parse().ok()?,
                gateway: uci_get_opt(Self::UCI_NET_LAN_GATEWAY).await?.parse().ok()?,
                dns_servers: uci_get_opt(Self::UCI_NET_LAN_DNS)
                    .await
                    .unwrap_or_else(String::new)
                    .split_whitespace()
                    .map(str::parse)
                    .map(Result::ok)
                    .collect::<Option<Vec<_>>>()?,
            }),
        };

        Some(protocol)
    }

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> anyhow::Result<()> {
        let mut stdin = vec![];

        match config {
            NetworkProtocolConfig::Dhcp => {
                stdin.extend_from_slice(&[
                    format!(
                        "set {}='{}'",
                        Self::UCI_NET_LAN_PROTO,
                        Self::UCI_NET_LAN_PROTO_DHCP_VARIANT
                    ),
                    format!("delete {}", Self::UCI_NET_LAN_IPADDR),
                    format!("delete {}", Self::UCI_NET_LAN_NETMASK),
                    format!("delete {}", Self::UCI_NET_LAN_GATEWAY),
                    format!("delete {}", Self::UCI_NET_LAN_DNS),
                ]);
            }
            NetworkProtocolConfig::Static(config) => {
                stdin.extend_from_slice(&[
                    format!(
                        "set {}='{}'",
                        Self::UCI_NET_LAN_PROTO,
                        Self::UCI_NET_LAN_PROTO_STATIC_VARIANT
                    ),
                    format!("set {}='{}'", Self::UCI_NET_LAN_IPADDR, config.address),
                    format!("set {}='{}'", Self::UCI_NET_LAN_NETMASK, config.netmask),
                    format!("set {}='{}'", Self::UCI_NET_LAN_GATEWAY, config.gateway),
                    format!(
                        "set {}='{}'",
                        Self::UCI_NET_LAN_DNS,
                        config
                            .dns_servers
                            .iter()
                            .map(Ipv4Addr::to_string)
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                ]);
            }
        }

        stdin.push(format!("commit {}", Self::UCI_NET_LAN));

        let output = call_command_stdin("uci", &["-q", "batch"], &stdin.join("\n")).await?;
        if !output.status.success() || !output.stderr.is_empty() {
            let msg = String::from_utf8_lossy(&output.stderr).to_string();
            bail!(msg);
        }

        call_command("/etc/init.d/network", &["restart"]).await?;

        Ok(())
    }

    async fn captive_portal_redirect_host(&self) -> Option<String> {
        self.ip_address().await.map(|ip| ip.to_string())
    }

    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError> {
        let has_password = config.password.is_some();
        info!(
            ssid = %config.ssid,
            has_password = has_password,
            encryption = ?config.encryption,
            "Connecting to WiFi for initial setup"
        );

        self.wifi_save_and_connect(config.ssid.clone(), config.password, config.encryption)
            .await
            .inspect_err(|err| {
                error!(
                    error = %err,
                    ssid = %config.ssid,
                    "Failed to save and connect to WiFi"
                );
            })
            .map_err(|err| InitialSetupError::WifiConnectionFailure(err.to_string()))?;

        info!(ssid = %config.ssid, "WiFi connection established successfully");

        self.disable_captive_portal().await?;

        self.update_device_state()
            .await
            .inspect_err(|err| error!(error = %err, "Failed to update device state"))
            .map_err(|err| InitialSetupError::UnexpectedFailure(err.to_string()))?;

        Ok(())
    }

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError> {
        if !self.is_factory_default().await {
            return Err(InitialSetupError::NotSupported);
        }

        self.configure_wifi_ap().await
    }

    async fn wifi_scan(&self) -> anyhow::Result<Vec<WifiScanItem>> {
        // NOTE: Future can be cancelled before returning result, but it is necessary to signal that scan has ended
        struct DropGuard {
            wifi_event_sender: tokio::sync::broadcast::Sender<WifiEvent>,
        }

        impl Drop for DropGuard {
            fn drop(&mut self) {
                if let Err(err) = self.wifi_event_sender.send(WifiEvent::ScanEnded) {
                    debug!(error = %err, "Failed to send WiFi scan ended event");
                }
            }
        }

        let _guard = DropGuard {
            wifi_event_sender: self.wifi_event_sender.clone(),
        };

        if let Err(err) = self.wifi_event_sender.send(WifiEvent::ScanStarted) {
            debug!(error = %err, "Failed to send WiFi scan started event");
        }

        self.wifi_manager.scan().await
    }

    fn subscribe_wifi_events(&self) -> tokio::sync::broadcast::Receiver<WifiEvent> {
        self.wifi_event_sender.subscribe()
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        system_reboot().await.map_err(|e| anyhow!(e))
    }

    async fn device_state(&self) -> BmcState {
        // check wifi reconfiguration flag first (highest priority for operational devices)
        if self.is_wifi_reconfig().await {
            BmcState::WifiReconfiguration
        }
        // check factory default flag
        else if self.is_factory_default().await {
            BmcState::FactoryDefault
        }
        // check flag if setup is pending
        else if self.is_setup_pending().await {
            BmcState::SetupPending
        } else {
            BmcState::Operational
        }
    }

    async fn update_device_state(&self) -> anyhow::Result<()> {
        match self.device_state().await {
            BmcState::FactoryDefault => {
                // Remove factory default flag
                let result = call_command(
                    "sh",
                    &[
                        "-c",
                        ". /lib/functions/bos-defaults.sh && unset_factory_default",
                    ],
                )
                .await
                .map_err(|e| anyhow!("Failed to remove factory default flag, error: {e}"));
                if result.is_ok() {
                    let _ = self.wifi_reconfig_sender.send(false);
                }
                result
            }
            BmcState::SetupPending => {
                // Remove setup pending flag
                call_command(
                    "sh",
                    &[
                        "-c",
                        ". /lib/functions/bos-defaults.sh && unset_setup_pending",
                    ],
                )
                .await
                .map_err(|e| anyhow!("Failed to remove setup pending flag, error: {e}"))
            }
            BmcState::WifiReconfiguration => {
                // Remove wifi reconfig flag (return to Operational)
                // NOTE: Intentionally duplicates unset_wifi_reconfig_flag() to match
                // the pattern used by other states (FactoryDefault, SetupPending) and
                // to avoid error type conversion from InitialSetupError to anyhow::Error
                let result = call_command(
                    "sh",
                    &[
                        "-c",
                        ". /lib/functions/bos-defaults.sh && unset_wifi_reconfig",
                    ],
                )
                .await
                .map_err(|e| anyhow!("Failed to remove wifi reconfig flag, error: {e}"));
                if result.is_ok() {
                    let _ = self.wifi_reconfig_sender.send(false);
                }
                result
            }
            BmcState::Operational => Ok(()),
        }
    }

    async fn wifi_ssid(&self) -> anyhow::Result<String> {
        self.wifi_manager
            .get_ap_ssid()
            .await
            .ok_or(anyhow!("Wi-Fi interface not in AP mode."))
    }

    async fn init_wifi_ap(&self) -> Result<(), Self::Error> {
        let state = self.device_state().await;
        if matches!(
            state,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        ) {
            self.configure_wifi_ap()
                .await
                .map_err(|e| self::Error::InitialSetupWifiAp(e.to_string()))?;
        }
        // Enable captive portal for wifi reconfig (factory default has it pre-configured)
        if state == BmcState::WifiReconfiguration {
            self.enable_captive_portal()
                .await
                .map_err(|e| self::Error::InitialSetupWifiAp(e.to_string()))?;
        }
        Ok(())
    }

    async fn wifi_save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<(), Self::Error> {
        self.wifi_manager
            .save_and_connect(ssid, password, encryption)
            .await
            .map_err(|e| Error::WifiError(e.to_string()))
    }

    async fn wifi_status(&self) -> anyhow::Result<WifiData> {
        let iface = get_primary_interface()
            .ok_or(Error::WifiError("No Wi-Fi interface found".to_owned()))?;
        let status = self.wifi_manager.status().await?;

        Ok(WifiData {
            iface: IfaceData {
                ip: iface.ipv4_address(),
                mac: iface.mac_address(),
            },
            status,
        })
    }

    async fn wifi_saved_networks(&self) -> anyhow::Result<Vec<WifiStatus>> {
        Ok(self
            .wifi_manager
            .status_all()
            .await?
            .into_iter()
            .filter(|status| {
                status
                    .clone()
                    .configuration
                    .is_some_and(|conf| conf.mode == WifiMode::Station)
            })
            .collect::<Vec<WifiStatus>>())
    }

    async fn handle_graceful_shutdown(&self) {
        unix::handle_graceful_shutdown(&self.upgrade_in_progress).await;
    }

    async fn support_archive(&self, format: SupportArchiveFormat) -> Result<Vec<u8>, Error> {
        unix::get_support_archive(format).await.map_err(Into::into)
    }

    async fn sync_boot_environment(&self, config: &BootloaderConfig) -> Result<(), Self::Error> {
        self.uboot_env_manager
            .sync(config)
            .await
            .map_err(|e| Error::UbootEnv(e.to_string()))
    }
}

/// `-UBUS_STATUS_CONNECTION_FAILED` (-10) as a shell exit code. sysupgrade's
/// last command is `ubus call system sysupgrade`; procd accepts the upgrade
/// without answering the call, so the connection drops and sysupgrade exits
/// 246 on the success path.
const SYSUPGRADE_EXIT_UBUS_CONNECTION_FAILED: i32 = 246;

/// Map sysupgrade's exit status to the upgrade outcome.
fn interpret_sysupgrade_exit(exit_code: Option<i32>) -> Result<(), UpgradeError> {
    match exit_code {
        Some(0) => {
            info!("System upgrade completed successfully");
            Ok(())
        }
        Some(SYSUPGRADE_EXIT_UBUS_CONNECTION_FAILED) => {
            info!(
                exit_code = SYSUPGRADE_EXIT_UBUS_CONNECTION_FAILED,
                "System upgrade accepted; reboot follows"
            );
            Ok(())
        }
        // Error code "1" is returned on BCB when using incompatible image, unsigned image or wrong signature keys
        Some(1) => {
            error!(exit_code = 1, "Upgrade failed: invalid firmware image");
            Err(UpgradeError::InvalidImage)
        }
        // procd answers genuine rejections; they surface as other ubus
        // status exit codes (2, 8, 9, ...).
        Some(code) => {
            error!(exit_code = code, "Upgrade failed");
            Err(UpgradeError::Failed(format!(
                "upgrade failed with exit code {code}"
            )))
        }
        None => {
            error!("Upgrade process terminated without exit code");
            Err(UpgradeError::Failed(
                "upgrade process terminated without exit code".to_owned(),
            ))
        }
    }
}

/// Drain `reader` line-by-line, feeding each line to `on_line`.
///
/// Reads raw bytes and converts lossily: sysupgrade output is not
/// guaranteed to be UTF-8, and the drain must survive every line until
/// EOF — an early stop lets the pipe buffer fill and blocks sysupgrade
/// mid-flash. An I/O error on the pipe ends the drain; `wait()` still
/// decides the run's outcome.
async fn drain_lines(reader: impl tokio::io::AsyncRead + Unpin, mut on_line: impl FnMut(&str)) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                on_line(&String::from_utf8_lossy(&buf));
            }
            Err(err) => {
                debug!(error = %err, "draining sysupgrade output failed");
                break;
            }
        }
    }
}

async fn uci_get_opt(opt: &str) -> Option<String> {
    call_command_to_string("uci", &["get", opt])
        .await
        .ok()
        .map(|value| value.trim().to_owned())
}

#[must_use]
pub fn get_default_net_data(default_interface: &str) -> IfaceData {
    debug!("Getting net data... {default_interface}");

    let (ip, mac) = NetworkInterface::get_by_name(default_interface)
        .ok_or(bmc_shared_ii_net_drv::NetworkInterface::find_default)
        .map_or((None, None), |net_iface_default| {
            (
                net_iface_default.ipv4_address(),
                net_iface_default.mac_address(),
            )
        });

    debug!("IP: {ip:?}, MAC: {mac:?}");

    IfaceData { ip, mac }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ShadowFile(#[from] pwd::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Unix(#[from] unix::Error),
    #[error("Wi-Fi is not present")]
    WifiNotPresent,
    #[error("Cannot configure WiFi AP for initial setup: {0}")]
    InitialSetupWifiAp(String),
    #[error("Wrong password or other error: {0}")]
    WifiError(String),
    #[error("U-Boot environment error: {0}")]
    UbootEnv(String),
}

#[cfg(test)]
mod tests {
    use super::{UpgradeError, drain_lines, interpret_sysupgrade_exit};

    #[tokio::test]
    async fn drain_lines_survives_non_utf8_output() {
        let mut lines: Vec<String> = Vec::new();
        drain_lines(&b"ok\nbad \xff byte\nafter\n"[..], |line| {
            lines.push(line.to_owned());
        })
        .await;
        assert_eq!(
            lines,
            vec![
                "ok".to_owned(),
                "bad \u{FFFD} byte".to_owned(),
                "after".to_owned(),
            ],
            "a non-UTF-8 byte must not stop the drain"
        );
    }

    #[test]
    fn sysupgrade_exit_zero_is_success() {
        assert!(interpret_sysupgrade_exit(Some(0)).is_ok());
    }

    #[test]
    fn sysupgrade_exit_246_is_the_accepted_handoff_not_a_failure() {
        // procd accepts the upgrade without answering the ubus call, so a
        // successful flash exits with -UBUS_STATUS_CONNECTION_FAILED (246).
        // Reporting it as a failure showed "Upgrade failed" on every
        // successful upgrade.
        assert!(
            interpret_sysupgrade_exit(Some(246)).is_ok(),
            "exit 246 is the success path: the ubus call is never answered"
        );
    }

    #[test]
    fn sysupgrade_exit_one_is_an_invalid_image() {
        assert!(
            matches!(
                interpret_sysupgrade_exit(Some(1)),
                Err(UpgradeError::InvalidImage)
            ),
            "exit 1 is the image-check rejection and must stay a distinct error"
        );
    }

    #[test]
    fn sysupgrade_ubus_rejection_codes_are_failures() {
        // procd answers genuine rejections, so ubus exits with the real
        // status code (2, 8, 9, ...) — those must not ride the 246
        // acceptance path.
        for code in [2, 8, 9] {
            assert!(
                matches!(
                    interpret_sysupgrade_exit(Some(code)),
                    Err(UpgradeError::Failed(_))
                ),
                "exit {code} is a procd rejection, not an accepted handoff"
            );
        }
    }

    #[test]
    fn sysupgrade_death_by_signal_is_a_failure() {
        assert!(matches!(
            interpret_sysupgrade_exit(None),
            Err(UpgradeError::Failed(_))
        ));
    }
}

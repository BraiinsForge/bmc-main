// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::pwd::{PasswordHashType, SHADOW_PATH, ShadowFile};
use crate::session::OpenwrtSessionManager;
use crate::unix::system_reboot;
use crate::unix::{
    call_command, call_command_stdin, call_command_to_string, get_hostname, get_ip_address,
};
use crate::{ROOT_USERNAME, pwd, unix};
use anyhow::{anyhow, bail};
use bmc::manager::{BmcState, IfaceData, InitialSetupError, WifiData, WifiNetworkConfig};
use bmc::{
    BmcManager,
    manager::{NetworkProtocol, NetworkProtocolConfig, NetworkProtocolConfigStatic},
};
use bmc_platform::{BmcInfo, BmcPlatform, BosVersion};
use bmc_shared_ii_net::MacAddr;
use bmc_shared_ii_net::wifi::{EncryptionType, WifiScanItem, WifiStatus};
use bmc_shared_ii_net_drv::wifi::OpenwrtWifiManager;
use bmc_shared_ii_net_drv::{NetworkInterface, get_primary_interface};
use bmc_shared_time::time::Timezone;
use std::io;
use std::sync::Arc;
use std::{
    net::{IpAddr, Ipv4Addr},
    path::Path,
};
use tokio::{fs, process::Command};
use tracing::log::debug;
use tracing::{error, info};

#[derive(Debug)]
pub struct Manager {
    bmc_info: Arc<BmcInfo>,
    pub session_manager: OpenwrtSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    wifi_manager: Arc<OpenwrtWifiManager>,
    wifi_ap_ssid_base: String,
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

    #[must_use]
    pub fn new(
        session_manager: OpenwrtSessionManager,
        timezone: Timezone,
        wifi_manager: Arc<OpenwrtWifiManager>,
        wifi_ap_ssid_base: String,
    ) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(timezone);
        let bmc_info = BmcInfo::load().expect("Load BMC info failed");
        Self {
            bmc_info: Arc::new(bmc_info),
            session_manager,
            timezone_sender,
            wifi_manager,
            wifi_ap_ssid_base,
        }
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

    fn get_mac_address() -> Option<String> {
        NetworkInterface::get_by_name(Self::DEFAULT_INTERFACE)
            .and_then(|network| network.mac_address().map(|mac| mac.to_string()))
    }

    fn get_network_ip_by_name(name: &str) -> Option<IpAddr> {
        NetworkInterface::get_by_name(name).and_then(|network| network.ipv4_address())
    }

    fn get_mac_short_id(eth_data: &IfaceData) -> String {
        let mac = eth_data
            .mac
            .clone()
            .unwrap_or_else(|| {
                error!("Cannot obtain MAC address");
                MacAddr::default()
            })
            .to_string()
            .replace(MacAddr::DELIMITER, "");
        let mac = mac.as_bytes();

        mac.len()
            .checked_sub(3)
            .map_or("UNK".to_owned(), |start_idx| {
                String::from_utf8_lossy(&mac[start_idx..]).to_string()
            })
    }

    async fn configure_wifi_ap(&self) -> Result<(), InitialSetupError> {
        info!("Setting up WiFi AP for initial setup");
        let wifi_manager = self.wifi_manager.as_ref();
        debug!("Resetting wifi to default");

        wifi_manager
            .reset_config()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        wifi_manager
            .configure_ap_mode(self.wifi_ssid(), None, EncryptionType::None)
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        wifi_manager
            .enable_radio(true)
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
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
        .map_err(|e| InitialSetupError::UnexpectedFailure(format!("Failed to remove captive portal configuration: {e}")))?;

        debug!("Captive portal configuration disabled");
        Ok(())
    }
}

#[async_trait::async_trait]
impl BmcManager for Manager {
    type SessionManager = OpenwrtSessionManager;
    type Error = Error;

    async fn version(&self) -> BosVersion {
        self.bmc_info.bos_version.clone()
    }

    fn platform(&self) -> BmcPlatform {
        BmcPlatform::BraiinsBmc
    }

    async fn upgrade(&self, keep_settings: bool, upgrade_image_path: &Path) -> anyhow::Result<()> {
        let mut sysupgrade = Command::new(Self::SYSUPGRADE_BIN);
        if !keep_settings {
            sysupgrade.arg(Self::SYSUPGRADE_ARG_NO_SAVE);
        }
        sysupgrade.arg(upgrade_image_path.as_os_str());

        let mut handle = sysupgrade.spawn()?;

        let status = handle
            .wait()
            .await
            .map_err(|_| anyhow!("Invalid firmware image"))?;

        if let Some(code) = status.code() {
            match code {
                // Error code "1" is returned on BCB when using incompatible image, unsigned image or wrong signature keys
                1 => Err(anyhow!("Invalid firmware image")),
                _ => Ok(()),
            }
        } else {
            Err(anyhow!("Upgrade failed"))
        }
    }

    async fn check_and_remove_upgrade_marker(&self) -> bool {
        let is_after_upgrade = Path::new(Self::UPGRADE_RESULT_FILE_PATH).exists();

        if is_after_upgrade {
            _ = fs::remove_file(Self::UPGRADE_RESULT_FILE_PATH).await;
        }

        is_after_upgrade
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
        info!("Changing `{ROOT_USERNAME}` password");

        let mut shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        shadow_file.set_password(ROOT_USERNAME, password, PasswordHashType::Md5)?;

        let temp_shadow_file_path = format!("{SHADOW_PATH}.tmp");

        fs::write(&temp_shadow_file_path, shadow_file.to_string()).await?;
        fs::rename(&temp_shadow_file_path, SHADOW_PATH).await?;

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

    async fn hostname(&self) -> Option<String> {
        match uci_get_opt(Self::UCI_SYSTEM_HOSTNAME).await {
            None => get_hostname().await,
            hostname => hostname,
        }
    }

    fn mac_address(&self) -> Option<String> {
        Self::get_mac_address()
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        if let Some(ip) = Self::get_network_ip_by_name(Self::DEFAULT_INTERFACE) {
            return Some(ip);
        }

        let wifi_ip_addr = self
            .wifi_manager
            .get_wifi_device_name()
            .await
            .ok()
            .and_then(|wifi_dev_name| Self::get_network_ip_by_name(wifi_dev_name.as_ref()));

        if wifi_ip_addr.is_some() {
            return wifi_ip_addr;
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
        debug!(
            "Connecting to wifi '{}', with password: {:?}",
            config.ssid, config.password
        );
        self.wifi_save_and_connect(config.ssid, config.password, config.encryption)
            .await
            .map_err(|e| InitialSetupError::WifiConnectionFailure(e.to_string()))?;
        debug!("Connection to wifi finished successfully");

        self.disable_captive_portal().await?;

        self.update_device_state()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
    }

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError> {
        if !self.is_factory_default().await {
            return Err(InitialSetupError::NotSupported);
        }

        self.configure_wifi_ap().await
    }

    async fn wifi_scan(&self) -> anyhow::Result<Vec<WifiScanItem>> {
        self.wifi_manager.scan().await
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        system_reboot().await.map_err(|e| anyhow!(e))
    }

    async fn device_state(&self) -> BmcState {
        // check factory default flag
        if self.is_factory_default().await {
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
                call_command(
                    "sh",
                    &[
                        "-c",
                        ". /lib/functions/bos-defaults.sh && unset_factory_default",
                    ],
                )
                .await
                .map_err(|e| anyhow!("Failed to remove factory default flag, error: {}", e))
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
                .map_err(|e| anyhow!("Failed to remove setup pending flag, error: {}", e))
            }
            BmcState::Operational => Ok(()),
        }
    }

    fn wifi_ssid(&self) -> String {
        let mac_id = Self::get_mac_short_id(&get_default_net_data(Self::DEFAULT_INTERFACE));
        format!("{} {mac_id}", self.wifi_ap_ssid_base)
    }

    async fn init_wifi_ap(&self) -> Result<(), Self::Error> {
        if self.is_factory_default().await {
            self.configure_wifi_ap()
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
        self.wifi_manager.status_all().await
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
}

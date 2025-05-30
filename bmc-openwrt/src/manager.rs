// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::pwd::{PasswordHashType, SHADOW_PATH, ShadowFile};
use crate::session::OpenwrtSessionManager;
use crate::{ROOT_USERNAME, pwd, unix};
use anyhow::{anyhow, bail};
use bmc::{
    BmcManager,
    manager::{NetworkProtocol, NetworkProtocolConfig, NetworkProtocolConfigStatic},
};
use bmc_platform::BmcPlatform;
use bmc_shared::time::Timezone;
use std::io;
use std::{
    net::{IpAddr, Ipv4Addr},
    path::Path,
};
use tokio::{fs, process::Command};
use tracing::info;

use crate::{
    network::NetworkInterface,
    unix::{
        call_command, call_command_stdin, call_command_to_string, get_hostname, get_ip_address,
    },
};

#[derive(Debug)]
pub struct Manager {
    pub session_manager: OpenwrtSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
}

impl Manager {
    const SYSUPGRADE_BIN: &'static str = "/sbin/sysupgrade";
    const SYSUPGRADE_ARG_NO_SAVE: &'static str = "-n";
    const UPGRADE_RESULT_FILE_PATH: &str = "/etc/upgrade_result";
    const DEFAULT_INTERFACE: &str = "eth0";
    const DEFAULT_AP_INTERFACE_NAME: &'static str = "ethap0";
    const UCI_SYSTEM_ZONENAME: &str = "system.@system[0].zonename";
    const UCI_SYSTEM_TIMEZONE: &str = "system.@system[0].timezone";
    const UCI_SYSTEM_HOSTNAME: &str = "system.@system[0].hostname";
    const UCI_NET_LAN: &str = "network.lan";
    const UCI_NET_LAN_PROTO_DHCP_VARIANT: &str = "dhcp";
    const UCI_NET_LAN_PROTO_STATIC_VARIANT: &str = "static";
    const UCI_NET_LAN_PROTO: &str = "network.lan.proto";
    const UCI_NET_LAN_IPADDR: &str = "network.lan.ipaddr";
    const UCI_NET_LAN_NETMASK: &str = "network.lan.netmask";
    const UCI_NET_LAN_GATEWAY: &str = "network.lan.gateway";
    const UCI_NET_LAN_DNS: &str = "network.lan.dns";

    #[must_use]
    pub fn new(session_manager: OpenwrtSessionManager, timezone: Timezone) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(timezone);
        Self {
            session_manager,
            timezone_sender,
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

    fn get_active_ip() -> Option<IpAddr> {
        [Self::DEFAULT_INTERFACE, Self::DEFAULT_AP_INTERFACE_NAME]
            .iter()
            .find_map(|iface| Self::get_network_ip_by_name(iface))
            .or_else(get_ip_address)
    }
}

#[async_trait::async_trait]
impl BmcManager for Manager {
    type SessionManager = OpenwrtSessionManager;
    type Error = Error;

    fn version(&self) -> String {
        todo!()
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
        let zonename_cmd = format!("{}={}", Self::UCI_SYSTEM_ZONENAME, timezone.iana);
        call_command("uci", &["set", &zonename_cmd]).await?;

        let timezone_cmd: String = format!("{}={}", Self::UCI_SYSTEM_TIMEZONE, timezone.posix);
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

    async fn hostname(&self) -> Option<String> {
        match uci_get_opt(Self::UCI_SYSTEM_HOSTNAME).await {
            None => get_hostname().await,
            hostname => hostname,
        }
    }

    fn ip_address(&self) -> Option<IpAddr> {
        Self::get_active_ip()
    }

    fn mac_address(&self) -> Option<String> {
        Self::get_mac_address()
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
}

async fn uci_get_opt(opt: &str) -> Option<String> {
    call_command_to_string("uci", &["get", opt])
        .await
        .ok()
        .map(|value| value.trim().to_owned())
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ShadowFile(#[from] pwd::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Unix(#[from] unix::Error),
}

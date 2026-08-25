// Copyright (C) 2026  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bmc_net_drv::{
    NetworkInterface, default_gateway, default_resolv_conf_path, first_non_loopback_ip, hostname,
    nameservers,
};
use bmc_net_types::network::{
    IfaceData, NetworkInfo, NetworkProtocol, NetworkProtocolConfig, NetworkProtocolConfigStatic,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

use crate::command::call_command;
use crate::provisioning::{NoProvisioning, ProvisioningState};
use crate::{NetworkConfig, NetworkManager, WifiControl};

const DEFAULT_HOSTNAME: &str = "Antminer";
/// Config file consumed by the `S38network` init script on buildroot boards.
const NETWORK_CONF_PATH: &str = "/etc/network.conf";
const NETWORK_SERVICE_PATH: &str = "/etc/init.d/S38network";

const CONF_HOSTNAME: &str = "hostname";
const CONF_DHCP: &str = "dhcp";
const CONF_IPADDRESS: &str = "ipaddress";
const CONF_GATEWAY: &str = "gateway";
const CONF_NETMASK: &str = "netmask";
const CONF_DNSSERVERS: &str = "dnsservers";

/// Buildroot implementation of [`NetworkManager`] for ethernet-only boards
/// (Amlogic, BeagleBone, CVITEK). Network config lives in `/etc/network.conf`,
/// applied by the `S38network` service; there is no WiFi and no
/// factory-default/setup state machine.
#[derive(Debug)]
pub struct BuildrootNetworkManager {
    /// Primary network interface (e.g. "eth0") for IP/MAC/info lookups.
    interface_name: String,
    provisioning: NoProvisioning,
    /// Serializes the read-modify-write of `/etc/network.conf` across the
    /// concurrent `&self` setters reached from the gRPC server, so two edits
    /// cannot interleave or lose an update.
    config_lock: tokio::sync::Mutex<()>,
    /// Signalled after every successful hostname write; see
    /// [`NetworkConfig::hostname_change_notifier`].
    hostname_changed: Arc<Notify>,
}

impl BuildrootNetworkManager {
    #[must_use]
    pub fn new(interface_name: String) -> Self {
        Self {
            interface_name,
            provisioning: NoProvisioning::default(),
            config_lock: tokio::sync::Mutex::new(()),
            hostname_changed: Arc::new(Notify::new()),
        }
    }

    /// Load `/etc/network.conf`. A missing file yields the default config, but a
    /// file that is present yet unparseable is an error rather than a silent
    /// default: the read-modify-write setters must not clobber a config we could
    /// not read (e.g. a single malformed line would otherwise flip a static
    /// setup to DHCP and drop the address on the next hostname change).
    async fn load_config(&self) -> Result<NetworkConf> {
        match tokio::fs::read_to_string(NETWORK_CONF_PATH).await {
            Ok(contents) => NetworkConf::parse(&contents)
                .ok_or_else(|| anyhow!("{NETWORK_CONF_PATH} exists but could not be parsed")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NetworkConf::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Write `/etc/network.conf` and restart the `S38network` service so the new
    /// configuration is applied.
    ///
    /// On Amlogic/BeagleBone/CVITEK `/etc/network.conf` is a symlink into the
    /// persistent stock partition (`/mnt/$BOS_ENV_STOCK_CONFIG/network.conf`),
    /// which `S38network` recreates from that copy on boot. We therefore write
    /// through the link to its target rather than renaming a fresh file over
    /// the link itself: `rename(2)` does not follow symlinks, so renaming over
    /// `/etc/network.conf` would replace the link with a regular file in the
    /// volatile `/etc` overlay and the change would be lost on the next reboot.
    /// [`Self::load_config`] reads via `read_to_string`, which follows the link,
    /// so reads were already correct — only this write path needed to.
    ///
    /// The write goes to a per-process temp file in the target's own directory
    /// which is then renamed over the target: rename is atomic on the same
    /// filesystem, so a power loss mid-write cannot leave a truncated config
    /// (which `load_config` would treat as a hard error, bricking all setters
    /// until manual intervention). The target's directory is fsynced after the
    /// rename so the new directory entry is itself durable across a power cut.
    /// The temp name includes the pid so a racing process cannot share it;
    /// same-process concurrency is serialized by `config_lock` in the callers.
    async fn store_config(&self, config: &NetworkConf) -> Result<()> {
        let target = resolve_link_target(NETWORK_CONF_PATH).await;
        let temp_name = format!(".network.conf.{}.tmp", std::process::id());
        let temp_path = match target.parent() {
            Some(dir) => dir.join(temp_name),
            None => PathBuf::from(temp_name),
        };
        let mut file = tokio::fs::File::create(&temp_path).await?;
        file.write_all(config.serialize().as_bytes()).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp_path, &target).await?;
        if let Some(parent) = target.parent() {
            tokio::fs::File::open(parent).await?.sync_all().await?;
        }
        call_command(NETWORK_SERVICE_PATH, &["restart"]).await
    }

    fn mac_address_lookup(&self) -> Option<String> {
        NetworkInterface::get_by_name(&self.interface_name)
            .or_else(NetworkInterface::find_default)
            .and_then(|network| network.mac_address().map(|mac| mac.to_string()))
    }
}

/// Resolve the file to actually write for `path`. When `path` is a symlink (as
/// `/etc/network.conf` is on the buildroot boards, pointing into the persistent
/// stock partition), follow it one level so the write lands on the persistent
/// target and the link is preserved; a relative link target is resolved against
/// the link's own directory. Otherwise `path` is written directly. If `path`
/// does not exist yet, it is written directly (the first write creates it).
async fn resolve_link_target(path: &str) -> PathBuf {
    let path = Path::new(path);
    let Ok(meta) = tokio::fs::symlink_metadata(path).await else {
        return path.to_owned();
    };
    if !meta.file_type().is_symlink() {
        return path.to_owned();
    }
    match tokio::fs::read_link(path).await {
        Ok(target) if target.is_absolute() => target,
        Ok(target) => match path.parent() {
            Some(dir) => dir.join(target),
            None => target,
        },
        Err(_) => path.to_owned(),
    }
}

#[async_trait]
impl NetworkConfig for BuildrootNetworkManager {
    async fn hostname(&self) -> Option<String> {
        self.load_config()
            .await
            .ok()
            .and_then(|config| config.hostname)
            .or_else(hostname)
    }

    fn mac_address(&self) -> Option<String> {
        self.mac_address_lookup()
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        // `getifaddrs(3)` walk; keep it off the async executor.
        let interface_name = self.interface_name.clone();
        tokio::task::spawn_blocking(move || {
            NetworkInterface::get_by_name(&interface_name)
                .and_then(|n| n.ipv4_address())
                .or_else(first_non_loopback_ip)
        })
        .await
        .expect("BUG: IP address interface walk task panicked")
    }

    async fn network_config(&self) -> Option<NetworkProtocolConfig> {
        self.load_config().await.ok().map(|config| config.protocol)
    }

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> Result<()> {
        self.apply_network_settings(Some(config), None).await
    }

    async fn set_hostname(&self, new_hostname: String) -> Result<()> {
        self.apply_network_settings(None, Some(new_hostname)).await
    }

    /// Both settings live in the same `/etc/network.conf`, so applying them
    /// together is one read-modify-write and one `S38network` restart instead
    /// of two.
    async fn apply_network_settings(
        &self,
        config: Option<NetworkProtocolConfig>,
        new_hostname: Option<String>,
    ) -> Result<()> {
        if let Some(new_hostname) = new_hostname.as_deref() {
            crate::validate_hostname(new_hostname)?;
        }
        if config.is_none() && new_hostname.is_none() {
            return Ok(());
        }
        let _guard = self.config_lock.lock().await;
        let mut current = self.load_config().await?;
        if let Some(config) = config {
            current.protocol = config;
        }
        let hostname_changing = new_hostname.is_some();
        if let Some(new_hostname) = new_hostname {
            current.hostname = Some(new_hostname);
        }
        // `S38network` expects the hostname to always be present, otherwise it
        // passes an empty `hostname:` to udhcpc and keeps the stale value.
        if current.hostname.is_none() {
            current.hostname = Some(hostname().unwrap_or_else(|| DEFAULT_HOSTNAME.into()));
        }
        self.store_config(&current).await?;
        if hostname_changing {
            self.hostname_changed.notify_one();
        }
        Ok(())
    }

    async fn network_info(&self) -> Result<NetworkInfo> {
        let config = self.load_config().await?;
        let (dns_servers, gateway) = match &config.protocol {
            NetworkProtocolConfig::Static(static_config) => {
                // `UNSPECIFIED` (0.0.0.0) is the in-memory marker for a config
                // with no `gateway=` line; report it as "no default route"
                // rather than a bogus default gateway.
                let gateway = (static_config.gateway != Ipv4Addr::UNSPECIFIED)
                    .then_some(static_config.gateway);
                (static_config.dns_servers.clone(), gateway)
            }
            NetworkProtocolConfig::Dhcp => (
                nameservers(default_resolv_conf_path()).await,
                default_gateway(&self.interface_name).await,
            ),
        };
        // One off-executor `getifaddrs(3)` walk feeds both the MAC and the
        // networks: bind the interface once and reuse it for both.
        let interface_name = self.interface_name.clone();
        let (mac_address, networks) = tokio::task::spawn_blocking(move || {
            let iface = NetworkInterface::get_by_name(&interface_name)
                .or_else(NetworkInterface::find_default);
            let mac_address = iface.as_ref().and_then(NetworkInterface::mac_address);
            let networks = iface.map(|iface| iface.ipv4_networks()).unwrap_or_default();
            (mac_address, networks)
        })
        .await
        .expect("BUG: interface walk task panicked");
        Ok(NetworkInfo {
            interface_name: self.interface_name.clone(),
            mac_address,
            hostname: config.hostname.clone().or_else(hostname),
            protocol: Some(NetworkProtocol::from(&config.protocol)),
            dns_servers,
            networks,
            default_gateway: gateway,
        })
    }

    fn eth_data(&self) -> IfaceData {
        NetworkInterface::get_by_name(&self.interface_name)
            .or_else(NetworkInterface::find_default)
            .map(|iface| iface.iface_data())
            .unwrap_or_default()
    }

    fn hostname_change_notifier(&self) -> Arc<Notify> {
        self.hostname_changed.clone()
    }
}

impl NetworkManager for BuildrootNetworkManager {
    /// Buildroot boards are ethernet-only.
    fn wifi(&self) -> Option<&dyn WifiControl> {
        None
    }

    fn provisioning(&self) -> &dyn ProvisioningState {
        &self.provisioning
    }
}

/// Parsed contents of `/etc/network.conf`.
#[derive(Debug, Default)]
struct NetworkConf {
    protocol: NetworkProtocolConfig,
    hostname: Option<String>,
}

impl NetworkConf {
    fn parse(text: &str) -> Option<Self> {
        let fields: HashMap<&str, &str> = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim(), value.trim()))
            .collect();

        let hostname = fields.get(CONF_HOSTNAME).map(ToString::to_string);

        let protocol = if fields.get(CONF_DHCP) == Some(&"true") {
            NetworkProtocolConfig::Dhcp
        } else {
            // Only `ipaddress`/`netmask` are genuinely required; an absent
            // `gateway`/`dnsservers` line (legacy or externally-written files)
            // falls back to a default rather than making the whole config
            // unparseable, which `load_config` would turn into a setter-bricking
            // hard error. An absent gateway is represented by `UNSPECIFIED`
            // (0.0.0.0), which `serialize`/`network_info` treat as "no gateway".
            let gateway = match fields.get(CONF_GATEWAY) {
                Some(value) => value.parse().ok()?,
                None => Ipv4Addr::UNSPECIFIED,
            };
            // A single garbage DNS entry must not brick the whole config (a bad
            // one is simply skipped), mirroring the OpenWrt `uci_dns_servers`
            // behaviour; DNS servers are not load-bearing for a safe rewrite.
            let dns_servers = fields
                .get(CONF_DNSSERVERS)
                .map(|value| {
                    value
                        .split_whitespace()
                        .filter_map(|s| Ipv4Addr::from_str(s).ok())
                        .collect()
                })
                .unwrap_or_default();
            NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
                address: fields.get(CONF_IPADDRESS)?.parse().ok()?,
                netmask: fields.get(CONF_NETMASK)?.parse().ok()?,
                gateway,
                dns_servers,
            })
        };

        Some(Self { protocol, hostname })
    }

    fn serialize(&self) -> String {
        let mut lines = vec![];

        if let Some(hostname) = &self.hostname {
            lines.push(format!("{CONF_HOSTNAME}={hostname}"));
        }

        match &self.protocol {
            NetworkProtocolConfig::Dhcp => lines.push(format!("{CONF_DHCP}=true")),
            NetworkProtocolConfig::Static(config) => {
                let dns_servers = config
                    .dns_servers
                    .iter()
                    .map(Ipv4Addr::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                lines.push(format!("{CONF_IPADDRESS}={}", config.address));
                // Omit the gateway line entirely for a gateway-less config
                // rather than persisting a bogus `gateway=0.0.0.0`.
                if config.gateway != Ipv4Addr::UNSPECIFIED {
                    lines.push(format!("{CONF_GATEWAY}={}", config.gateway));
                }
                lines.push(format!("{CONF_NETMASK}={}", config.netmask));
                lines.push(format!("{CONF_DNSSERVERS}={dns_servers}"));
            }
        }

        let mut text = lines.join("\n");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATIC_CONF: &str = "hostname=localhost\nipaddress=10.33.50.111\ngateway=10.33.50.1\nnetmask=255.255.255.0\ndnsservers=8.8.8.8 8.8.4.4\n";
    const DHCP_CONF: &str = "hostname=localhost\ndhcp=true\n";

    fn static_conf() -> NetworkConf {
        NetworkConf {
            hostname: Some("localhost".to_owned()),
            protocol: NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
                address: Ipv4Addr::new(10, 33, 50, 111),
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                gateway: Ipv4Addr::new(10, 33, 50, 1),
                dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(8, 8, 4, 4)],
            }),
        }
    }

    fn dhcp_conf() -> NetworkConf {
        NetworkConf {
            hostname: Some("localhost".to_owned()),
            protocol: NetworkProtocolConfig::Dhcp,
        }
    }

    #[test]
    fn static_config_roundtrip() {
        assert_eq!(static_conf().serialize(), STATIC_CONF);
        let parsed = NetworkConf::parse(STATIC_CONF).expect("BUG: parse failed");
        assert_eq!(parsed.hostname, static_conf().hostname);
        assert_eq!(parsed.protocol, static_conf().protocol);
    }

    #[test]
    fn gateway_less_static_config_omits_gateway_line() {
        // A legacy file without a `gateway=` line parses as UNSPECIFIED and must
        // not gain a bogus `gateway=0.0.0.0` line when serialized back.
        const NO_GATEWAY_CONF: &str = "hostname=localhost\nipaddress=10.33.50.111\nnetmask=255.255.255.0\ndnsservers=8.8.8.8\n";
        let parsed = NetworkConf::parse(NO_GATEWAY_CONF).expect("BUG: parse failed");
        let NetworkProtocolConfig::Static(static_config) = &parsed.protocol else {
            panic!("BUG: expected static config");
        };
        assert_eq!(static_config.gateway, Ipv4Addr::UNSPECIFIED);
        let serialized = parsed.serialize();
        assert!(!serialized.contains(CONF_GATEWAY));
        assert_eq!(
            NetworkConf::parse(&serialized)
                .expect("BUG: reparse failed")
                .protocol,
            parsed.protocol
        );
    }

    #[test]
    fn dhcp_config_roundtrip() {
        assert_eq!(dhcp_conf().serialize(), DHCP_CONF);
        let parsed = NetworkConf::parse(DHCP_CONF).expect("BUG: parse failed");
        assert_eq!(parsed.hostname, dhcp_conf().hostname);
        assert_eq!(parsed.protocol, dhcp_conf().protocol);
    }
}

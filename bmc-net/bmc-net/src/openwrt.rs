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

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use bmc_net_drv::wifi::WifiDriver;
use bmc_net_drv::{NetworkInterface, first_non_loopback_ip, hostname};
use bmc_net_types::MacAddr;
use bmc_net_types::network::{
    BmcState, IfaceData, InitialSetupError, NetworkInfo, NetworkProtocol, NetworkProtocolConfig,
    NetworkProtocolConfigStatic, WifiData, WifiEvent, WifiNetworkConfig,
};
use bmc_net_types::wifi::{EncryptionType, WifiMode, WifiScanItem, WifiStatus};
use tokio::sync::broadcast;
use tracing::info;

use crate::command::{
    BOS_DEFAULTS_LIB, BOS_FACTORY_DEFAULT_LIB, call_command, call_command_stdin,
    call_command_to_string, run_sourced, run_sourced_to_string,
};
use crate::provisioning::{ProvisioningState, UciProvisioningState};
use crate::{NetworkConfig, NetworkManager, WifiControl};

const WIFI_EVENTS_CAPACITY: usize = 10;

const UCI_SYSTEM_HOSTNAME: &str = "system.@system[0].hostname";
const UCI_NET_LAN_PROTO_DHCP: &str = "dhcp";
const UCI_NET_LAN_PROTO_STATIC: &str = "static";

const INIT_SCRIPT_NETWORK: &str = "/etc/init.d/network";
const INIT_SCRIPT_SYSTEM: &str = "/etc/init.d/system";
const INIT_SCRIPT_DNSMASQ: &str = "/etc/init.d/dnsmasq";

/// Number of trailing MAC hex characters used as the setup-AP SSID suffix.
const MAC_SSID_SUFFIX_LEN: usize = 3;
/// SSID suffix placeholder used when the MAC is too short to derive one.
const MAC_UNKNOWN_SUFFIX: &str = "UNK";

/// Resolve the interface whose addresses describe this device's connectivity.
///
/// `interface_name` wins while it carries an IPv4, which keeps an ethernet
/// miner reporting `eth0`. A configured interface that exists but holds no
/// address (a WiFi-only board, where `eth0` is present and empty) would
/// otherwise report no address at all, so the uplink is then taken from
/// [`bmc_net_observe::primary_interface`] — the same ranking the connectivity
/// view uses, which prefers a station interface and never lets a setup AP
/// shadow the real uplink. The routing table is the last resort.
///
/// Blocking (`getifaddrs(3)`); async callers use
/// [`UciNetworkManager::primary_iface_nonblocking`].
fn primary_iface(interface_name: &str) -> Option<NetworkInterface> {
    NetworkInterface::get_by_name(interface_name)
        .filter(|iface| !iface.ipv4_networks().is_empty())
        .or_else(|| {
            bmc_net_observe::primary_interface().and_then(|name| {
                NetworkInterface::get_by_name(&name).or_else(NetworkInterface::find_default)
            })
        })
        .or_else(NetworkInterface::find_default)
}

/// UCI (`bos-defaults.sh` + `uci`) implementation of [`NetworkManager`]:
/// network config, the factory-default/setup state machine, the setup captive
/// portal, and a WiFi driver for scan/connect/AP. Shared by the stm32mp15
/// display and the OpenWrt/LEDE miners, so it is named for the UCI mechanism
/// rather than a single platform.
#[derive(Debug)]
pub struct UciNetworkManager {
    /// WiFi driver; `None` on platforms without WiFi (e.g. ethernet-only miners).
    wifi: Option<Arc<dyn WifiDriver>>,
    /// UCI network section this manager configures ("lan" or "wifi_sta").
    network_section: String,
    /// Primary network interface (e.g. "eth0", "wlan0") for IP/MAC/info lookups.
    interface_name: String,
    /// Product display name used to build the setup-AP SSID.
    product_name: String,
    wifi_event_sender: broadcast::Sender<WifiEvent>,
    /// Device provisioning state machine (factory-default/setup/reconfig flags).
    provisioning: Arc<dyn ProvisioningState>,
}

impl UciNetworkManager {
    pub async fn new(
        network_section: impl Into<String>,
        wifi: Option<Arc<dyn WifiDriver>>,
        interface_name: String,
        product_name: String,
    ) -> Self {
        let network_section = network_section.into();
        let (wifi_event_sender, _) = broadcast::channel(WIFI_EVENTS_CAPACITY);

        Self {
            wifi,
            network_section,
            interface_name,
            product_name,
            wifi_event_sender,
            provisioning: Arc::new(UciProvisioningState::new().await),
        }
    }

    fn wifi_driver(&self) -> Result<&dyn WifiDriver> {
        self.wifi
            .as_deref()
            .ok_or_else(|| anyhow!("WiFi is not available on this platform"))
    }

    fn uci_net(&self) -> String {
        format!("network.{}", self.network_section)
    }

    fn uci_net_opt(&self, option: &str) -> String {
        format!("network.{}.{option}", self.network_section)
    }

    /// [`primary_iface`] run on the blocking thread pool.
    async fn primary_iface_nonblocking(&self) -> Option<NetworkInterface> {
        let interface_name = self.interface_name.clone();
        tokio::task::spawn_blocking(move || primary_iface(&interface_name))
            .await
            .expect("BUG: primary interface lookup task panicked")
    }

    fn mac_address_lookup(&self) -> Option<String> {
        primary_iface(&self.interface_name)
            .and_then(|network| network.mac_address().map(|mac| mac.to_string()))
    }

    /// Reads a UCI network option and parses it as an IPv4 address. A
    /// present-but-unparseable value is logged (rather than silently reported
    /// as "unconfigured") before returning `None`.
    async fn uci_ipv4(&self, option: &str) -> Option<Ipv4Addr> {
        let opt = self.uci_net_opt(option);
        let raw = uci_get_opt(&opt).await?;
        let Ok(addr) = raw.parse() else {
            tracing::warn!("uci option `{opt}` holds an unparseable IPv4 address `{raw}`");
            return None;
        };
        Some(addr)
    }

    /// Configured DNS servers from the UCI `dns` option (whitespace-separated),
    /// skipping any unparseable entry.
    async fn uci_dns_servers(&self) -> Vec<Ipv4Addr> {
        uci_get_opt(&self.uci_net_opt("dns"))
            .await
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|dns| dns.parse().ok())
            .collect()
    }

    async fn calculate_wifi_ssid(&self) -> Result<String> {
        let mac = run_defaults_script_output("wifi_mac").await?;
        Ok(format!(
            "{} {}",
            self.product_name,
            mac_short_id(mac.trim())
        ))
    }

    async fn configure_wifi_ap(&self) -> Result<(), InitialSetupError> {
        let ssid = self
            .calculate_wifi_ssid()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        info!(%ssid, "Configuring WiFi AP for initial setup");

        let to_err = |e: anyhow::Error| InitialSetupError::UnexpectedFailure(e.to_string());
        let wifi = self.wifi_driver().map_err(to_err)?;
        wifi.reset_config().await.map_err(to_err)?;
        wifi.configure_ap_mode(ssid, None, EncryptionType::None)
            .await
            .map_err(to_err)?;
        wifi.enable_radio(true).await.map_err(to_err)?;
        // The reload above only queues the reconfiguration, so returning here
        // would report success while the radio is still switching modes.
        wifi.wait_for_ap_active().await.map_err(to_err)?;
        Ok(())
    }

    async fn enable_captive_portal(&self) -> Result<(), InitialSetupError> {
        run_factory_default_script(&format!(
            "enable_captive_portal $FACTORY_DEFAULT_AP_IP_ADDR && {INIT_SCRIPT_DNSMASQ} restart"
        ))
        .await
        .map_err(|e| InitialSetupError::UnexpectedFailure(format!("enable captive portal: {e}")))
    }

    async fn disable_captive_portal(&self) -> Result<(), InitialSetupError> {
        run_factory_default_script(&format!(
            "disable_captive_portal && {INIT_SCRIPT_DNSMASQ} restart"
        ))
        .await
        .map_err(|e| InitialSetupError::UnexpectedFailure(format!("disable captive portal: {e}")))
    }
}

#[async_trait]
impl NetworkConfig for UciNetworkManager {
    async fn hostname(&self) -> Option<String> {
        uci_get_opt(UCI_SYSTEM_HOSTNAME).await.or_else(hostname)
    }

    fn mac_address(&self) -> Option<String> {
        self.mac_address_lookup()
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        // Keep the fallback in the same blocking task as the primary lookup.
        let interface_name = self.interface_name.clone();
        tokio::task::spawn_blocking(move || {
            primary_iface(&interface_name)
                .and_then(|n| n.ipv4_address())
                .or_else(first_non_loopback_ip)
        })
        .await
        .expect("BUG: IP address interface walk task panicked")
    }

    /// One `uci get` rather than the five `network_config` needs.
    async fn network_protocol(&self) -> Option<NetworkProtocol> {
        match uci_get_opt(&self.uci_net_opt("proto")).await.as_deref() {
            Some(UCI_NET_LAN_PROTO_DHCP) => Some(NetworkProtocol::Dhcp),
            Some(UCI_NET_LAN_PROTO_STATIC) => Some(NetworkProtocol::Static),
            _ => None,
        }
    }

    async fn network_config(&self) -> Option<NetworkProtocolConfig> {
        let config = match self.network_protocol().await? {
            NetworkProtocol::Dhcp => NetworkProtocolConfig::Dhcp,
            NetworkProtocol::Static => NetworkProtocolConfig::Static(NetworkProtocolConfigStatic {
                address: self.uci_ipv4("ipaddr").await?,
                netmask: self.uci_ipv4("netmask").await?,
                // A section without a `gateway` option is a gateway-less static
                // config, not an unconfigured one: report the shared
                // `UNSPECIFIED == no gateway` marker the buildroot backend uses.
                gateway: self
                    .uci_ipv4("gateway")
                    .await
                    .unwrap_or(Ipv4Addr::UNSPECIFIED),
                dns_servers: self.uci_dns_servers().await,
            }),
        };
        Some(config)
    }

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> Result<()> {
        let proto = self.uci_net_opt("proto");
        let ipaddr = self.uci_net_opt("ipaddr");
        let netmask = self.uci_net_opt("netmask");
        let gateway = self.uci_net_opt("gateway");
        let dns = self.uci_net_opt("dns");
        let mut stdin = match config {
            NetworkProtocolConfig::Dhcp => vec![
                format!("set {proto}='{UCI_NET_LAN_PROTO_DHCP}'"),
                format!("delete {ipaddr}"),
                format!("delete {netmask}"),
                format!("delete {gateway}"),
                format!("delete {dns}"),
            ],
            NetworkProtocolConfig::Static(config) => vec![
                format!("set {proto}='{UCI_NET_LAN_PROTO_STATIC}'"),
                format!("set {ipaddr}='{}'", config.address),
                format!("set {netmask}='{}'", config.netmask),
                // `UNSPECIFIED` (0.0.0.0) is the shared in-memory marker for "no
                // gateway"; drop the option rather than persisting a bogus
                // `gateway='0.0.0.0'`, matching the buildroot backend.
                if config.gateway == Ipv4Addr::UNSPECIFIED {
                    format!("delete {gateway}")
                } else {
                    format!("set {gateway}='{}'", config.gateway)
                },
                format!(
                    "set {dns}='{}'",
                    config
                        .dns_servers
                        .iter()
                        .map(Ipv4Addr::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            ],
        };
        stdin.push(format!("commit {}", self.uci_net()));

        let output = call_command_stdin("uci", &["-q", "batch"], &stdin.join("\n")).await?;
        // Judge success by the exit status alone. The batch has already
        // committed by this point, so treating a benign warning on stderr as a
        // failure would report an error for a configuration that did take
        // effect — the worst of both outcomes. stderr only enriches the message.
        if !output.status.success() {
            bail!(
                "`uci batch` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if !output.stderr.is_empty() {
            tracing::warn!(
                "`uci batch` succeeded with warnings: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        call_command(INIT_SCRIPT_NETWORK, &["restart"]).await
    }

    async fn set_hostname(&self, hostname: String) -> Result<()> {
        crate::validate_hostname(&hostname)?;
        call_command(
            "uci",
            &["set", &format!("{UCI_SYSTEM_HOSTNAME}={hostname}")],
        )
        .await?;
        call_command("uci", &["commit", "system"]).await?;
        call_command(INIT_SCRIPT_SYSTEM, &["reload"]).await?;
        // NOTE: this intentionally restarts networking, which drops the client
        // connection that issued the request. udhcpc only re-advertises the
        // hostname (`-x hostname:<name>`) when the network service (re)starts;
        // a `system reload` alone updates the kernel hostname but leaves the
        // active DHCP lease on the old name, so the restart is required for the
        // rename to actually take effect.
        call_command(INIT_SCRIPT_NETWORK, &["restart"]).await
    }

    async fn network_info(&self) -> Result<NetworkInfo> {
        // One off-executor interface walk feeds both the MAC and the networks.
        let iface = self.primary_iface_nonblocking().await;
        Ok(NetworkInfo {
            interface_name: self.interface_name.clone(),
            mac_address: iface.as_ref().and_then(NetworkInterface::mac_address),
            hostname: self.hostname().await,
            protocol: self.network_protocol().await,
            // Live runtime values (resolv.conf + dnsmasq fallback, and the
            // rtnetlink default route) rather than the UCI config, so the
            // reported DNS/gateway are correct on DHCP too, not just static.
            dns_servers: bmc_net_drv::resolved_nameservers().await,
            networks: iface.map(|iface| iface.ipv4_networks()).unwrap_or_default(),
            default_gateway: bmc_net_drv::default_gateway(&self.interface_name).await,
        })
    }

    /// Blocking `getifaddrs(3)` walk; do not call from latency-sensitive
    /// async contexts.
    fn eth_data(&self) -> IfaceData {
        primary_iface(&self.interface_name)
            .map(|iface| iface.iface_data())
            .unwrap_or_default()
    }
}

#[async_trait]
impl WifiControl for UciNetworkManager {
    async fn scan(&self) -> Result<Vec<WifiScanItem>> {
        // Signal scan end even if the future is cancelled before returning.
        struct DropGuard(broadcast::Sender<WifiEvent>);
        impl Drop for DropGuard {
            fn drop(&mut self) {
                let _ = self.0.send(WifiEvent::ScanEnded);
            }
        }
        let _guard = DropGuard(self.wifi_event_sender.clone());
        let _ = self.wifi_event_sender.send(WifiEvent::ScanStarted);

        self.wifi_driver()?.scan().await
    }

    async fn status(&self) -> Result<WifiData> {
        let wifi = self.wifi_driver()?;
        let status = wifi.status().await?;
        // Report the wireless interface's own IP/MAC. `get_primary_interface`
        // returns the first running non-loopback interface with an IPv4, which
        // on a miner with ethernet up is eth0 — the WiFi tile would then show
        // ethernet data. The walk stays off-executor either way.
        let device = wifi.wifi_device_name().await?;
        let lookup = device.clone();
        let iface = tokio::task::spawn_blocking(move || NetworkInterface::get_by_substr(&lookup))
            .await
            .expect("BUG: WiFi interface lookup task panicked")
            .ok_or_else(|| anyhow!("Wi-Fi interface {device} not found"))?;
        Ok(WifiData {
            iface: iface.iface_data(),
            status,
        })
    }

    async fn saved_networks(&self) -> Result<Vec<WifiStatus>> {
        Ok(self
            .wifi_driver()?
            .status_all()
            .await?
            .into_iter()
            .filter(|status| {
                status
                    .configuration
                    .as_ref()
                    .is_some_and(|conf| conf.mode == WifiMode::Station)
            })
            .collect())
    }

    async fn ssid(&self) -> Result<String> {
        let wifi = self.wifi_driver()?;
        if let Some(ssid) = wifi.ap_ssid().await {
            return Ok(ssid);
        }
        // NOTE: per the trait contract, fall back to the joined station SSID
        // when no AP is advertised. Callers must not treat an error as "not in
        // AP mode": an operational station yields its network's SSID.
        wifi.sta_ssid()
            .await
            .ok_or_else(|| anyhow!("Wi-Fi neither in AP mode nor joined to a network"))
    }

    async fn wifi_save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<()> {
        self.wifi_driver()?
            .save_and_connect(ssid, password, encryption)
            .await
    }

    async fn set_wifi_enabled(&self, enable: bool) -> Result<()> {
        self.wifi_driver()?.enable_radio(enable).await
    }

    async fn init_wifi_ap(&self) -> Result<()> {
        let state = self.provisioning.device_state().await;
        if matches!(
            state,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        ) {
            self.configure_wifi_ap().await?;
        }
        // Factory default has the captive portal pre-configured; reconfig needs it enabled.
        if state == BmcState::WifiReconfiguration {
            self.enable_captive_portal().await?;
        }
        // A provisioned device must not keep the portal: factory-default images
        // ship the dnsmasq hijack already in place, so a board that left setup
        // without going through `stop_wifi_ap` would resolve every hijacked
        // domain to the setup AP and never reach its pool. Clearing it here
        // makes the startup path self-healing regardless of how setup ended.
        if !matches!(
            state,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        ) {
            self.disable_captive_portal().await?;
        }
        // The watch was seeded from the flag at construction, i.e. before the
        // AP existed. Re-publish now that it is verified up so watchers that
        // resolved an SSID too early are woken to try again.
        if matches!(
            state,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        ) {
            self.provisioning.publish_setup_ap_active(true);
        }
        Ok(())
    }

    async fn stop_wifi_ap(&self) -> Result<()> {
        self.wifi_driver()?.stop_ap().await?;
        // The captive portal must be gone once the setup AP is down; a
        // failure here means dnsmasq may still hijack DNS, so surface it.
        self.disable_captive_portal().await?;
        Ok(())
    }

    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError> {
        info!(ssid = %config.ssid, "Connecting to WiFi for initial setup");
        self.wifi_driver()
            .map_err(|e| InitialSetupError::WifiConnectionFailure(e.to_string()))?
            .save_and_connect(config.ssid, config.password, config.encryption)
            .await
            .map_err(|e| InitialSetupError::WifiConnectionFailure(e.to_string()))?;
        self.disable_captive_portal().await?;
        self.provisioning
            .advance()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
    }

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError> {
        if !self.provisioning.is_factory_default().await {
            return Err(InitialSetupError::NotSupported);
        }
        self.configure_wifi_ap().await
    }

    async fn enter_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
        info!("Entering WiFi reconfiguration mode");
        self.provisioning
            .mark_wifi_reconfig()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))?;
        self.configure_wifi_ap().await?;
        self.enable_captive_portal().await?;
        // Announce setup mode last. Watchers resolve the AP SSID the moment
        // this flips and do not poll again, so publishing before the AP is
        // broadcasting leaves them stuck on their previous value.
        self.provisioning.publish_setup_ap_active(true);
        Ok(())
    }

    async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
        if !self.provisioning.is_wifi_reconfig().await {
            return Ok(());
        }
        info!("Exiting WiFi reconfiguration mode");
        // NOTE: leaving reconfiguration must not keep the AP broadcasting, so
        // this tears down both the radio and the captive portal. `stop_ap`
        // disables only the AP-mode iface, preserving any station configured
        // earlier in the session.
        self.stop_wifi_ap()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(format!("stop wifi ap: {e}")))?;
        self.provisioning
            .clear_wifi_reconfig()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
    }

    async fn captive_portal_redirect_host(&self) -> Option<String> {
        self.ip_address().await.map(|ip| ip.to_string())
    }

    fn subscribe_wifi_events(&self) -> broadcast::Receiver<WifiEvent> {
        self.wifi_event_sender.subscribe()
    }
}

impl NetworkManager for UciNetworkManager {
    fn wifi(&self) -> Option<&dyn WifiControl> {
        // Honour the trait contract on ethernet-only boards: without a driver
        // there is no WiFi capability to hand out, so callers discover that
        // through `None` instead of hitting a runtime error per method.
        self.wifi.is_some().then_some(self as &dyn WifiControl)
    }

    fn provisioning(&self) -> &dyn ProvisioningState {
        self.provisioning.as_ref()
    }
}

fn mac_short_id(mac: &str) -> String {
    let mac = mac.replace(MacAddr::DELIMITER, "");
    // Last MAC_SSID_SUFFIX_LEN characters, kept in order; `chars()` avoids
    // string indexing (forbidden by clippy::string_slice) without the
    // byte -> lossy-utf8 round-trip. Fewer than that many characters is a
    // malformed MAC, so fall back to the unknown marker.
    mac.chars()
        .count()
        .checked_sub(MAC_SSID_SUFFIX_LEN)
        .map_or_else(
            || MAC_UNKNOWN_SUFFIX.to_owned(),
            |skip| mac.chars().skip(skip).collect(),
        )
}

async fn uci_get_opt(opt: &str) -> Option<String> {
    call_command_to_string("uci", &["get", opt])
        .await
        .ok()
        .map(|value| value.trim().to_owned())
}

/// Run a bos-defaults snippet and return its stdout.
async fn run_defaults_script_output(snippet: &str) -> Result<String> {
    run_sourced_to_string(BOS_DEFAULTS_LIB, snippet).await
}

async fn run_factory_default_script(snippet: &str) -> Result<()> {
    run_sourced(BOS_FACTORY_DEFAULT_LIB, snippet).await
}

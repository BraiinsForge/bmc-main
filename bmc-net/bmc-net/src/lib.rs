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

//! Platform-independent network management for the `bmc-net` crate set.
//!
//! The [`NetworkManager`] trait is the single entry point: it abstracts WiFi,
//! ethernet/static-IP configuration, the factory-default/setup state machine,
//! and the setup captive portal. Concrete backends live in submodules
//! ([`openwrt`], [`buildroot`], and the [`mock`] test double) and are selected
//! by the platform. Consumers (the display binary and `boser`) depend only on
//! the trait, so networking behaviour is shared across products.
//!
//! Fallible methods return [`anyhow::Result`] for general failures, except the
//! interactive setup flow, which uses the typed
//! [`InitialSetupError`](bmc_net_types::network::InitialSetupError) so callers
//! can distinguish "unsupported" from a connection failure.

use std::net::IpAddr;
use std::sync::Arc;

use anyhow::bail;
use async_trait::async_trait;
use tokio::sync::Notify;

pub mod buildroot;
mod command;
pub mod mock;
pub mod openwrt;
pub mod provisioning;
use bmc_net_types::network::{
    IfaceData, InitialSetupError, NetworkInfo, NetworkProtocol, NetworkProtocolConfig, WifiData,
    WifiEvent, WifiNetworkConfig,
};
use bmc_net_types::wifi::{EncryptionType, WifiScanItem, WifiStatus};
use tokio::sync::broadcast;

/// Ethernet/static-IP configuration and interface facts. Every backend
/// implements this; it is the always-present half of [`NetworkManager`].
///
/// Read-only accessors return `Option`, where `None` means "not
/// available/unconfigured on this platform" rather than an error.
#[async_trait]
pub trait NetworkConfig: Send + Sync + std::fmt::Debug {
    /// System hostname, if one is configured.
    async fn hostname(&self) -> Option<String>;
    /// MAC address of the primary interface, formatted as `aa:bb:cc:dd:ee:ff`.
    fn mac_address(&self) -> Option<String>;
    /// Current IPv4/IPv6 address of the primary interface, if any.
    async fn ip_address(&self) -> Option<IpAddr>;
    /// Current network protocol configuration (DHCP or static), if configured.
    async fn network_config(&self) -> Option<NetworkProtocolConfig>;
    /// Just which protocol is configured, without the static-address detail.
    ///
    /// Backends override this when they can answer it more cheaply than by
    /// reading the whole configuration — the UCI backend needs one `uci get`
    /// here versus five for [`network_config`].
    ///
    /// [`network_config`]: NetworkConfig::network_config
    async fn network_protocol(&self) -> Option<NetworkProtocol> {
        self.network_config()
            .await
            .as_ref()
            .map(NetworkProtocol::from)
    }
    /// Applies a new network protocol configuration.
    async fn set_network_config(&self, config: NetworkProtocolConfig) -> anyhow::Result<()>;
    /// Sets the system hostname and restarts networking to apply it.
    async fn set_hostname(&self, hostname: String) -> anyhow::Result<()>;
    /// Applies a protocol configuration and/or a hostname in a single pass.
    ///
    /// Callers changing both must use this rather than calling
    /// [`set_network_config`] and [`set_hostname`] in sequence: each restarts
    /// networking on its own, so back-to-back calls disrupt the link twice and
    /// leave the device half-configured if interrupted in between. Backends
    /// override this to write both in one transaction and restart once.
    ///
    /// [`set_network_config`]: NetworkConfig::set_network_config
    /// [`set_hostname`]: NetworkConfig::set_hostname
    async fn apply_network_settings(
        &self,
        config: Option<NetworkProtocolConfig>,
        hostname: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(config) = config {
            self.set_network_config(config).await?;
        }
        if let Some(hostname) = hostname {
            self.set_hostname(hostname).await?;
        }
        Ok(())
    }
    /// Aggregated interface/hostname/DNS/gateway snapshot for display and PAPI.
    async fn network_info(&self) -> anyhow::Result<NetworkInfo>;
    /// Interface data (IP + MAC) for the manager's primary interface.
    ///
    /// Blocking `getifaddrs(3)` walk; avoid on latency-sensitive async paths.
    fn eth_data(&self) -> IfaceData;
    /// Notified after every successful hostname write, so consumers that
    /// advertise the hostname (the mDNS responder) follow renames without
    /// polling and without every caller having to remember to signal.
    fn hostname_change_notifier(&self) -> Arc<Notify>;
}

/// Optional WiFi capability: station scan/connect, the setup access point, and
/// the setup captive portal. Reached through [`NetworkManager::wifi`], which is
/// `None` on boards without WiFi, so callers discover support without catching
/// an error.
#[async_trait]
pub trait WifiControl: Send + Sync + std::fmt::Debug {
    /// Scans for visible access points.
    async fn scan(&self) -> anyhow::Result<Vec<WifiScanItem>>;
    /// Current WiFi status (mode, configuration, station link).
    async fn status(&self) -> anyhow::Result<WifiData>;
    /// Status of every saved WiFi network.
    async fn saved_networks(&self) -> anyhow::Result<Vec<WifiStatus>>;
    /// SSID of the access point the device is currently advertising, or `None`
    /// when no AP is up.
    ///
    /// Unlike [`ssid`], this never falls back to the joined station network, so
    /// callers that must show the *setup AP* name (the display's settings tray)
    /// cannot accidentally advertise the station SSID while the AP is still
    /// coming up.
    ///
    /// [`ssid`]: WifiControl::ssid
    async fn ap_ssid(&self) -> Option<String>;
    /// SSID the device currently advertises (AP) or is joined to (station).
    async fn ssid(&self) -> anyhow::Result<String>;
    /// Saves the given credentials and connects to the network in station mode.
    async fn wifi_save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> anyhow::Result<()>;
    /// Enables or disables the WiFi radio.
    async fn set_wifi_enabled(&self, enable: bool) -> anyhow::Result<()>;
    /// Brings up the WiFi access point using the current configuration.
    async fn init_wifi_ap(&self) -> anyhow::Result<()>;
    /// Brings the WiFi access point down (symmetric to [`init_wifi_ap`]) and
    /// disables the setup captive portal if it was active. Idempotent: a no-op
    /// when no AP is up.
    ///
    /// [`init_wifi_ap`]: WifiControl::init_wifi_ap
    async fn stop_wifi_ap(&self) -> anyhow::Result<()>;
    /// Runs first-time WiFi setup with the supplied credentials.
    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError>;
    /// Reverts to the initial setup AP (only valid while factory-default).
    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError>;
    /// Enters WiFi-reconfiguration mode (setup AP + captive portal).
    async fn enter_wifi_reconfiguration(&self) -> Result<(), InitialSetupError>;
    /// Leaves WiFi-reconfiguration mode; a no-op if not currently in it.
    async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError>;
    /// Host that the captive portal should redirect clients to, if active.
    async fn captive_portal_redirect_host(&self) -> Option<String>;
    /// Subscribes to WiFi lifecycle events (scan start/end).
    fn subscribe_wifi_events(&self) -> broadcast::Receiver<WifiEvent>;
}

/// Platform network facade composing the always-present [`NetworkConfig`] with
/// the optional [`WifiControl`] capability and the [`ProvisioningState`] state
/// machine, so networking behaviour is shared across the display binary and
/// boser.
///
/// [`ProvisioningState`]: crate::provisioning::ProvisioningState
pub trait NetworkManager: NetworkConfig {
    /// The WiFi control surface, or `None` on boards without WiFi.
    fn wifi(&self) -> Option<&dyn WifiControl>;
    /// The device provisioning state machine (factory-default/setup/reconfig).
    fn provisioning(&self) -> &dyn crate::provisioning::ProvisioningState;

    /// The WiFi control surface, erroring instead of returning `None` when the
    /// board has no WiFi — for callers that require it and want a message.
    fn require_wifi(&self) -> anyhow::Result<&dyn WifiControl> {
        self.wifi()
            .ok_or_else(|| bmc_net_types::wifi::WifiUnsupportedError.into())
    }
}

/// The WiFi interface's address while it is in station mode, i.e. the device's
/// uplink.
///
/// Narrower than [`NetworkConfig::ip_address`], which answers "is anything
/// addressable" and is right for the captive-portal host but not for "did this
/// device get onto the network".
///
/// `Ok(None)` is a confirmed absence: the interface is a station and holds no
/// address. `Err` is "unknown" — the mode or the interface could not be read,
/// which is not evidence either way. Callers that reset or reboot on a missing
/// uplink depend on the two staying distinct.
///
/// A free function rather than a [`NetworkManager`] method because the trait
/// has no `#[async_trait]`, and a native `async fn` on it would cost every
/// caller its `&dyn NetworkManager`.
pub async fn station_ip_address(manager: &dyn NetworkManager) -> anyhow::Result<Option<IpAddr>> {
    let data = manager.require_wifi()?.status().await?;
    let configuration = data
        .status
        .configuration
        .ok_or_else(|| anyhow::anyhow!("the enabled WiFi interface has no parsed configuration"))?;
    if configuration.mode != bmc_net_types::wifi::WifiMode::Station {
        // Not "no uplink": the question does not apply to an AP interface, and
        // `status` reads the first *enabled* section, which need not be the
        // station one.
        return Err(anyhow::anyhow!(
            "the enabled WiFi interface is in {:?} mode, not station",
            configuration.mode
        ));
    }
    Ok(data.iface.ip)
}

/// Validates a hostname against RFC 1123: at most 253 characters total,
/// dot-separated labels of 1-63 ASCII alphanumeric/hyphen characters, with
/// no label starting or ending with a hyphen.
///
/// Every [`NetworkManager::set_hostname`] backend calls this before writing
/// anything: the buildroot backend serializes the hostname into the
/// line-oriented `/etc/network.conf` (consumed by the `S38network` shell
/// script), so unvalidated input such as `"deck\ndhcp=true"` would inject
/// arbitrary config lines.
///
/// # Errors
///
/// Returns an error describing the first violated rule.
pub fn validate_hostname(hostname: &str) -> anyhow::Result<()> {
    /// RFC 1123 maximum total hostname length, in bytes.
    const MAX_HOSTNAME_LEN: usize = 253;
    /// RFC 1123 maximum length of a single dot-separated label, in bytes.
    const MAX_LABEL_LEN: usize = 63;

    if hostname.is_empty() {
        bail!("hostname must not be empty");
    }
    if hostname.len() > MAX_HOSTNAME_LEN {
        bail!("hostname exceeds {MAX_HOSTNAME_LEN} bytes");
    }
    for label in hostname.split('.') {
        if label.is_empty() || label.len() > MAX_LABEL_LEN {
            bail!("hostname labels must be 1-{MAX_LABEL_LEN} bytes: {hostname:?}");
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!(
                "hostname may only contain ASCII letters, digits, hyphens and dots: {hostname:?}"
            );
        }
        if label.starts_with('-') || label.ends_with('-') {
            bail!("hostname labels must not start or end with a hyphen: {hostname:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_hostname;

    #[test]
    fn valid_hostnames_accepted() {
        assert!(validate_hostname("Antminer").is_ok());
        assert!(validate_hostname("miner-01.example.com").is_ok());
    }

    #[test]
    fn newline_injection_rejected() {
        assert!(validate_hostname("deck\ndhcp=true").is_err());
    }

    #[test]
    fn malformed_hostnames_rejected() {
        assert!(validate_hostname("").is_err());
        assert!(validate_hostname("host name").is_err());
        assert!(validate_hostname("-miner").is_err());
        assert!(validate_hostname("miner..lan").is_err());
        assert!(validate_hostname(&"a".repeat(64)).is_err());
        assert!(validate_hostname(&"a.".repeat(127)).is_err());
    }
}

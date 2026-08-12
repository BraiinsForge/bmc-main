// Copyright (C) 2022  Braiins Systems s.r.o.
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

//! Network-interface helpers and Wi-Fi drivers for the `bmc-net` crate set.
//!
//! [`NetworkInterface`] wraps interface enumeration (IP/MAC/gateway/network
//! lookups) on top of `pnet`/`get_if_addrs`. The [`wifi`] module defines the
//! [`WifiDriver`](wifi::WifiDriver) trait and its two backends — `nl80211`
//! (OpenWrt/`ubus`/`iwinfo`) and `esp32` (ESP32-over-SDIO via `esp32-sdio-cli`)
//! — selected per platform by the network manager.

use anyhow::{Context, Result, anyhow};
use bmc_net_types::MacAddr;
use get_if_addrs::IfAddr;
use log::{info, warn};
use pnet::datalink;
use pnet::datalink::{MacAddr as PNetMacAddr, NetworkInterface as PNetNetworkInterface};
use pnet::ipnetwork::IpNetwork;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Duration;
use tokio::net::UdpSocket;

pub mod wifi;

/// System hostname from procfs. Re-exported from `bmc-net-observe`, the sync
/// read-only observation crate, so all callers share one implementation.
pub use bmc_net_observe::hostname;

pub const WIRELESS_CONFIG_FILE_PATH: &str = "/etc/config/wireless";

/// Conventional name of the wired interface on every supported board. Shared so
/// the managers and their consumers agree on one spelling.
pub const DEFAULT_ETH_INTERFACE: &str = "eth0";

const RESOLV_CONF_PATH: &str = "/etc/resolv.conf";
/// Where dnsmasq publishes the real upstream resolvers when it fronts DNS on
/// localhost (so `/etc/resolv.conf` only lists 127.0.0.1).
const RESOLV_CONF_DNSMASQ_PATH: &str = "/tmp/resolv.conf.auto";
const PROC_NET_ROUTE_PATH: &str = "/proc/net/route";

/// First routable interface address (IPv4 or IPv6), via `getifaddrs`. Used as a
/// coarse fallback when a specific interface's IPv4 is unavailable.
///
/// Loopback, link-local (IPv6 `fe80::/10`, IPv4 APIPA `169.254.0.0/16`) and
/// unspecified addresses are skipped: they enumerate before a real address on
/// an interface that has not obtained a lease yet, and reporting one as "the
/// device IP" yields an unusable captive-portal redirect. This mirrors
/// `bmc_net_observe`'s `is_routable` predicate.
#[must_use]
pub fn first_non_loopback_ip() -> Option<IpAddr> {
    pick_routable_ip(get_if_addrs::get_if_addrs().ok()?)
}

/// First routable address from an interface list. Pure, for testing.
fn pick_routable_ip(interfaces: Vec<get_if_addrs::Interface>) -> Option<IpAddr> {
    interfaces.into_iter().find_map(|iface| {
        let ip: IpAddr = match iface.addr {
            IfAddr::V4(addr) => addr.ip.into(),
            IfAddr::V6(addr) => addr.ip.into(),
        };
        is_routable(ip).then_some(ip)
    })
}

/// True if `ip` is usable for connectivity (not loopback, link-local, or
/// unspecified).
fn is_routable(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => !v4.is_link_local(),
        // `Ipv6Addr::is_unicast_link_local` is still unstable; match `fe80::/10`
        // directly, as `bmc-net-observe` does for IPv4.
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

/// IPv4 `nameserver` entries parsed from a resolv.conf-format file at `path`.
/// Returns an empty vec if the file is missing or unreadable.
pub async fn nameservers(path: &str) -> Vec<Ipv4Addr> {
    let Ok(contents) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| line.strip_prefix("nameserver"))
        .filter_map(|rest| rest.trim().parse().ok())
        .collect()
}

/// Default resolv.conf path (`/etc/resolv.conf`).
#[must_use]
pub fn default_resolv_conf_path() -> &'static str {
    RESOLV_CONF_PATH
}

/// Effective upstream IPv4 nameservers.
///
/// Reads `/etc/resolv.conf`; if that lists only the local resolver
/// (`127.0.0.1`) — i.e. dnsmasq is fronting DNS — it falls back to the
/// dnsmasq-published upstream list at `/tmp/resolv.conf.auto`. This keeps the
/// "where do the real resolvers live behind dnsmasq" knowledge inside the
/// library rather than in every consumer.
pub async fn resolved_nameservers() -> Vec<Ipv4Addr> {
    let servers = nameservers(RESOLV_CONF_PATH).await;
    if servers == [Ipv4Addr::LOCALHOST] {
        let upstream = nameservers(RESOLV_CONF_DNSMASQ_PATH).await;
        if !upstream.is_empty() {
            return upstream;
        }
    }
    servers
}

/// IPv4 default gateway for `interface_name` (destination `0.0.0.0` in the main
/// table), read from `/proc/net/route`. `None` if there is no default route.
pub async fn default_gateway(interface_name: &str) -> Option<Ipv4Addr> {
    let contents = tokio::fs::read_to_string(PROC_NET_ROUTE_PATH).await.ok()?;
    contents.lines().skip(1).find_map(|line| {
        let mut fields = line.split_whitespace();
        let iface = fields.next()?;
        let destination = fields.next()?;
        let gateway = fields.next()?;
        if iface != interface_name || destination != "00000000" {
            return None;
        }
        let raw = u32::from_str_radix(gateway, 16).ok()?;
        // Gateway is little-endian hex; a zero gateway is not a real default route.
        (raw != 0).then(|| Ipv4Addr::from(raw.to_le_bytes()))
    })
}

fn get_primary_interface_details() -> Option<(IpAddr, PNetMacAddr)> {
    let iface = get_primary_interface()?;
    let network = iface.inner.ips.into_iter().find(IpNetwork::is_ipv4)?;
    let mac = iface.inner.mac?;
    Some((network.ip(), mac))
}

pub fn get_primary_interface() -> Option<NetworkInterface> {
    datalink::interfaces().into_iter().find_map(|interface| {
        let interface_clone = interface.clone();
        match (
            interface.is_running(),
            interface.is_loopback(),
            interface.ips.into_iter().find(IpNetwork::is_ipv4),
            interface.mac,
        ) {
            (true, false, Some(_), Some(_)) => Some(NetworkInterface {
                inner: interface_clone,
            }),
            _ => None,
        }
    })
}

pub async fn ip_report(format: impl AsRef<str>) -> Result<()> {
    // port that app is supposed to listen on
    const REPORTER_PORT: u16 = 14235;

    // port we bind to
    const MY_PORT: u16 = 14236;

    // how long to wait for reporter app reply
    const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

    let (ip, mac) =
        get_primary_interface_details().context("primary interface couldn't be determined")?;
    let ip = ip.to_string();
    let mac = mac.to_string();
    let hostname = hostname().context("hostname unavailable")?;
    let format = format.as_ref();

    info!("broadcasting IP address: {ip}, MAC address: {mac}, hostname: {hostname}");

    // original ip reporter app seems to be fine with handshake coming from different port,
    // we bind to same port mostly to use as poor man's exclusive lock
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MY_PORT)).await?;
    socket.set_broadcast(true)?;

    let message = format
        .replace("${IP}", &ip)
        .replace("${MAC}", &mac)
        .replace("${HOSTNAME}", &hostname);

    // first we broadcast message with our info
    socket
        .send_to(
            message.as_bytes(),
            SocketAddrV4::new(Ipv4Addr::BROADCAST, REPORTER_PORT),
        )
        .await?;

    // reporter app should respond by sending back mac...
    let mut recv_buf = [0; 64];
    // Using tokio::time::timeout, so we can handle timeout error
    match tokio::time::timeout(REPLY_TIMEOUT, socket.recv_from(&mut recv_buf)).await {
        Ok(Ok((n, remote_addr))) => {
            let response = &recv_buf[..n.min(recv_buf.len())];

            // ...which we then confirm
            if response != mac.as_bytes() {
                return Err(anyhow!("received incorrect response from {remote_addr}"));
            }

            // NOTE: unlike original implementation, we do not broadcast confirmation message
            // NOTE: original implementation adds some more bytes which are probably garbage
            //       just in case, captured data was: 'OK\x00\x00send_a'
            socket.send_to(b"OK\0", remote_addr).await?;
            info!("confirmed IP report to {remote_addr}");
        }
        Ok(Err(e)) => {
            warn!("error receiving response: {e}");
        }
        Err(_) => {
            // This message is shown to user if toolbox app is not running on local network
            warn!(
                "timeout for the IP report reply expired: No response received from the listening device."
            );
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
pub struct NetworkInterface {
    inner: PNetNetworkInterface,
}

impl NetworkInterface {
    #[must_use]
    pub fn get_by_name(intf_name: &str) -> Option<Self> {
        pnet::datalink::interfaces()
            .into_iter()
            .find(|ifce| ifce.name.eq_ignore_ascii_case(intf_name))
            .map(|network| Self { inner: network })
    }

    #[must_use]
    pub fn find_default() -> Option<Self> {
        pnet::datalink::interfaces()
            .into_iter()
            .find(|ifce| ifce.is_up() && !ifce.is_loopback() && !ifce.ips.is_empty())
            .map(|network| Self { inner: network })
    }

    #[must_use]
    pub fn mac_address(&self) -> Option<MacAddr> {
        self.inner
            .mac
            .map(|mac| PNetMacAddrWrapper::from(mac).into())
    }

    #[must_use]
    pub fn ipv4_address(&self) -> Option<IpAddr> {
        self.inner
            .ips
            .iter()
            .find(|ip| ip.is_ipv4())
            .map(IpNetwork::ip)
    }

    /// This interface's IPv4 address and MAC packaged as an [`IfaceData`].
    ///
    /// [`IfaceData`]: bmc_net_types::network::IfaceData
    #[must_use]
    pub fn iface_data(&self) -> bmc_net_types::network::IfaceData {
        bmc_net_types::network::IfaceData {
            ip: self.ipv4_address(),
            mac: self.mac_address(),
        }
    }

    #[must_use]
    pub fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[must_use]
    pub fn index(&self) -> u32 {
        self.inner.index
    }

    #[must_use]
    pub fn all_ips(&self) -> Vec<IpNetwork> {
        self.inner.ips.clone()
    }

    /// This interface's IPv4 addresses as [`bmc_net_types::network::IpNetwork`]
    /// (address + netmask); IPv6 addresses are skipped.
    #[must_use]
    pub fn ipv4_networks(&self) -> Vec<bmc_net_types::network::IpNetwork> {
        self.inner
            .ips
            .iter()
            .filter_map(|network| match network {
                IpNetwork::V4(network) => Some(bmc_net_types::network::IpNetwork {
                    address: network.ip(),
                    netmask: network.mask(),
                }),
                IpNetwork::V6(_) => None,
            })
            .collect()
    }

    #[must_use]
    pub fn get_by_substr(substring: &str) -> Option<Self> {
        pnet::datalink::interfaces()
            .into_iter()
            .find(|ifce| ifce.name.contains(substring))
            .map(|network| Self { inner: network })
    }
}

/// Internal adapter for converting `pnet`'s MAC type into [`MacAddr`] without
/// leaking the `pnet` type across the crate boundary.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct PNetMacAddrWrapper {
    inner: PNetMacAddr,
}

impl From<PNetMacAddr> for PNetMacAddrWrapper {
    fn from(mac: PNetMacAddr) -> Self {
        Self { inner: mac }
    }
}

impl From<PNetMacAddrWrapper> for MacAddr {
    fn from(value: PNetMacAddrWrapper) -> Self {
        Self::from([
            value.inner.0,
            value.inner.1,
            value.inner.2,
            value.inner.3,
            value.inner.4,
            value.inner.5,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_link_local_and_loopback_addresses() {
        use get_if_addrs::{Ifv4Addr, Ifv6Addr, Interface};

        let link_local_v6 = Interface {
            name: "wlan0".to_owned(),
            addr: IfAddr::V6(Ifv6Addr {
                ip: "fe80::1".parse().expect("BUG: bad test address"),
                netmask: "ffff:ffff:ffff:ffff::".parse().expect("BUG: bad test mask"),
                broadcast: None,
            }),
        };
        let apipa_v4 = Interface {
            name: "eth0".to_owned(),
            addr: IfAddr::V4(Ifv4Addr {
                ip: Ipv4Addr::new(169, 254, 1, 2),
                netmask: Ipv4Addr::new(255, 255, 0, 0),
                broadcast: None,
            }),
        };
        let routable = Interface {
            name: "eth1".to_owned(),
            addr: IfAddr::V4(Ifv4Addr {
                ip: Ipv4Addr::new(192, 168, 1, 10),
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                broadcast: None,
            }),
        };

        assert_eq!(
            pick_routable_ip(vec![link_local_v6, apipa_v4, routable]),
            Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)))
        );
    }

    #[test]
    fn none_when_every_address_is_unusable() {
        use get_if_addrs::{Ifv4Addr, Interface};

        let loopback = Interface {
            name: "lo".to_owned(),
            addr: IfAddr::V4(Ifv4Addr {
                ip: Ipv4Addr::LOCALHOST,
                netmask: Ipv4Addr::new(255, 0, 0, 0),
                broadcast: None,
            }),
        };
        assert_eq!(pick_routable_ip(vec![loopback]), None);
    }
}

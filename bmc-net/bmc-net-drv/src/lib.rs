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

use anyhow::{Context, Result, anyhow};
use ii_net::MacAddr;
use log::{info, warn};
use pnet::datalink;
use pnet::datalink::{MacAddr as PNetMacAddr, NetworkInterface as PNetNetworkInterface};
use pnet::ipnetwork::IpNetwork;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Duration;
use std::{fs, io};
use tokio::net::UdpSocket;

pub mod wifi;

pub const WIRELESS_CONFIG_FILE_PATH: &str = "/etc/config/wireless";

fn get_hostname() -> io::Result<String> {
    const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";
    fs::read_to_string(HOSTNAME_PATH).map(|x| x.trim().to_owned())
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
    let hostname = get_hostname()?;
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

    pub fn get_inner(&self) -> PNetNetworkInterface {
        self.inner.clone()
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

    #[must_use]
    pub fn get_by_substr(substring: &str) -> Option<Self> {
        pnet::datalink::interfaces()
            .into_iter()
            .find(|ifce| ifce.name.contains(substring))
            .map(|network| Self { inner: network })
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PNetMacAddrWrapper {
    inner: PNetMacAddr,
}

impl From<PNetMacAddr> for PNetMacAddrWrapper {
    fn from(mac: PNetMacAddr) -> Self {
        Self { inner: mac }
    }
}

impl From<PNetMacAddrWrapper> for MacAddr {
    fn from(value: PNetMacAddrWrapper) -> Self {
        Self::new(
            value.inner.0,
            value.inner.1,
            value.inner.2,
            value.inner.3,
            value.inner.4,
            value.inner.5,
        )
    }
}

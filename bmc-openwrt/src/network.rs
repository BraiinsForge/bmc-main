// Copyright (C) 2025  Braiins Systems s.r.o.

use pnet::datalink::{MacAddr as PNetMacAddr, NetworkInterface as PNetNetworkInterface};
use std::fmt::{Display, Formatter, Result};
use std::net::IpAddr;

pub struct NetworkInterface {
    inner: PNetNetworkInterface,
}

impl NetworkInterface {
    pub fn get_by_name(intf_name: &str) -> Option<Self> {
        pnet::datalink::interfaces()
            .into_iter()
            .find(|ifce| ifce.name.eq_ignore_ascii_case(intf_name))
            .map(|network| Self { inner: network })
    }

    pub fn mac_address(&self) -> Option<MacAddr> {
        self.inner
            .mac
            .map(|mac| PNetMacAddrWrapper::from(mac).into())
    }

    pub fn ipv4_address(&self) -> Option<IpAddr> {
        self.inner
            .ips
            .iter()
            .find(|ip| ip.is_ipv4())
            .map(pnet::ipnetwork::IpNetwork::ip)
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MacAddr {
    mac_addr: String,
}

impl MacAddr {
    #[expect(clippy::many_single_char_names)]
    pub fn new(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) -> Self {
        let mac_addr = format!("{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}",);
        Self { mac_addr }
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.mac_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_addr_string_format() {
        let mac_addr = MacAddr::new(0x01, 0x23, 0x45, 0x67, 0x89, 0xAB);

        assert_eq!(&mac_addr.to_string(), "01:23:45:67:89:ab");
    }
}

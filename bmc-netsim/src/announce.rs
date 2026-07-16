// Copyright (C) 2026  Braiins Systems s.r.o.

//! mDNS announcement: advertise a resource so widgets discover it.

use std::net::{Ipv4Addr, UdpSocket};

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::blueprint::AnnounceSpec;

/// Advertises resources on the local network.
pub trait Announcer {
    /// Advertise one resource under `name` on `port`.
    fn announce(&self, name: &str, port: u16, spec: &AnnounceSpec) -> Result<()>;
}

/// mDNS/DNS-SD announcer backed by a shared `mdns-sd` daemon.
pub struct MdnsAnnouncer {
    daemon: ServiceDaemon,
    /// The one LAN IPv4 to advertise.
    ///
    /// `enable_addr_auto` otherwise lists every interface (docker/veth/loopback),
    /// which bloats each `_http._tcp` record and leaks unreachable addresses
    /// — costly for a Deck browsing a busy type over lossy WiFi multicast.
    ///
    /// `None` (host offline) falls back to auto.
    lan_ip: Option<Ipv4Addr>,
}

impl std::fmt::Debug for MdnsAnnouncer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsAnnouncer").finish_non_exhaustive()
    }
}

impl MdnsAnnouncer {
    /// Start the background mDNS daemon.
    pub fn new() -> Result<Self> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            lan_ip: primary_lan_ipv4(),
        })
    }
}

/// The primary LAN IPv4 — the source address the OS would use to reach a public
/// host. No packet is sent; the connect only picks the default-route interface,
/// so it skips docker/veth/loopback. `None` when offline.
fn primary_lan_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("8.8.8.8", 80)).ok()?;
    match sock.local_addr().ok()? {
        std::net::SocketAddr::V4(addr) => Some(*addr.ip()),
        std::net::SocketAddr::V6(_) => None,
    }
}

impl Announcer for MdnsAnnouncer {
    fn announce(&self, name: &str, port: u16, spec: &AnnounceSpec) -> Result<()> {
        let AnnounceSpec::Mdns {
            service_type,
            subtype,
            txt,
        } = spec;
        // DNS-SD encodes a subtype in the service-type string as `<sub>._sub.<type>.<domain>`;
        // mdns-sd splits it back out on register, so a widget browsing `<sub>._sub.<type>` matches this service.
        let ty_domain = match subtype {
            Some(sub) => format!("{sub}._sub.{service_type}.local."),
            None => format!("{service_type}.local."),
        };
        let host_name = format!("{}.local.", dns_label(name));
        let props: Vec<(String, String)> =
            txt.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        // Advertise the one LAN address a Deck can reach, not every interface's
        // (see `lan_ip`); fall back to auto-detecting all when the host is offline.
        let info = match self.lan_ip {
            Some(ip) => {
                let ip = ip.to_string();
                ServiceInfo::new(&ty_domain, name, &host_name, ip.as_str(), port, &props[..])?
            }
            None => ServiceInfo::new(&ty_domain, name, &host_name, "", port, &props[..])?
                .enable_addr_auto(),
        };
        self.daemon.register(info)?;
        tracing::debug!(name = %name, ty = %ty_domain, port, "registered mDNS service");
        Ok(())
    }
}

/// Reduce an instance name to a DNS-safe hostname label. RFC 1035 forbids
/// hyphen-edged labels, so the edges are trimmed; an all-punctuation name
/// trims empty and falls back to `device`.
fn dns_label(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = mapped.trim_matches('-');
    if trimmed.is_empty() {
        "device".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::dns_label;

    #[test]
    fn maps_punctuation_and_trims_hyphen_edges() {
        assert_eq!(dns_label("bmm-01"), "bmm-01", "already valid, unchanged");
        assert_eq!(dns_label(".bmm."), "bmm", "edge punctuation trimmed");
        assert_eq!(dns_label("-x-"), "x", "hyphen edges trimmed");
        assert_eq!(dns_label("a.b_c"), "a-b-c", "inner punctuation kept as -");
        assert_eq!(dns_label("..."), "device", "all-punctuation falls back");
        assert_eq!(dns_label(""), "device", "empty falls back");
    }
}

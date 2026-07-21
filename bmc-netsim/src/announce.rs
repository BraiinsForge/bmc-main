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

//! mDNS announcement: advertise a resource so widgets discover it.

use std::net::{Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::blueprint::AnnounceSpec;

/// Advertises resources on the local network.
pub trait Announcer {
    /// Advertise one resource under `name` on `port`.
    fn announce(&self, name: &str, port: u16, spec: &AnnounceSpec) -> Result<()>;
}

/// Per-service offset within a re-announcement cycle, so the fleet
/// re-announces as a drizzle of small packets rather than one burst.
const REANNOUNCE_STAGGER: Duration = Duration::from_millis(100);

/// The re-announcement period of each service.
const REANNOUNCE_PERIOD: Duration = Duration::from_secs(50);

/// mDNS/DNS-SD announcer with one `mdns-sd` daemon per announced service.
///
/// A shared daemon answers a type query with all matching instances
/// in one aggregated response; at fleet scale that fragments,
/// and fragmented multicast rarely survives WiFi
/// (Deck 2026-07-18: a 90-device fleet was undiscoverable by query,
/// while per-service announcements got through).
/// Real devices are independent responders with small answers;
/// a daemon per service simulates exactly that.
/// Each service also re-announces periodically,
/// so a browser that missed a response still hears
/// each device's own small unsolicited announcements.
pub struct MdnsAnnouncer {
    /// Services announced so far — each one's re-announce schedule offset.
    /// `u32` for the `Duration` multiplier.
    announced: AtomicU32,
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
    /// Create the announcer.
    pub fn new() -> Result<Self> {
        Ok(Self {
            announced: AtomicU32::new(0),
            lan_ip: primary_lan_ipv4(),
        })
    }
}

/// Re-register the service each period, staggered by its ordinal;
/// `register` of an already-registered service re-announces it.
async fn reannounce(daemon: ServiceDaemon, info: ServiceInfo, ordinal: u32) {
    tokio::time::sleep(REANNOUNCE_PERIOD + REANNOUNCE_STAGGER * ordinal).await;
    let mut tick = tokio::time::interval(REANNOUNCE_PERIOD);
    loop {
        tick.tick().await;
        if let Err(err) = daemon.register(info.clone()) {
            tracing::warn!("mDNS re-announce of {} failed: {err}", info.get_fullname());
        }
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
        let daemon = ServiceDaemon::new()?;
        daemon.register(info.clone())?;
        let ordinal = self.announced.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(reannounce(daemon, info, ordinal));
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

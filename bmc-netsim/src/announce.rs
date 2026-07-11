// Copyright (C) 2026  Braiins Systems s.r.o.

//! mDNS announcement: advertise a resource so widgets discover it.

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
        })
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
        // Empty `ip` + `enable_addr_auto` advertises (and tracks)
        // the host's real interface addresses, so a Deck on the LAN can reach it.
        let info = ServiceInfo::new(&ty_domain, name, &host_name, "", port, &props[..])?
            .enable_addr_auto();
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

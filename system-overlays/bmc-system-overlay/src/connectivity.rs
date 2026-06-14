// Copyright (C) 2026  Braiins Systems s.r.o.

//! Synchronous, low-cadence connectivity probe shared by OS-driven overlays.
//!
//! "Online" means: at least one non-loopback interface holds a routable IPv4
//! address (link-local 169.254/16 excluded). The device is WiFi-centric and the
//! codebase exposes no separate ethernet carrier probe, so IPv4 presence is the
//! single signal for "neither WiFi nor ethernet connected".
//!
//! The startup IP overlay also needs the saved station SSID for display text.
//! It comes from OpenWrt's `uci` CLI (which normalizes quoting/comments), and is
//! intentionally observational: this helper does not start, retry, repair, or
//! reconfigure WiFi.

use std::net::Ipv4Addr;

use get_if_addrs::{IfAddr, Interface};

/// True if `ip` is usable for connectivity (not loopback, not link-local).
#[must_use]
fn is_routable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

/// Return the routable IPv4 for one interface if it has one.
#[must_use]
fn interface_ipv4(iface: &Interface) -> Option<Ipv4Addr> {
    match &iface.addr {
        IfAddr::V4(v4) if !iface.is_loopback() && is_routable(v4.ip) => Some(v4.ip),
        IfAddr::V4(_) | IfAddr::V6(_) => None,
    }
}

/// Pick the preferred routable IPv4 from an interface list. Pure, for testing.
///
/// Prefer WiFi station interfaces (kernel `wlan*` names; the trailing index is
/// not stable across boots/platforms, so match the prefix, not a fixed name).
/// Otherwise fall back to lexicographic interface-name order so the result is
/// deterministic and does not depend on raw `getifaddrs(3)` enumeration order.
#[must_use]
fn pick_ipv4(interfaces: &[Interface]) -> Option<Ipv4Addr> {
    let mut candidates: Vec<(&str, Ipv4Addr)> = interfaces
        .iter()
        .filter_map(|iface| interface_ipv4(iface).map(|ip| (iface.name.as_str(), ip)))
        .collect();
    // wlan* first (false sorts before true), then lexicographic within a group.
    candidates.sort_by_key(|(name, _)| (!name.starts_with("wlan"), *name));
    candidates.first().map(|(_, ip)| *ip)
}

/// The device's primary IPv4 address, or `None` when offline. This performs a
/// `getifaddrs(3)` walk; overlays must call it behind their own poll cache, not
/// once per host frame.
#[must_use]
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    pick_ipv4(&interfaces)
}

/// First enabled station-mode SSID from `uci show wireless` output. Pure, for
/// testing. The output is one `key=value` line per option, values single-quoted
/// and already comment-free; sections appear as `wireless.<id>=<type>`.
#[must_use]
fn station_ssid_from_uci_show(output: &str) -> Option<String> {
    #[derive(Default)]
    struct Section {
        mode: Option<String>,
        ssid: Option<String>,
        disabled: bool,
    }
    let mut sections: Vec<(String, Section)> = Vec::new();
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim_matches('\'');
        let mut parts = key.split('.');
        if parts.next() != Some("wireless") {
            continue;
        }
        let Some(id) = parts.next() else { continue };
        match parts.next() {
            None if value == "wifi-iface" => sections.push((id.to_owned(), Section::default())),
            None => {}
            Some(option) => {
                let Some((_, section)) = sections.last_mut().filter(|(sid, _)| sid == id) else {
                    continue;
                };
                match option {
                    "mode" => section.mode = Some(value.to_owned()),
                    "ssid" => section.ssid = Some(value.to_owned()),
                    "disabled" => section.disabled = matches!(value, "1" | "true" | "yes" | "on"),
                    _ => {}
                }
            }
        }
    }
    sections
        .into_iter()
        .filter(|(_, s)| s.mode.as_deref() == Some("sta") && !s.disabled)
        .find_map(|(_, s)| s.ssid.filter(|ssid| !ssid.is_empty()))
}

/// Saved station SSID via OpenWrt's `uci` (not by hand-parsing the config file):
/// run `uci -q show wireless` and select the first enabled station section.
/// Synchronous subprocess; safe for the startup overlay's low-cadence `tick`.
/// Observational only — never starts, retries, or reconfigures WiFi.
#[must_use]
pub fn configured_station_ssid() -> Option<String> {
    let output = std::process::Command::new("uci")
        .args(["-q", "show", "wireless"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    station_ssid_from_uci_show(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use get_if_addrs::Ifv4Addr;

    fn v4(name: &str, ip: Ipv4Addr) -> Interface {
        Interface {
            name: name.to_owned(),
            addr: IfAddr::V4(Ifv4Addr {
                ip,
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                broadcast: None,
            }),
        }
    }

    #[test]
    fn prefers_wifi_ipv4_before_ethernet_even_when_ethernet_is_first() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::LOCALHOST),
            v4("eth0", Ipv4Addr::new(192, 168, 1, 50)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(pick_ipv4(&ifaces), Some(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn falls_back_to_lexicographically_first_routable_interface() {
        let ifaces = vec![
            v4("zz0", Ipv4Addr::new(10, 0, 0, 9)),
            v4("aa0", Ipv4Addr::new(192, 168, 1, 50)),
        ];
        assert_eq!(pick_ipv4(&ifaces), Some(Ipv4Addr::new(192, 168, 1, 50)));
    }

    #[test]
    fn prefers_lowest_wlan_index_among_multiple() {
        let ifaces = vec![
            v4("wlan1", Ipv4Addr::new(10, 0, 0, 7)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(pick_ipv4(&ifaces), Some(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn none_when_only_loopback_and_link_local() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::LOCALHOST),
            v4("wlan0", Ipv4Addr::new(169, 254, 9, 9)),
        ];
        assert_eq!(pick_ipv4(&ifaces), None);
    }

    #[test]
    fn parses_enabled_station_ssid_from_uci_show() {
        let output = "\
wireless.radio0=wifi-device
wireless.radio0.type='mac80211'
wireless.ap=wifi-iface
wireless.ap.mode='ap'
wireless.ap.ssid='Deck setup'
wireless.sta=wifi-iface
wireless.sta.mode='sta'
wireless.sta.ssid='Office WiFi'
wireless.sta.disabled='0'
";
        assert_eq!(
            station_ssid_from_uci_show(output),
            Some("Office WiFi".to_owned())
        );
    }

    #[test]
    fn skips_disabled_station_in_uci_show() {
        let output = "\
wireless.old=wifi-iface
wireless.old.mode='sta'
wireless.old.disabled='1'
wireless.old.ssid='Old WiFi'
wireless.new=wifi-iface
wireless.new.mode='sta'
wireless.new.ssid='New WiFi'
";
        assert_eq!(
            station_ssid_from_uci_show(output),
            Some("New WiFi".to_owned())
        );
    }

    #[test]
    fn none_when_only_ap_mode_in_uci_show() {
        let output = "\
wireless.ap=wifi-iface
wireless.ap.mode='ap'
wireless.ap.ssid='Deck setup'
";
        assert_eq!(station_ssid_from_uci_show(output), None);
    }
}

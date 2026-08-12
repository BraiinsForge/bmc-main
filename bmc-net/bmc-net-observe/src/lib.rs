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

//! Synchronous, read-only network observation shared by OS-driven overlays and
//! diagnostics: hostname, primary routable IPv4, saved station SSID, and WiFi
//! signal. Intentionally observational — nothing here starts, retries, or
//! reconfigures networking, and it pulls no async runtime.
//!
//! [`probe`] does one pass (a `getifaddrs(3)` walk, one `uci -q show wireless`
//! spawn, one `/proc/net/wireless` read) and returns a [`Snapshot`]; callers
//! that want a single field can use [`hostname`], [`primary_ipv4`],
//! [`configured_station_ssid`], or [`wifi_signal_dbm`].

use std::collections::HashMap;
use std::net::Ipv4Addr;

use get_if_addrs::{IfAddr, Interface};

const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";
const PROC_NET_WIRELESS_PATH: &str = "/proc/net/wireless";

/// WiFi operating mode for a network interface.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum WifiMode {
    Ap,
    Station,
    #[default]
    Unknown,
}

impl WifiMode {
    /// Uplink preference for `pick_ipv4`: a station interface is the real
    /// uplink, an unknown interface (mode not reported by `uci`) is a maybe, and
    /// an AP interface (a coexisting setup AP) must never shadow the uplink.
    fn rank(self) -> u8 {
        match self {
            WifiMode::Station => 0,
            WifiMode::Unknown => 1,
            WifiMode::Ap => 2,
        }
    }
}

/// True if `ip` is usable for connectivity (not loopback, not link-local).
#[must_use]
fn is_routable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

/// Return the routable IPv4 for one interface if it has one.
#[must_use]
fn interface_ipv4(iface: &Interface) -> Option<Ipv4Addr> {
    match &iface.addr {
        IfAddr::V4(v4) if is_routable(v4.ip) => Some(v4.ip),
        IfAddr::V4(_) | IfAddr::V6(_) => None,
    }
}

/// Pick the preferred routable IPv4 from an interface list. Pure, for testing.
///
/// Prefer WiFi station interfaces, then unknown-mode ones, with AP-mode last so
/// a coexisting setup AP (used during reconfiguration) does not shadow the real
/// uplink (see `WifiMode::rank`). Within a mode rank, prefer kernel `wlan*`
/// names (the trailing index is not stable across boots/platforms, so match the
/// prefix, not a fixed name), then fall back to lexicographic interface-name
/// order so the result is deterministic and does not depend on raw
/// `getifaddrs(3)` enumeration order.
#[must_use]
fn pick_interface<'a>(
    interfaces: &'a [Interface],
    modes: &HashMap<String, WifiMode>,
) -> Option<&'a str> {
    let mut candidates: Vec<(&str, WifiMode)> = interfaces
        .iter()
        .filter_map(|iface| {
            interface_ipv4(iface)?;
            let mode = modes.get(&iface.name).copied().unwrap_or_default();
            Some((iface.name.as_str(), mode))
        })
        .collect();
    // Station, then unknown, then AP; wlan* before others, then lexicographic.
    candidates.sort_by_key(|(name, mode)| (mode.rank(), !name.starts_with("wlan"), *name));
    candidates.first().map(|(name, _)| *name)
}

/// Address of the interface [`pick_interface`] selects. Pure, for testing.
#[must_use]
fn pick_ipv4(interfaces: &[Interface], modes: &HashMap<String, WifiMode>) -> Option<Ipv4Addr> {
    let name = pick_interface(interfaces, modes)?;
    interfaces
        .iter()
        .find(|iface| iface.name == name)
        .and_then(interface_ipv4)
}

/// One `wifi-iface` section parsed from `uci show wireless` output.
struct WifiIfaceSection {
    ifname: Option<String>,
    mode: WifiMode,
    ssid: Option<String>,
    disabled: bool,
}

/// Parse all `wifi-iface` sections from `uci show wireless` output. Pure, for
/// testing. The output is one `key=value` line per option, values single-quoted
/// and comment-free; sections appear as `wireless.<id>=<type>`.
#[must_use]
fn wifi_iface_sections_from_uci_show(output: &str) -> Vec<WifiIfaceSection> {
    struct RawSection {
        id: String,
        section: WifiIfaceSection,
    }
    let mut sections: Vec<RawSection> = Vec::new();
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
            None if value == "wifi-iface" => sections.push(RawSection {
                id: id.to_owned(),
                section: WifiIfaceSection {
                    ifname: None,
                    mode: WifiMode::Unknown,
                    ssid: None,
                    disabled: false,
                },
            }),
            None => {}
            Some(option) => {
                let Some(raw) = sections.last_mut().filter(|r| r.id == id) else {
                    continue;
                };
                match option {
                    "ifname" => raw.section.ifname = Some(value.to_owned()),
                    "mode" => {
                        raw.section.mode = match value {
                            "ap" => WifiMode::Ap,
                            "sta" => WifiMode::Station,
                            _ => WifiMode::Unknown,
                        };
                    }
                    "ssid" => raw.section.ssid = Some(value.to_owned()),
                    "disabled" => {
                        raw.section.disabled = matches!(value, "1" | "true" | "yes" | "on");
                    }
                    _ => {}
                }
            }
        }
    }
    sections.into_iter().map(|r| r.section).collect()
}

/// First enabled station-mode SSID from parsed `wifi-iface` sections.
#[must_use]
fn station_ssid_from_sections(sections: &[WifiIfaceSection]) -> Option<String> {
    sections
        .iter()
        .filter(|s| s.mode == WifiMode::Station && !s.disabled)
        .find_map(|s| {
            s.ssid
                .as_deref()
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
}

/// Map each WiFi interface name to its UCI-configured mode from parsed sections.
#[must_use]
fn modes_map_from_sections(sections: &[WifiIfaceSection]) -> HashMap<String, WifiMode> {
    sections
        .iter()
        .filter_map(|s| s.ifname.as_ref().map(|name| (name.clone(), s.mode)))
        .collect()
}

/// Run `uci -q show wireless` once and return the parsed sections, or an empty
/// `Vec` on any error (missing binary, non-zero exit, non-UTF-8 output).
fn uci_show_wireless_sections() -> Vec<WifiIfaceSection> {
    let Ok(output) = std::process::Command::new("uci")
        .args(["-q", "show", "wireless"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };
    wifi_iface_sections_from_uci_show(&text)
}

/// WiFi signal level (dBm) of the first wireless interface in
/// `/proc/net/wireless` content. The "level" column may carry a trailing dot.
#[must_use]
fn wifi_signal_from_proc_net_wireless(content: &str) -> Option<i32> {
    for line in content.lines().skip(2) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 4 {
            let level = cols[3].trim_end_matches('.');
            if let Ok(value) = level.parse::<i32>() {
                return Some(value);
            }
        }
    }
    None
}

/// One probe pass's network readings. `ipv4: None` means genuinely offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Primary routable IPv4, or `None` when offline (see `pick_ipv4`).
    pub ipv4: Option<Ipv4Addr>,
    /// First enabled station-mode SSID from the saved UCI wireless config.
    pub station_ssid: Option<String>,
    /// Signal level of the first interface in `/proc/net/wireless`.
    pub wifi_signal_dbm: Option<i32>,
}

/// Assemble a snapshot from one pass's raw inputs. Pure, for testing.
#[must_use]
fn snapshot_from(
    interfaces: &[Interface],
    sections: &[WifiIfaceSection],
    proc_net_wireless: Option<&str>,
) -> Snapshot {
    let modes = modes_map_from_sections(sections);
    Snapshot {
        ipv4: pick_ipv4(interfaces, &modes),
        station_ssid: station_ssid_from_sections(sections),
        wifi_signal_dbm: proc_net_wireless.and_then(wifi_signal_from_proc_net_wireless),
    }
}

/// One blocking probe pass: a `getifaddrs(3)` walk, one `uci -q show wireless`
/// spawn, one `/proc/net/wireless` read. Can block for seconds while the kernel
/// holds rtnl, so run it off any latency-sensitive thread. `None` when the
/// interface walk itself errors (so a failed pass can leave the last-known
/// snapshot in place instead of masquerading as "offline").
#[must_use]
pub fn probe() -> Option<Snapshot> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    let sections = uci_show_wireless_sections();
    let proc_net_wireless = std::fs::read_to_string(PROC_NET_WIRELESS_PATH).ok();
    Some(snapshot_from(
        &interfaces,
        &sections,
        proc_net_wireless.as_deref(),
    ))
}

/// System hostname from procfs, trimmed. `None` if unreadable or empty.
#[must_use]
pub fn hostname() -> Option<String> {
    let raw = std::fs::read_to_string(HOSTNAME_PATH).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Name of the interface carrying the uplink, by the same ranking as
/// [`primary_ipv4`]. `None` when no interface has a routable address.
#[must_use]
pub fn primary_interface() -> Option<String> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    let modes = modes_map_from_sections(&uci_show_wireless_sections());
    pick_interface(&interfaces, &modes).map(ToOwned::to_owned)
}

/// Primary routable IPv4 (WiFi-station-preferred), or `None` when offline.
#[must_use]
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    let modes = modes_map_from_sections(&uci_show_wireless_sections());
    pick_ipv4(&interfaces, &modes)
}

/// First enabled station-mode SSID from the saved UCI wireless config.
#[must_use]
pub fn configured_station_ssid() -> Option<String> {
    station_ssid_from_sections(&uci_show_wireless_sections())
}

/// WiFi signal level (dBm) of the first interface in `/proc/net/wireless`.
#[must_use]
pub fn wifi_signal_dbm() -> Option<i32> {
    let content = std::fs::read_to_string(PROC_NET_WIRELESS_PATH).ok()?;
    wifi_signal_from_proc_net_wireless(&content)
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

    fn with_mode(iface: Interface, mode: WifiMode) -> (Interface, String, WifiMode) {
        let name = iface.name.clone();
        (iface, name, mode)
    }

    fn modes_map(entries: &[(Interface, String, WifiMode)]) -> HashMap<String, WifiMode> {
        entries
            .iter()
            .map(|(_, name, mode)| (name.clone(), *mode))
            .collect()
    }

    fn ifaces_from(entries: &[(Interface, String, WifiMode)]) -> Vec<Interface> {
        entries.iter().map(|(iface, _, _)| iface.clone()).collect()
    }

    /// The WiFi-only board shape: `eth0` is present as a link but holds no
    /// address, so the uplink must be resolved to `wlan0` rather than reported
    /// as absent.
    #[test]
    fn picks_the_wifi_interface_when_ethernet_has_no_address() {
        let interfaces = vec![v4("wlan0", Ipv4Addr::new(192, 168, 1, 106))];
        let modes = HashMap::from([("wlan0".to_owned(), WifiMode::Station)]);
        assert_eq!(pick_interface(&interfaces, &modes), Some("wlan0"));
        assert_eq!(
            pick_ipv4(&interfaces, &modes),
            Some(Ipv4Addr::new(192, 168, 1, 106))
        );
    }

    #[test]
    fn prefers_wifi_ipv4_before_ethernet_even_when_ethernet_is_first() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::LOCALHOST),
            v4("eth0", Ipv4Addr::new(192, 168, 1, 50)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(
            pick_ipv4(&ifaces, &HashMap::new()),
            Some(Ipv4Addr::new(10, 0, 0, 5))
        );
    }

    #[test]
    fn falls_back_to_lexicographically_first_routable_interface() {
        let ifaces = vec![
            v4("zz0", Ipv4Addr::new(10, 0, 0, 9)),
            v4("aa0", Ipv4Addr::new(192, 168, 1, 50)),
        ];
        assert_eq!(
            pick_ipv4(&ifaces, &HashMap::new()),
            Some(Ipv4Addr::new(192, 168, 1, 50))
        );
    }

    #[test]
    fn prefers_lowest_wlan_index_among_multiple() {
        let ifaces = vec![
            v4("wlan1", Ipv4Addr::new(10, 0, 0, 7)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(
            pick_ipv4(&ifaces, &HashMap::new()),
            Some(Ipv4Addr::new(10, 0, 0, 5))
        );
    }

    #[test]
    fn none_when_only_loopback_and_link_local() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::LOCALHOST),
            v4("wlan0", Ipv4Addr::new(169, 254, 9, 9)),
        ];
        assert_eq!(pick_ipv4(&ifaces, &HashMap::new()), None);
    }

    #[test]
    fn pick_ipv4_prefers_station_over_ap() {
        let entries = vec![
            with_mode(v4("wlan0", Ipv4Addr::new(192, 168, 1, 1)), WifiMode::Ap),
            with_mode(
                v4("wlan1", Ipv4Addr::new(10, 40, 20, 75)),
                WifiMode::Station,
            ),
        ];
        assert_eq!(
            pick_ipv4(&ifaces_from(&entries), &modes_map(&entries)),
            Some(Ipv4Addr::new(10, 40, 20, 75))
        );
    }

    #[test]
    fn pick_ipv4_prefers_station_over_unknown_despite_name_order() {
        // The station is the higher-named wlan1; the unknown-mode wlan0 would win
        // on lexicographic name order, so this fails unless mode rank dominates.
        let entries = vec![
            with_mode(
                v4("wlan0", Ipv4Addr::new(192, 168, 1, 1)),
                WifiMode::Unknown,
            ),
            with_mode(
                v4("wlan1", Ipv4Addr::new(10, 40, 20, 75)),
                WifiMode::Station,
            ),
        ];
        assert_eq!(
            pick_ipv4(&ifaces_from(&entries), &modes_map(&entries)),
            Some(Ipv4Addr::new(10, 40, 20, 75))
        );
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
        let sections = wifi_iface_sections_from_uci_show(output);
        assert_eq!(
            station_ssid_from_sections(&sections),
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
        let sections = wifi_iface_sections_from_uci_show(output);
        assert_eq!(
            station_ssid_from_sections(&sections),
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
        let sections = wifi_iface_sections_from_uci_show(output);
        assert_eq!(station_ssid_from_sections(&sections), None);
    }

    #[test]
    fn wifi_signal_parses_level_with_trailing_dot() {
        let content = "\
Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
 wlan0: 0000   70.  -52.  -256        0      0      0      0      0        0
";
        assert_eq!(wifi_signal_from_proc_net_wireless(content), Some(-52));
    }

    #[test]
    fn wifi_signal_none_without_interface_lines() {
        let content = "\
Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
";
        assert_eq!(wifi_signal_from_proc_net_wireless(content), None);
    }

    #[test]
    fn snapshot_from_assembles_all_values_in_one_pass() {
        let interfaces = vec![v4("wlan0", Ipv4Addr::new(10, 0, 0, 5))];
        let uci = "\
wireless.sta=wifi-iface
wireless.sta.ifname='wlan0'
wireless.sta.mode='sta'
wireless.sta.ssid='Office WiFi'
";
        let sections = wifi_iface_sections_from_uci_show(uci);
        let wireless = "\
header
header
 wlan0: 0000   70.  -52.  -256        0      0      0      0      0        0
";
        assert_eq!(
            snapshot_from(&interfaces, &sections, Some(wireless)),
            Snapshot {
                ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
                station_ssid: Some("Office WiFi".to_owned()),
                wifi_signal_dbm: Some(-52),
            }
        );
    }

    #[test]
    fn snapshot_from_is_all_none_when_offline_and_unconfigured() {
        assert_eq!(
            snapshot_from(&[], &[], None),
            Snapshot {
                ipv4: None,
                station_ssid: None,
                wifi_signal_dbm: None,
            }
        );
    }
}

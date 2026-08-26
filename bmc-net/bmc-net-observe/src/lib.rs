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
//! spawn, one `ubus call network.wireless status` spawn for the netdev-to-mode
//! mapping, a sysfs `carrier` read per wired candidate, one `/proc/net/wireless`
//! read) and returns a [`Snapshot`]; callers
//! that want a single field can use [`hostname`], [`primary_ipv4`],
//! [`configured_station_ssid`], or [`wifi_signal_dbm`].

use std::collections::HashMap;
use std::net::Ipv4Addr;

use get_if_addrs::{IfAddr, Interface};

const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";
const PROC_NET_WIRELESS_PATH: &str = "/proc/net/wireless";

/// WiFi operating mode for a network interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum WifiMode {
    Ap,
    Station,
    #[default]
    Unknown,
}

/// Setup-AP interfaces that never appear in `uci` wireless config. The ESP32 is
/// an external chip driven over its own firmware path, so its AP is reported as
/// `Unknown` and would otherwise outrank a real wireless uplink.
const KNOWN_AP_INTERFACES: &[&str] = &["ethap0"];

/// Uplink preference, best first. The order is the product decision: the
/// cable when it is plugged in, else the WiFi station, else whatever else has
/// an address, and a setup AP only when nothing else does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum UplinkRank {
    /// A non-`wlan*` interface with no wireless mode: assumed to be the cable.
    Wired,
    Station,
    /// A `wlan*` interface the wireless status could not classify.
    Unclassified,
    /// Any access point, so a setup AP never shadows a real uplink.
    Ap,
}

fn uplink_rank(name: &str, mode: WifiMode) -> UplinkRank {
    match mode {
        WifiMode::Ap => UplinkRank::Ap,
        // The ESP32 AP has no wireless section, so it is known by name.
        WifiMode::Unknown if KNOWN_AP_INTERFACES.contains(&name) => UplinkRank::Ap,
        WifiMode::Station => UplinkRank::Station,
        WifiMode::Unknown if !name.starts_with("wlan") => UplinkRank::Wired,
        WifiMode::Unknown => UplinkRank::Unclassified,
    }
}

/// Drop wired candidates whose link is down. A statically configured `eth0`
/// keeps its address after the cable is unplugged and would otherwise shadow a
/// working WiFi uplink. Only wired interfaces are judged this way: a WiFi
/// netdev's carrier follows association, which its address already reflects.
/// Pure, for testing.
fn drop_unplugged_wired(
    interfaces: Vec<Interface>,
    modes: &HashMap<String, WifiMode>,
    has_carrier: impl Fn(&str) -> bool,
) -> Vec<Interface> {
    interfaces
        .into_iter()
        .filter(|iface| {
            let mode = modes.get(&iface.name).copied().unwrap_or_default();
            uplink_rank(&iface.name, mode) != UplinkRank::Wired || has_carrier(&iface.name)
        })
        .collect()
}

/// Link state from `/sys/class/net/<name>/carrier`: `1` with link, `0`
/// without, and `EINVAL` while the interface is administratively down (no
/// uplink either). Any other failure means the answer is unknown, and an
/// unknown link is kept rather than dropped.
fn sysfs_carrier(name: &str) -> bool {
    match std::fs::read_to_string(format!("/sys/class/net/{name}/carrier")) {
        Ok(carrier) => carrier.trim() == "1",
        Err(e) => e.kind() != std::io::ErrorKind::InvalidInput,
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
/// Ranked wired first, then WiFi station, then unclassified, then AP last so a
/// coexisting setup AP does not shadow the real uplink. Ties break on the
/// interface name (matching the `wlan` prefix, whose trailing index is not
/// stable across boots) for a deterministic result independent of raw
/// `getifaddrs(3)` order.
#[must_use]
fn ranked_candidates<'a>(
    interfaces: &'a [Interface],
    modes: &HashMap<String, WifiMode>,
) -> Vec<(&'a str, WifiMode)> {
    let mut candidates: Vec<(&str, WifiMode)> = interfaces
        .iter()
        .filter_map(|iface| {
            interface_ipv4(iface)?;
            let mode = modes.get(&iface.name).copied().unwrap_or_default();
            Some((iface.name.as_str(), mode))
        })
        .collect();
    // Wired, then station, then unclassified, then AP; ties broken by name.
    candidates.sort_by_key(|(name, mode)| (uplink_rank(name, *mode), *name));
    candidates
}

#[must_use]
fn pick_interface<'a>(
    interfaces: &'a [Interface],
    modes: &HashMap<String, WifiMode>,
) -> Option<&'a str> {
    ranked_candidates(interfaces, modes)
        .first()
        .map(|(name, _)| *name)
}

/// Like [`pick_interface`] but never an AP (by mode, or the ESP32 AP by name):
/// the setup AP's own address is not an uplink, and a setup flow watching for
/// "the device got an IP" must not trigger on it. Unknown-mode interfaces stay
/// eligible, since the wireless status cannot classify a non-WiFi uplink and it
/// still counts.
#[must_use]
fn pick_station_interface<'a>(
    interfaces: &'a [Interface],
    modes: &HashMap<String, WifiMode>,
) -> Option<&'a str> {
    ranked_candidates(interfaces, modes)
        .into_iter()
        .find(|(name, mode)| uplink_rank(name, *mode) != UplinkRank::Ap)
        .map(|(name, _)| name)
}

/// Routable address of a named interface, if it has one.
#[must_use]
fn ipv4_of(interfaces: &[Interface], name: &str) -> Option<Ipv4Addr> {
    interfaces
        .iter()
        .find(|iface| iface.name == name)
        .and_then(interface_ipv4)
}

/// Address of the interface [`pick_interface`] selects. Pure, for testing.
#[must_use]
fn pick_ipv4(interfaces: &[Interface], modes: &HashMap<String, WifiMode>) -> Option<Ipv4Addr> {
    ipv4_of(interfaces, pick_interface(interfaces, modes)?)
}

/// Address of the interface [`pick_station_interface`] selects. Pure, for
/// testing.
#[must_use]
fn pick_station_ipv4(
    interfaces: &[Interface],
    modes: &HashMap<String, WifiMode>,
) -> Option<Ipv4Addr> {
    ipv4_of(interfaces, pick_station_interface(interfaces, modes)?)
}

/// One `wifi-iface` section parsed from `uci show wireless` output.
struct WifiIfaceSection {
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

/// Map each WiFi netdev to its mode, from `ubus call network.wireless status`.
///
/// The uci sections cannot answer this: `wifi-iface.ifname` is optional and
/// nothing writes it, so netifd names the netdev and only its runtime status
/// reports which name went with which mode.
///
/// Parsed as a `Value` rather than into a struct because the real output
/// repeats keys — a station interface carries `"mode"` twice — which a derived
/// `Deserialize` rejects outright. The same output carries the WiFi PSK, so it
/// is never logged, nor quoted into an error.
#[must_use]
fn modes_map_from_ubus(output: &str) -> HashMap<String, WifiMode> {
    let Ok(radios) = serde_json::from_str::<serde_json::Value>(output) else {
        tracing::warn!("wireless status is not valid JSON");
        return HashMap::new();
    };
    let mut modes = HashMap::new();
    for radio in radios.as_object().into_iter().flatten().map(|(_, v)| v) {
        let interfaces = radio
            .get("interfaces")
            .and_then(serde_json::Value::as_array);
        for iface in interfaces.into_iter().flatten() {
            let Some(name) = iface.get("ifname").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let mode = iface
                .get("config")
                .and_then(|config| config.get("mode"))
                .and_then(serde_json::Value::as_str);
            modes.insert(
                name.to_owned(),
                match mode {
                    Some("ap") => WifiMode::Ap,
                    Some("sta") => WifiMode::Station,
                    _ => WifiMode::Unknown,
                },
            );
        }
    }
    modes
}

/// Raw `ubus call network.wireless status` output, or `None` where there is no
/// ubus to ask — a host build, or a device whose wireless stack is not up.
/// Callers then see an empty mode map, which ranks every interface `Unknown`.
fn ubus_wireless_status() -> Option<String> {
    let output = std::process::Command::new("ubus")
        .args(["call", "network.wireless", "status"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
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
    /// Like `ipv4` but never the setup AP's own address, so a setup flow can
    /// watch for a real uplink (see `pick_station_ipv4`).
    pub station_ipv4: Option<Ipv4Addr>,
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
    wireless_status: Option<&str>,
    proc_net_wireless: Option<&str>,
) -> Snapshot {
    let modes = wireless_status.map(modes_map_from_ubus).unwrap_or_default();
    Snapshot {
        ipv4: pick_ipv4(interfaces, &modes),
        station_ipv4: pick_station_ipv4(interfaces, &modes),
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
    let wireless_status = ubus_wireless_status();
    let modes = wireless_status
        .as_deref()
        .map(modes_map_from_ubus)
        .unwrap_or_default();
    let interfaces = drop_unplugged_wired(interfaces, &modes, sysfs_carrier);
    let proc_net_wireless = std::fs::read_to_string(PROC_NET_WIRELESS_PATH).ok();
    Some(snapshot_from(
        &interfaces,
        &sections,
        wireless_status.as_deref(),
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
    let (interfaces, modes) = live_candidates()?;
    pick_interface(&interfaces, &modes).map(ToOwned::to_owned)
}

/// The addressed interfaces and their wireless modes as ranked in production:
/// one `getifaddrs(3)` walk, one `ubus` spawn, unplugged cables dropped.
fn live_candidates() -> Option<(Vec<Interface>, HashMap<String, WifiMode>)> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    let modes = ubus_wireless_status()
        .map(|status| modes_map_from_ubus(&status))
        .unwrap_or_default();
    let interfaces = drop_unplugged_wired(interfaces, &modes, sysfs_carrier);
    Some((interfaces, modes))
}

/// Primary routable IPv4 (cable first, then WiFi station, see `pick_interface`
/// for the full ranking), or `None` when offline.
#[must_use]
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let (interfaces, modes) = live_candidates()?;
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

    /// A real `ubus call network.wireless status` reply, secrets replaced.
    /// `mode` is repeated because the device repeats it; a derived
    /// `Deserialize` would reject that, so the parser must not use one.
    const UBUS_STATION: &str = r#"{
        "radio0": {
            "up": true,
            "config": { "channel": "1", "band": "2g" },
            "interfaces": [
                {
                    "section": "@wifi-iface[2]",
                    "ifname": "wlan0",
                    "config": {
                        "encryption": "psk2",
                        "key": "redacted",
                        "mode": "sta",
                        "ssid": "Office WiFi",
                        "disabled": false,
                        "mode": "sta",
                        "network": [ "wifi_sta" ]
                    },
                    "vlans": [],
                    "stations": []
                }
            ]
        }
    }"#;

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
    fn prefers_the_cable_when_both_ethernet_and_wifi_are_up() {
        let interfaces = vec![
            v4("eth0", Ipv4Addr::new(10, 33, 50, 103)),
            v4("wlan0", Ipv4Addr::new(192, 168, 1, 106)),
        ];
        let modes = HashMap::from([("wlan0".to_owned(), WifiMode::Station)]);
        assert_eq!(pick_interface(&interfaces, &modes), Some("eth0"));
    }

    #[test]
    fn an_unplugged_static_cable_does_not_shadow_the_wifi_uplink() {
        // A static `eth0` keeps its address with the cable out; only the
        // carrier tells, and only wired candidates are judged by it.
        let interfaces = vec![
            v4("eth0", Ipv4Addr::new(10, 33, 50, 103)),
            v4("wlan0", Ipv4Addr::new(192, 168, 1, 106)),
        ];
        let modes = HashMap::from([("wlan0".to_owned(), WifiMode::Station)]);
        let plugged = drop_unplugged_wired(interfaces.clone(), &modes, |_| true);
        assert_eq!(pick_interface(&plugged, &modes), Some("eth0"));
        let unplugged = drop_unplugged_wired(interfaces, &modes, |name| name != "eth0");
        assert_eq!(pick_interface(&unplugged, &modes), Some("wlan0"));
    }

    #[test]
    fn the_esp32_setup_ap_never_shadows_a_wifi_uplink() {
        // `ethap0` is the ESP32 setup AP. It has no `uci` wireless section, so it
        // arrives as `Unknown`; ranking wired-first without naming it explicitly
        // would let it outrank the station carrying the real uplink.
        let interfaces = vec![
            v4("ethap0", Ipv4Addr::new(10, 0, 0, 21)),
            v4("wlan0", Ipv4Addr::new(192, 168, 1, 106)),
        ];
        let modes = HashMap::from([("wlan0".to_owned(), WifiMode::Station)]);
        assert_eq!(pick_interface(&interfaces, &modes), Some("wlan0"));
    }

    #[test]
    fn the_setup_ap_is_used_when_it_is_the_only_address() {
        let interfaces = vec![v4("ethap0", Ipv4Addr::new(10, 0, 0, 21))];
        assert_eq!(pick_interface(&interfaces, &HashMap::new()), Some("ethap0"));
    }

    #[test]
    fn prefers_ethernet_ipv4_before_wifi_when_both_are_up() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::LOCALHOST),
            v4("eth0", Ipv4Addr::new(192, 168, 1, 50)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(
            pick_ipv4(&ifaces, &HashMap::new()),
            Some(Ipv4Addr::new(192, 168, 1, 50))
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
            snapshot_from(&interfaces, &sections, Some(UBUS_STATION), Some(wireless)),
            Snapshot {
                ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
                station_ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
                station_ssid: Some("Office WiFi".to_owned()),
                wifi_signal_dbm: Some(-52),
            }
        );
    }

    #[test]
    fn snapshot_from_is_all_none_when_offline_and_unconfigured() {
        assert_eq!(
            snapshot_from(&[], &[], None, None),
            Snapshot {
                ipv4: None,
                station_ipv4: None,
                station_ssid: None,
                wifi_signal_dbm: None,
            }
        );
    }

    #[test]
    fn ubus_status_maps_the_netdev_to_its_mode() {
        // The uci sections cannot: nothing writes `wifi-iface.ifname`, so a
        // map keyed on it comes back empty and every mode reads Unknown.
        let modes = modes_map_from_ubus(UBUS_STATION);
        assert_eq!(modes.get("wlan0"), Some(&WifiMode::Station));
    }

    #[test]
    fn a_station_address_is_none_while_only_the_setup_ap_holds_one() {
        // `ipv4` still reports it, so the offline chip stays hidden while the
        // AP serves clients; the AP's own address is simply not an uplink.
        let ubus = r#"{"radio0":{"interfaces":[{"ifname":"wlan0","config":{"mode":"ap"}}]}}"#;
        let modes = modes_map_from_ubus(ubus);
        let interfaces = vec![v4("wlan0", Ipv4Addr::new(192, 168, 8, 1))];
        assert_eq!(
            pick_ipv4(&interfaces, &modes),
            Some(Ipv4Addr::new(192, 168, 8, 1))
        );
        assert_eq!(pick_station_ipv4(&interfaces, &modes), None);
    }

    #[test]
    fn an_ap_interface_is_kept_out_of_the_station_address() {
        let ubus = r#"{"radio0":{"interfaces":[
            {"ifname":"wlan0","config":{"mode":"ap"}},
            {"ifname":"wlan1","config":{"mode":"sta"}}
        ]}}"#;
        let modes = modes_map_from_ubus(ubus);
        let interfaces = vec![
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 21)),
            v4("wlan1", Ipv4Addr::new(192, 168, 1, 5)),
        ];
        assert_eq!(
            pick_station_ipv4(&interfaces, &modes),
            Some(Ipv4Addr::new(192, 168, 1, 5))
        );
    }

    #[test]
    fn the_esp32_setup_ap_is_never_the_station_address() {
        // `ethap0` has no wireless section and no mode, so only its name says
        // it is the setup AP; `station_ipv4` must skip it like `ipv4` does.
        let interfaces = vec![v4("ethap0", Ipv4Addr::new(10, 0, 0, 21))];
        assert_eq!(pick_station_ipv4(&interfaces, &HashMap::new()), None);
        let interfaces = vec![
            v4("ethap0", Ipv4Addr::new(10, 0, 0, 21)),
            v4("wlan0", Ipv4Addr::new(192, 168, 1, 106)),
        ];
        let modes = HashMap::from([("wlan0".to_owned(), WifiMode::Station)]);
        assert_eq!(
            pick_station_ipv4(&interfaces, &modes),
            Some(Ipv4Addr::new(192, 168, 1, 106))
        );
    }

    #[test]
    fn an_unknown_mode_interface_still_counts_as_a_station_address() {
        // `uci`/`ubus` cannot classify a non-WiFi uplink, and it is still one.
        let ubus = r#"{"radio0":{"interfaces":[{"ifname":"wlan0","config":{"mode":"ap"}}]}}"#;
        let modes = modes_map_from_ubus(ubus);
        let interfaces = vec![
            v4("wlan0", Ipv4Addr::new(192, 168, 8, 1)),
            v4("eth0", Ipv4Addr::new(10, 0, 0, 7)),
        ];
        assert_eq!(
            pick_station_ipv4(&interfaces, &modes),
            Some(Ipv4Addr::new(10, 0, 0, 7))
        );
    }

    #[test]
    fn unparsable_wireless_status_leaves_every_mode_unknown() {
        assert!(modes_map_from_ubus("not json").is_empty());
    }
}

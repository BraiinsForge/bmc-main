// Copyright (C) 2026  Braiins Systems s.r.o.

//! Background connectivity prober shared by OS-driven overlays.
//!
//! "Online" means: at least one non-loopback interface holds a routable IPv4
//! address (link-local 169.254/16 excluded). The device is WiFi-centric and the
//! codebase exposes no separate ethernet carrier probe, so IPv4 presence is the
//! single signal for "neither WiFi nor ethernet connected".
//!
//! A detached thread probes once per second — one `getifaddrs(3)` walk, one
//! `uci -q show wireless` spawn (which normalizes quoting/comments), one
//! `/proc/net/wireless` read — and publishes a [`Snapshot`]. Overlay ticks
//! read it via [`snapshot_if_changed()`] and never block on the kernel's rtnl
//! lock. The probe is intentionally observational: it does not start, retry,
//! repair, or reconfigure WiFi.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::time::Duration;

use get_if_addrs::{IfAddr, Interface};

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
fn pick_ipv4(interfaces: &[Interface], modes: &HashMap<String, WifiMode>) -> Option<Ipv4Addr> {
    let mut candidates: Vec<(&str, WifiMode, Ipv4Addr)> = interfaces
        .iter()
        .filter_map(|iface| {
            let ip = interface_ipv4(iface)?;
            let mode = modes.get(&iface.name).copied().unwrap_or_default();
            Some((iface.name.as_str(), mode, ip))
        })
        .collect();
    // Station, then unknown, then AP; wlan* before others, then lexicographic.
    candidates.sort_by_key(|(name, mode, _)| (mode.rank(), !name.starts_with("wlan"), *name));
    candidates.first().map(|(_, _, ip)| *ip)
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

/// One probe pass's network readings. `ipv4: None` means genuinely offline;
/// "not yet probed" is [`snapshot_if_changed()`] returning `None` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Primary routable IPv4, or `None` when offline (see `pick_ipv4`).
    pub ipv4: Option<Ipv4Addr>,
    /// First enabled station-mode SSID from the saved UCI wireless config.
    pub station_ssid: Option<String>,
    /// Signal level of the first interface in `/proc/net/wireless`.
    pub wifi_signal_dbm: Option<i32>,
}

/// Opaque change marker of a published [`Snapshot`]. Returned by
/// [`snapshot_if_changed`] and handed back as `seen` on the next poll;
/// "different = changed" is the only semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotVersion(NonZeroU64);

impl SnapshotVersion {
    /// Version of the prober's first publish; a fixed point for test doubles
    /// that fake a single published snapshot.
    pub const FIRST: Self = Self(NonZeroU64::MIN);
}

/// A published [`Snapshot`] paired with its change marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedSnapshot {
    /// Change marker to hand back as `seen` on the next poll.
    pub version: SnapshotVersion,
    /// The published readings.
    pub snapshot: Snapshot,
}

/// Publisher/reader pair between the prober thread and overlay ticks. The
/// mutex is held only to swap or clone the value, never across a probe.
///
/// The raw `version` counter is 0 until the first publish (readers see that
/// as "no version yet") and bumps only when the published content differs
/// from the previous snapshot. It is incremented exclusively while the mutex
/// is held, so a (version, snapshot) pair read under the lock is always
/// consistent; the lock-free load in [`Self::read_if_changed`] is only a
/// cheap "anything new?" gate.
#[derive(Default)]
struct ProbeState {
    version: AtomicU64,
    snapshot: Mutex<Option<Snapshot>>,
}

impl ProbeState {
    fn publish(&self, snapshot: Snapshot) {
        let mut guard = self.lock();
        if guard.as_ref() != Some(&snapshot) {
            self.version.fetch_add(1, Ordering::Relaxed);
            *guard = Some(snapshot);
        }
    }

    /// Latest snapshot with its version, or `None` when the version still
    /// equals `seen` (or nothing has been published yet). The unchanged case
    /// is one atomic load — no lock, no allocation — so this is safe to poll
    /// per frame.
    fn read_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
        let seen = seen.map_or(0, |v| v.0.get());
        // Relaxed suffices: this is only a gate. A reader that passes it
        // synchronizes through the mutex acquire below and re-reads the
        // version there; one that races a bump just catches up next poll.
        if self.version.load(Ordering::Relaxed) == seen {
            return None;
        }
        let guard = self.lock();
        let version = NonZeroU64::new(self.version.load(Ordering::Relaxed)).map(SnapshotVersion)?;
        guard
            .clone()
            .map(|snapshot| VersionedSnapshot { version, snapshot })
    }

    fn lock(&self) -> MutexGuard<'_, Option<Snapshot>> {
        // A panic can only poison a plain value swap or clone, so the inner
        // value is always intact; recover it instead of propagating.
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Assemble a snapshot from one pass's raw inputs. Pure, for testing. This is
/// the spec's "injectable probe cycle": the testable unit is this pure
/// assembly over raw inputs; `probe_once` stays a thin I/O shim.
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

/// One blocking probe pass: a `getifaddrs(3)` walk, one `uci -q show
/// wireless` spawn, one `/proc/net/wireless` read. Runs only on the prober
/// thread — it can block for seconds while the kernel holds rtnl. `None` when
/// the interface walk itself errors, so a failed probe leaves the last-known
/// snapshot in place instead of masquerading as "offline".
fn probe_once() -> Option<Snapshot> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    let sections = uci_show_wireless_sections();
    let proc_net_wireless = std::fs::read_to_string("/proc/net/wireless").ok();
    Some(snapshot_from(
        &interfaces,
        &sections,
        proc_net_wireless.as_deref(),
    ))
}

/// Pause between probe passes.
const PROBE_PERIOD: Duration = Duration::from_secs(1);

/// Spawn the detached prober thread. The probe path only walks getifaddrs,
/// spawns one subprocess, and parses small strings; 128 KiB of stack is
/// plenty, and the 2 MiB Rust default would waste address space on 32-bit
/// ARM. On spawn failure the snapshot stays `None` forever, which readers
/// treat as "never probed".
fn spawn_prober(state: &'static ProbeState) {
    let spawned = std::thread::Builder::new()
        .name("connectivity-prober".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            loop {
                // AssertUnwindSafe: the pass only produces a value; ProbeState
                // recovers from poisoning, so no broken state can leak.
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe_once)) {
                    Ok(Some(snapshot)) => state.publish(snapshot),
                    Ok(None) => tracing::warn!("connectivity probe pass could not read interfaces"),
                    Err(_) => tracing::error!("connectivity probe pass panicked"),
                }
                std::thread::sleep(PROBE_PERIOD);
            }
        });
    if let Err(err) = spawned {
        tracing::error!("failed to spawn connectivity prober thread: {err}");
    }
}

/// Shared prober state; the first access spawns the prober thread.
fn prober_state() -> &'static ProbeState {
    static STATE: ProbeState = ProbeState {
        version: AtomicU64::new(0),
        snapshot: Mutex::new(None),
    };
    static SPAWN: Once = Once::new();
    SPAWN.call_once(|| spawn_prober(&STATE));
    &STATE
}

/// Latest connectivity snapshot and its version, or `None` while the content
/// has not changed since `seen` (pass `None` initially, then the last
/// returned version — `None` also covers "prober has not published yet",
/// possibly forever if its thread failed to spawn). The unchanged case does
/// no allocation, so this is safe to poll on a per-frame animation tick.
/// Spawns the prober on first call; never blocks beyond a value-swap mutex.
#[must_use]
pub fn snapshot_if_changed(seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
    prober_state().read_if_changed(seen)
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
    fn probe_state_read_returns_latest_publish() {
        let state = ProbeState::default();
        state.publish(Snapshot {
            ipv4: None,
            station_ssid: None,
            wifi_signal_dbm: None,
        });
        state.publish(Snapshot {
            ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
            station_ssid: Some("Office WiFi".to_owned()),
            wifi_signal_dbm: Some(-52),
        });
        assert_eq!(
            state.read_if_changed(None).map(|update| update.snapshot),
            Some(Snapshot {
                ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
                station_ssid: Some("Office WiFi".to_owned()),
                wifi_signal_dbm: Some(-52),
            })
        );
    }

    #[test]
    fn read_if_changed_returns_none_until_first_publish() {
        let state = ProbeState::default();
        assert_eq!(state.read_if_changed(None), None);
    }

    // The tray polls on a ~30 Hz animation tick; the version gate is what lets
    // those ticks skip the snapshot clone, so an unchanged re-publish (the
    // prober re-reads every second) must not look like a change.
    #[test]
    fn identical_republish_does_not_bump_version() {
        let state = ProbeState::default();
        let snapshot = Snapshot {
            ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
            station_ssid: Some("Office WiFi".to_owned()),
            wifi_signal_dbm: Some(-52),
        };
        state.publish(snapshot.clone());
        let first = state
            .read_if_changed(None)
            .expect("BUG: first publish must be visible");
        assert_eq!(first.snapshot, snapshot);

        state.publish(snapshot.clone());
        assert_eq!(state.read_if_changed(Some(first.version)), None);
    }

    #[test]
    fn changed_publish_bumps_version_and_returns_new_content() {
        let state = ProbeState::default();
        let offline = Snapshot {
            ipv4: None,
            station_ssid: None,
            wifi_signal_dbm: None,
        };
        let online = Snapshot {
            ipv4: Some(Ipv4Addr::new(10, 0, 0, 5)),
            station_ssid: Some("Office WiFi".to_owned()),
            wifi_signal_dbm: Some(-52),
        };
        state.publish(offline);
        let first = state
            .read_if_changed(None)
            .expect("BUG: first publish must be visible");

        state.publish(online.clone());
        assert_eq!(
            state
                .read_if_changed(Some(first.version))
                .map(|update| update.snapshot),
            Some(online)
        );
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

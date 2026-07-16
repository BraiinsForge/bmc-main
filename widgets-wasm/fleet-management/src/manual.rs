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

use crate::device::{DeviceFamily, DeviceId, DeviceIdentity, DeviceList, DeviceSource};

/// Turn one operator-entered `host` or `host:port` entry into a manual device
/// identity, or `None` if the entry is empty or carries an unusable port.
pub(crate) fn manual_identity(
    family: DeviceFamily,
    default_port: u16,
    entry: &str,
) -> Option<DeviceIdentity> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (host, port) = match trimmed.rsplit_once(':') {
        Some((host, suffix)) => {
            let port = suffix.parse::<u16>().ok().filter(|p| *p != 0)?;
            (host, port)
        }
        None => (trimmed, default_port),
    };
    if host.is_empty() {
        return None;
    }
    // A colon left in `host` is an ambiguous bare IPv6 literal: `fe80::1` would
    // mis-split into host `fe80:` / port `1` and resolve to a wrong endpoint.
    // Require bracket notation (`[fe80::1]:80`) rather than accepting that.
    if host.contains(':') && !host.starts_with('[') {
        return None;
    }
    Some(DeviceIdentity {
        id: DeviceId::for_manual(family, trimmed),
        family,
        name: host.to_owned(),
        host: host.to_owned(),
        port,
        source: DeviceSource::Manual,
    })
}

/// Resolve operator-entered host `entries` into the desired manual identities
/// for a family. Returns `None` when entries were supplied but none parsed into
/// a usable identity — the caller should then leave the existing manual set
/// unchanged (mirroring the invalid-JSON guard) rather than clear it on a typo
/// like `["10.0.0.5:abc"]`. An empty `entries` slice returns `Some(empty)`: the
/// explicit clear.
pub(crate) fn desired_identities(
    family: DeviceFamily,
    default_port: u16,
    entries: &[String],
) -> Option<Vec<DeviceIdentity>> {
    if entries.is_empty() {
        return Some(Vec::new());
    }
    let desired: Vec<DeviceIdentity> = entries
        .iter()
        .filter_map(|entry| manual_identity(family, default_port, entry))
        .collect();
    if desired.is_empty() {
        None
    } else {
        Some(desired)
    }
}

/// Outcome of reconciling one family's manual set: the ids dropped (so their
/// cached session tokens can be cleared) and whether any device was added (so
/// polling can be (re)started).
pub(crate) struct ManualReconcile {
    pub(crate) removed_ids: Vec<DeviceId>,
    pub(crate) added_any: bool,
}

/// Reconcile `list`'s manual devices for `family` to exactly `desired`:
/// upsert every desired identity and remove every manual id no longer wanted.
/// Discovered devices are never read or mutated.
pub(crate) fn reconcile_manual_into(
    list: &mut DeviceList,
    family: DeviceFamily,
    desired: Vec<DeviceIdentity>,
) -> ManualReconcile {
    let current = list.manual_ids_for_family(family);
    let desired_ids: Vec<DeviceId> = desired.iter().map(|d| d.id.clone()).collect();
    let mut added_any = false;
    for identity in desired {
        if list.upsert(identity) {
            added_any = true;
        }
    }
    let removed_ids: Vec<DeviceId> = current
        .into_iter()
        .filter(|id| !desired_ids.contains(id))
        .collect();
    for id in &removed_ids {
        list.remove(id);
    }
    ManualReconcile {
        removed_ids,
        added_any,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bos_manual(entry: &str) -> Option<DeviceIdentity> {
        manual_identity(DeviceFamily::Bos, 80, entry)
    }

    #[test]
    fn bare_host_uses_default_port() {
        let id = bos_manual("10.0.0.5").expect("BUG: a bare host must be accepted");
        assert_eq!(id.id.as_str(), "bos/manual/10.0.0.5");
        assert_eq!(id.name, "10.0.0.5");
        assert_eq!(id.host, "10.0.0.5");
        assert_eq!(id.port, 80);
        assert_eq!(id.source, DeviceSource::Manual);
    }

    #[test]
    fn host_port_parses_the_port_and_strips_it_from_name() {
        let id = bos_manual("miner.local:8080").expect("BUG: a host:port entry must be accepted");
        assert_eq!(id.id.as_str(), "bos/manual/miner.local:8080");
        assert_eq!(id.name, "miner.local");
        assert_eq!(id.host, "miner.local");
        assert_eq!(id.port, 8080);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let id = bos_manual("  10.0.0.5  ")
            .expect("BUG: surrounding whitespace must be trimmed and accepted");
        assert_eq!(id.id.as_str(), "bos/manual/10.0.0.5");
    }

    #[test]
    fn rejects_unusable_entries() {
        for bad in [
            "",
            "   ",
            "host:0",
            "host:99999",
            "host:abc",
            "host:",
            ":8080",
            "fe80::1",
        ] {
            assert!(bos_manual(bad).is_none(), "must reject {bad:?}");
        }
    }

    #[test]
    fn accepts_bracketed_ipv6_with_port() {
        let id = bos_manual("[fe80::1]:8080")
            .expect("BUG: a bracketed IPv6 literal with a port must be accepted");
        assert_eq!(id.host, "[fe80::1]");
        assert_eq!(id.port, 8080);
    }

    #[test]
    fn accepts_hostname_and_ipv4_with_default_port() {
        assert_eq!(
            bos_manual("miner.local")
                .expect("BUG: a bare hostname must be accepted")
                .port,
            80
        );
        assert_eq!(
            bos_manual("192.168.1.7")
                .expect("BUG: a bare IPv4 must be accepted")
                .port,
            80
        );
    }

    fn manual_id(entry: &str) -> DeviceIdentity {
        bos_manual(entry).expect("BUG: a valid manual entry must parse")
    }

    fn discovered(name: &str) -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::for_family(DeviceFamily::Bos, name),
            family: DeviceFamily::Bos,
            name: name.to_owned(),
            host: "10.0.0.9".to_owned(),
            port: 80,
            source: DeviceSource::Discovered,
        }
    }

    fn bos_desired(entries: &[&str]) -> Option<Vec<DeviceIdentity>> {
        let owned: Vec<String> = entries.iter().map(|e| (*e).to_owned()).collect();
        desired_identities(DeviceFamily::Bos, 80, &owned)
    }

    #[test]
    fn desired_identities_empty_is_explicit_clear() {
        let out = bos_desired(&[]).expect("BUG: an empty list is the explicit clear, not a keep");
        assert!(out.is_empty());
    }

    #[test]
    fn desired_identities_all_invalid_keeps_previous() {
        assert!(
            bos_desired(&["10.0.0.5:abc", ""]).is_none(),
            "entries that all fail to parse must keep the previous set, not clear it"
        );
    }

    #[test]
    fn desired_identities_mixed_drops_only_the_invalid() {
        let out = bos_desired(&["10.0.0.5:abc", "10.0.0.6"])
            .expect("BUG: a list with at least one valid entry must reconcile");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id.as_str(), "bos/manual/10.0.0.6");
    }

    #[test]
    fn adds_new_manual_devices() {
        let mut list = DeviceList::new();
        let out = reconcile_manual_into(
            &mut list,
            DeviceFamily::Bos,
            vec![manual_id("10.0.0.5"), manual_id("10.0.0.6")],
        );
        assert!(out.added_any);
        assert!(out.removed_ids.is_empty());
        assert_eq!(list.manual_ids_for_family(DeviceFamily::Bos).len(), 2);
    }

    #[test]
    fn removes_manual_devices_absent_from_desired() {
        let mut list = DeviceList::new();
        reconcile_manual_into(
            &mut list,
            DeviceFamily::Bos,
            vec![manual_id("10.0.0.5"), manual_id("10.0.0.6")],
        );
        let out = reconcile_manual_into(&mut list, DeviceFamily::Bos, vec![manual_id("10.0.0.5")]);
        assert!(!out.added_any);
        assert_eq!(out.removed_ids.len(), 1);
        assert_eq!(out.removed_ids[0].as_str(), "bos/manual/10.0.0.6");
    }

    #[test]
    fn empty_desired_clears_all_manual_devices() {
        let mut list = DeviceList::new();
        reconcile_manual_into(
            &mut list,
            DeviceFamily::Bos,
            vec![manual_id("10.0.0.5"), manual_id("10.0.0.6")],
        );
        let out = reconcile_manual_into(&mut list, DeviceFamily::Bos, Vec::new());
        assert_eq!(out.removed_ids.len(), 2);
        assert!(list.manual_ids_for_family(DeviceFamily::Bos).is_empty());
    }

    #[test]
    fn never_touches_discovered_devices() {
        let mut list = DeviceList::new();
        list.upsert(discovered("disc._http._tcp.local."));
        let out = reconcile_manual_into(&mut list, DeviceFamily::Bos, Vec::new());
        assert!(out.removed_ids.is_empty());
        assert_eq!(list.len(), 1, "discovered device must survive");
    }

    #[test]
    fn editing_a_port_replaces_the_device() {
        let mut list = DeviceList::new();
        reconcile_manual_into(&mut list, DeviceFamily::Bos, vec![manual_id("10.0.0.5")]);
        let out = reconcile_manual_into(
            &mut list,
            DeviceFamily::Bos,
            vec![manual_id("10.0.0.5:8080")],
        );
        assert!(out.added_any);
        assert_eq!(out.removed_ids.len(), 1);
        assert_eq!(out.removed_ids[0].as_str(), "bos/manual/10.0.0.5");
        let ids = list.manual_ids_for_family(DeviceFamily::Bos);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].as_str(), "bos/manual/10.0.0.5:8080");
    }

    #[test]
    fn duplicate_desired_entry_upserts_once() {
        let mut list = DeviceList::new();
        let out = reconcile_manual_into(
            &mut list,
            DeviceFamily::Bos,
            vec![manual_id("10.0.0.5"), manual_id("10.0.0.5")],
        );
        assert!(out.added_any);
        assert!(out.removed_ids.is_empty());
        assert_eq!(list.manual_ids_for_family(DeviceFamily::Bos).len(), 1);
    }
}

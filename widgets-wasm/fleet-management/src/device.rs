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

use crate::model::MinerModel;
use crate::telemetry::{TelemetryReading, TelemetrySnapshot};

use core::ops::{Index, IndexMut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFamily {
    Bos,
    Ubos,
    Bitaxe,
}

impl DeviceFamily {
    /// Every family, in storage order. The single source of truth that
    /// [`FamilyMap`] is sized and indexed against.
    pub const ALL: [DeviceFamily; 3] =
        [DeviceFamily::Bos, DeviceFamily::Ubos, DeviceFamily::Bitaxe];

    /// Number of families; the length of every [`FamilyMap`].
    pub const COUNT: usize = Self::ALL.len();

    /// Position of this family in [`ALL`](Self::ALL); the index into a
    /// [`FamilyMap`].
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            DeviceFamily::Bos => 0,
            DeviceFamily::Ubos => 1,
            DeviceFamily::Bitaxe => 2,
        }
    }
}

/// A total map from every [`DeviceFamily`] to a `T`, indexed by
/// [`DeviceFamily::index`]. The backing array's length is tied to
/// [`DeviceFamily::COUNT`], so adding a family is a compile error until every
/// map is widened — unlike a bare `[T; 3]` paired with a hand-written index,
/// where a mismatch silently corrupts state at runtime.
pub struct FamilyMap<T>([T; DeviceFamily::COUNT]);

impl<T> FamilyMap<T> {
    pub fn from_fn(mut f: impl FnMut(DeviceFamily) -> T) -> Self {
        Self(core::array::from_fn(|i| f(DeviceFamily::ALL[i])))
    }
}

impl<T> Index<DeviceFamily> for FamilyMap<T> {
    type Output = T;
    fn index(&self, family: DeviceFamily) -> &T {
        &self.0[family.index()]
    }
}

impl<T> IndexMut<DeviceFamily> for FamilyMap<T> {
    fn index_mut(&mut self, family: DeviceFamily) -> &mut T {
        &mut self.0[family.index()]
    }
}

#[must_use]
pub fn family_label(family: DeviceFamily) -> &'static str {
    match family {
        DeviceFamily::Bos => "BOS",
        DeviceFamily::Ubos => "uBOS",
        DeviceFamily::Bitaxe => "Bitaxe",
    }
}

/// Stable, lowercase slug for the family, distinct from the display label.
#[must_use]
pub fn family_id(family: DeviceFamily) -> &'static str {
    match family {
        DeviceFamily::Bos => "bos",
        DeviceFamily::Ubos => "ubos",
        DeviceFamily::Bitaxe => "bitaxe",
    }
}

/// The manifest credential param keys for a family, empty for credential-less
/// families (Bitaxe/AxeOS). The one place the family↔credential-key mapping
/// lives, used to scope a token/telemetry reset to the family whose credentials
/// actually changed.
#[must_use]
pub fn credential_keys(family: DeviceFamily) -> &'static [&'static str] {
    match family {
        DeviceFamily::Bos => &["bos_password"],
        DeviceFamily::Ubos => &["ubos_username", "ubos_password"],
        DeviceFamily::Bitaxe => &[],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    #[must_use]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "used in host tests only; wasm builds ids via for_family"
        )
    )]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Build an id namespaced by family. The mDNS instance name alone is not
    /// unique across families: BOS and Bitaxe both advertise subtypes of the
    /// same `_http._tcp` base type, so their resolved names can collide.
    /// Prefixing the family slug keeps the two apart.
    #[must_use]
    pub fn for_family(family: DeviceFamily, name: &str) -> Self {
        let slug = family_id(family);
        let mut value = String::with_capacity(slug.len() + 1 + name.len());
        value.push_str(slug);
        value.push('/');
        value.push_str(name);
        Self(value)
    }

    /// Build an id for an operator-entered manual host. The `manual/` infix
    /// keeps manual ids structurally disjoint from `for_family` ids built from
    /// an mDNS instance name, so discovery removal can never address a manual
    /// row. The full entry (including any `:port`) is preserved, so `host` and
    /// `host:port` are distinct devices.
    #[must_use]
    pub fn for_manual(family: DeviceFamily, entry: &str) -> Self {
        let slug = family_id(family);
        let infix = "/manual/";
        let mut value = String::with_capacity(slug.len() + infix.len() + entry.len());
        value.push_str(slug);
        value.push_str(infix);
        value.push_str(entry);
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSource {
    Discovered,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub id: DeviceId,
    pub family: DeviceFamily,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub source: DeviceSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnownDevice {
    pub identity: DeviceIdentity,
    pub model: Option<MinerModel>,
    pub telemetry: Option<TelemetrySnapshot>,
    pub last_seen_seq: u64,
    pub reachable: bool,
}

#[derive(Debug, Default)]
pub struct DeviceList {
    devices: Vec<KnownDevice>,
    seq: u64,
}

impl DeviceList {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "used in host tests only; the render path keys on summary groups"
        )
    )]
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// A monotonic counter bumped on every mutation (discovery, telemetry,
    /// removal). A derived view cached against this value is recomputed only
    /// when the fleet actually changed, never per render frame.
    #[must_use]
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(
            dead_code,
            reason = "render-side cache key; used by the wasm render path and host tests"
        )
    )]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    #[must_use]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "used in host tests only; not reachable on the wasm target"
        )
    )]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Insert a newly discovered device, or update the identity of an existing
    /// one with the same id, stamping it with a fresh discovery sequence.
    /// Reachability is left untouched: it is set only by telemetry polling, so
    /// a device is never counted online from an mDNS sighting alone. Returns
    /// `true` when the device was newly inserted, so callers can log
    /// first-discovery without firing on every mDNS re-announcement.
    pub fn upsert(&mut self, identity: DeviceIdentity) -> bool {
        self.seq += 1;
        let seq = self.seq;
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|d| d.identity.id == identity.id)
        {
            existing.identity = identity;
            existing.last_seen_seq = seq;
            false
        } else {
            self.devices.push(KnownDevice {
                identity,
                model: None,
                telemetry: None,
                last_seen_seq: seq,
                reachable: false,
            });
            true
        }
    }

    /// Insert or update a discovered device and apply an optional discovery
    /// model hint. A missing hint leaves any existing model intact, so later
    /// rediscovery does not erase a model learned from telemetry. Returns
    /// `true` when the device was newly inserted.
    pub fn upsert_with_model_hint(
        &mut self,
        identity: DeviceIdentity,
        model_hint: Option<MinerModel>,
    ) -> bool {
        let id = identity.id.clone();
        let is_new = self.upsert(identity);
        if let Some(model) = model_hint {
            self.apply_model(&id, model);
        }
        is_new
    }

    /// Bump the discovery sequence of a device still being announced.
    /// Reachability is left to telemetry polling.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "re-discovery hook; the discovery handler currently upserts instead"
        )
    )]
    pub fn mark_seen(&mut self, id: &DeviceId) {
        self.seq += 1;
        let seq = self.seq;
        if let Some(existing) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            existing.last_seen_seq = seq;
        }
    }

    /// Remove a device that discovery reported as gone.
    pub fn remove(&mut self, id: &DeviceId) {
        self.seq += 1;
        self.devices.retain(|d| &d.identity.id != id);
    }

    pub fn iter(&self) -> impl Iterator<Item = &KnownDevice> {
        self.devices.iter()
    }

    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "whole-list snapshot; the driver now snapshots per family via ids_for_family"
        )
    )]
    #[must_use]
    pub fn ids(&self) -> Vec<DeviceId> {
        self.devices.iter().map(|d| d.identity.id.clone()).collect()
    }

    #[must_use]
    pub fn ids_for_family(&self, family: DeviceFamily) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|d| d.identity.family == family)
            .map(|d| d.identity.id.clone())
            .collect()
    }

    /// Ids of devices in `family` that were added manually (not discovered).
    /// The reconcile uses this so it only ever adds or removes manual rows,
    /// leaving mDNS-discovered devices untouched.
    #[must_use]
    pub fn manual_ids_for_family(&self, family: DeviceFamily) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|d| d.identity.family == family && d.identity.source == DeviceSource::Manual)
            .map(|d| d.identity.id.clone())
            .collect()
    }

    /// Stamp the latest telemetry reading and reachability onto a device.
    pub fn apply_telemetry(&mut self, id: &DeviceId, reading: TelemetryReading, reachable: bool) {
        self.seq += 1;
        let seq = self.seq;
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.telemetry = Some(TelemetrySnapshot {
                reading,
                refreshed_seq: seq,
            });
            dev.reachable = reachable;
        }
    }

    /// Stamp the most recently fetched model onto a device by id. Model and
    /// telemetry are updated independently; if a fetch fails the caller omits
    /// the call and the previous model is retained.
    pub fn apply_model(&mut self, id: &DeviceId, model: MinerModel) {
        self.seq += 1;
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.model = Some(model);
        }
    }

    /// Drop one family's telemetry and mark its devices unreachable (e.g. after
    /// that family's credentials changed). Other families are left untouched, so
    /// a credential edit does not blank a family whose credentials did not move.
    /// Devices stay listed; readings/model go back to absent and reachability is
    /// recomputed on the next telemetry pass.
    pub fn clear_telemetry_for(&mut self, family: DeviceFamily) {
        self.seq += 1;
        for dev in self
            .devices
            .iter_mut()
            .filter(|d| d.identity.family == family)
        {
            dev.telemetry = None;
            dev.model = None;
            dev.reachable = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::TelemetryReading;

    fn identity(id: &str, host: &str) -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::new(id),
            family: DeviceFamily::Bos,
            name: id.to_owned(),
            host: host.to_owned(),
            port: 80,
            source: DeviceSource::Discovered,
        }
    }

    #[test]
    fn upsert_inserts_a_new_device() {
        let mut list = DeviceList::new();
        assert!(list.is_empty());
        let is_new = list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        assert!(is_new, "first sighting of a device must report as new");
        assert_eq!(list.len(), 1);
        assert!(!list.is_empty());
    }

    #[test]
    fn upsert_updates_existing_device_with_same_id() {
        let mut list = DeviceList::new();
        assert!(list.upsert(identity("a._http._tcp.local.", "10.0.0.1")));
        let is_new = list.upsert(identity("a._http._tcp.local.", "10.0.0.9"));
        assert!(
            !is_new,
            "re-announcement of a known device must not report as new"
        );
        assert_eq!(list.len(), 1);
        let dev = list.iter().next().expect("BUG: device present");
        assert_eq!(dev.identity.host, "10.0.0.9");
    }

    #[test]
    fn upsert_does_not_mark_a_device_reachable() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        let dev = list.iter().next().expect("BUG: device present");
        assert!(
            !dev.reachable,
            "a freshly discovered device is not online until polled"
        );
    }

    #[test]
    fn for_family_namespaces_the_id_by_family() {
        let bos = DeviceId::for_family(DeviceFamily::Bos, "x._http._tcp.local.");
        let axe = DeviceId::for_family(DeviceFamily::Bitaxe, "x._http._tcp.local.");
        assert_eq!(bos.as_str(), "bos/x._http._tcp.local.");
        assert_eq!(axe.as_str(), "bitaxe/x._http._tcp.local.");
        assert_ne!(
            bos, axe,
            "same instance name in two families must not collide"
        );
    }

    #[test]
    fn for_manual_namespaces_under_manual_segment() {
        let id = DeviceId::for_manual(DeviceFamily::Bos, "10.0.0.5");
        assert_eq!(id.as_str(), "bos/manual/10.0.0.5");
        let discovered = DeviceId::for_family(DeviceFamily::Bos, "10.0.0.5");
        assert_ne!(id, discovered, "manual and discovered ids must not collide");
    }

    #[test]
    fn manual_ids_for_family_returns_only_manual_rows() {
        let mut list = DeviceList::new();
        list.upsert(identity("disc._http._tcp.local.", "10.0.0.1"));
        let mut man = identity("10.0.0.5", "10.0.0.5");
        man.id = DeviceId::for_manual(DeviceFamily::Bos, "10.0.0.5");
        man.source = DeviceSource::Manual;
        list.upsert(man);
        let manual = list.manual_ids_for_family(DeviceFamily::Bos);
        assert_eq!(manual.len(), 1);
        assert_eq!(manual[0].as_str(), "bos/manual/10.0.0.5");
    }

    #[test]
    fn remove_drops_device_by_id() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.remove(&DeviceId::new("a._http._tcp.local."));
        assert!(list.is_empty());
    }

    #[test]
    fn every_mutation_advances_seq() {
        // The render cache keys on seq; a mutation that left it unchanged would
        // strand a stale view (e.g. a removed miner lingering in the summary).
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");

        let before = list.seq();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        assert!(list.seq() > before, "upsert must advance seq");

        let before = list.seq();
        list.apply_telemetry(&id, TelemetryReading::default(), true);
        assert!(list.seq() > before, "apply_telemetry must advance seq");

        let before = list.seq();
        list.clear_telemetry_for(DeviceFamily::Bos);
        assert!(list.seq() > before, "clear_telemetry_for must advance seq");

        let before = list.seq();
        list.remove(&id);
        assert!(list.seq() > before, "remove must advance seq");
    }

    #[test]
    fn family_label_covers_all_families() {
        assert_eq!(family_label(DeviceFamily::Bos), "BOS");
        assert_eq!(family_label(DeviceFamily::Ubos), "uBOS");
        assert_eq!(family_label(DeviceFamily::Bitaxe), "Bitaxe");
    }

    #[test]
    fn family_all_is_consistent_with_index() {
        assert_eq!(DeviceFamily::COUNT, DeviceFamily::ALL.len());
        for (i, family) in DeviceFamily::ALL.iter().enumerate() {
            assert_eq!(family.index(), i, "ALL order must match index()");
        }
    }

    #[test]
    fn family_map_round_trips_each_family() {
        let mut map: FamilyMap<usize> = FamilyMap::from_fn(DeviceFamily::index);
        for family in DeviceFamily::ALL {
            assert_eq!(map[family], family.index());
        }
        map[DeviceFamily::Ubos] = 99;
        assert_eq!(map[DeviceFamily::Ubos], 99);
        assert_eq!(map[DeviceFamily::Bos], DeviceFamily::Bos.index());
    }

    #[test]
    fn family_id_is_a_lowercase_slug_per_family() {
        assert_eq!(family_id(DeviceFamily::Bos), "bos");
        assert_eq!(family_id(DeviceFamily::Ubos), "ubos");
        assert_eq!(family_id(DeviceFamily::Bitaxe), "bitaxe");
    }

    #[test]
    fn device_id_exposes_its_string() {
        assert_eq!(
            DeviceId::new("miner-a._http._tcp.local.").as_str(),
            "miner-a._http._tcp.local."
        );
    }

    #[test]
    fn mark_seen_leaves_reachability_to_polling() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_telemetry(
            &DeviceId::new("a._http._tcp.local."),
            TelemetryReading::default(),
            true,
        );
        list.mark_seen(&DeviceId::new("a._http._tcp.local."));
        assert_eq!(list.len(), 1);
        let dev = list.iter().next().expect("BUG: device present");
        assert!(
            dev.reachable,
            "mark_seen must not disturb a poll-set reachability"
        );
    }

    #[test]
    fn apply_telemetry_stamps_reading_and_reachability() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        let reading = TelemetryReading {
            power_w: Some(3_000.0),
            ..TelemetryReading::default()
        };
        list.apply_telemetry(&DeviceId::new("a._http._tcp.local."), reading, true);
        let dev = list.iter().next().expect("BUG: device present");
        assert!(dev.reachable);
        let snap = dev.telemetry.as_ref().expect("BUG: telemetry present");
        assert_eq!(snap.reading.power_w, Some(3_000.0));
    }

    #[test]
    fn apply_telemetry_can_mark_unreachable() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_telemetry(
            &DeviceId::new("a._http._tcp.local."),
            TelemetryReading::default(),
            false,
        );
        assert!(!list.iter().next().expect("BUG: present").reachable);
    }

    #[test]
    fn credential_keys_cover_each_family() {
        assert_eq!(credential_keys(DeviceFamily::Bos), ["bos_password"]);
        assert_eq!(
            credential_keys(DeviceFamily::Ubos),
            ["ubos_username", "ubos_password"]
        );
        assert!(
            credential_keys(DeviceFamily::Bitaxe).is_empty(),
            "AxeOS has no credentials and must never be reset by a credential edit"
        );
    }

    #[test]
    fn clear_telemetry_for_drops_readings() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_telemetry(
            &DeviceId::new("a._http._tcp.local."),
            TelemetryReading::default(),
            true,
        );
        list.clear_telemetry_for(DeviceFamily::Bos);
        let dev = list.iter().next().expect("BUG: present");
        assert!(dev.telemetry.is_none());
        assert!(!dev.reachable);
    }

    #[test]
    fn clear_telemetry_for_leaves_other_families_intact() {
        let mut list = DeviceList::new();
        list.upsert(identity("bos._http._tcp.local.", "10.0.0.1"));
        let axe = DeviceIdentity {
            family: DeviceFamily::Bitaxe,
            ..identity("axe._http._tcp.local.", "10.0.0.2")
        };
        let axe_id = axe.id.clone();
        list.upsert(axe);
        list.apply_telemetry(
            &DeviceId::new("bos._http._tcp.local."),
            TelemetryReading::default(),
            true,
        );
        list.apply_telemetry(&axe_id, TelemetryReading::default(), true);

        list.clear_telemetry_for(DeviceFamily::Bos);

        let bos = list
            .iter()
            .find(|d| d.identity.family == DeviceFamily::Bos)
            .expect("BUG: present");
        let axe = list
            .iter()
            .find(|d| d.identity.family == DeviceFamily::Bitaxe)
            .expect("BUG: present");
        assert!(
            bos.telemetry.is_none() && !bos.reachable,
            "BOS telemetry cleared"
        );
        assert!(
            axe.telemetry.is_some() && axe.reachable,
            "a credential-less family must keep its telemetry when BOS credentials change"
        );
    }

    #[test]
    fn ids_for_family_filters_to_one_family() {
        let mut list = DeviceList::new();
        list.upsert(identity("bos._http._tcp.local.", "10.0.0.1"));
        let mut ubos = identity("ubos._ubos._tcp.local.", "10.0.0.2");
        ubos.family = DeviceFamily::Ubos;
        list.upsert(ubos);

        let bos_ids = list.ids_for_family(DeviceFamily::Bos);
        assert_eq!(bos_ids.len(), 1);
        assert_eq!(bos_ids[0].as_str(), "bos._http._tcp.local.");
        assert_eq!(list.ids_for_family(DeviceFamily::Ubos).len(), 1);
        assert!(list.ids_for_family(DeviceFamily::Bitaxe).is_empty());
    }

    #[test]
    fn ids_lists_every_device_in_order() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.upsert(identity("b._http._tcp.local.", "10.0.0.2"));
        let ids = list.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].as_str(), "a._http._tcp.local.");
        assert_eq!(ids[1].as_str(), "b._http._tcp.local.");
    }

    fn model(name: &str) -> MinerModel {
        MinerModel {
            id: "stm32mp157c-ii2-bmm1".to_owned(),
            name: name.to_owned(),
            chip_type: None,
            chip_count: None,
            nominal_hashrate_ths: None,
        }
    }

    #[test]
    fn apply_model_stamps_model_onto_device() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_model(&DeviceId::new("a._http._tcp.local."), model("BMM 101"));
        let dev = list.iter().next().expect("BUG: device present");
        assert_eq!(dev.model.as_ref().map(|m| m.name.as_str()), Some("BMM 101"));
    }

    #[test]
    fn upsert_with_model_hint_stamps_model_onto_new_device() {
        let mut list = DeviceList::new();
        let is_new = list.upsert_with_model_hint(
            identity("axe._http._tcp.local.", "10.0.0.8"),
            Some(model("Bitaxe Gamma 602")),
        );
        assert!(is_new, "first sighting must report as new");
        let dev = list
            .iter()
            .next()
            .expect("BUG: upsert_with_model_hint must insert a new device");
        assert_eq!(
            dev.model.as_ref().map(|m| m.name.as_str()),
            Some("Bitaxe Gamma 602")
        );
    }

    #[test]
    fn upsert_with_no_model_hint_preserves_existing_model() {
        let mut list = DeviceList::new();
        list.upsert_with_model_hint(
            identity("axe._http._tcp.local.", "10.0.0.8"),
            Some(model("Bitaxe Gamma 602")),
        );
        let is_new =
            list.upsert_with_model_hint(identity("axe._http._tcp.local.", "10.0.0.9"), None);
        assert!(!is_new, "re-announcement must not report as new");
        let dev = list
            .iter()
            .next()
            .expect("BUG: upsert_with_model_hint must preserve the device");
        assert_eq!(dev.identity.host, "10.0.0.9");
        assert_eq!(
            dev.model.as_ref().map(|m| m.name.as_str()),
            Some("Bitaxe Gamma 602")
        );
    }

    #[test]
    fn clear_telemetry_for_also_clears_model() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_model(&DeviceId::new("a._http._tcp.local."), model("BMM 101"));
        list.clear_telemetry_for(DeviceFamily::Bos);
        let dev = list.iter().next().expect("BUG: present");
        assert!(dev.model.is_none());
        assert!(!dev.reachable);
    }
}

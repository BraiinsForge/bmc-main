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

use bmc_wasm_sdk::credentials;

use crate::model::MinerModel;
use crate::telemetry::{TelemetryReading, TelemetrySnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFamily {
    Bos,
    Ubos,
    Bitaxe,
}

impl DeviceFamily {
    /// Every family, in storage order — the canonical order the summary groups
    /// and the poll ring walk families in.
    pub const ALL: [DeviceFamily; 3] =
        [DeviceFamily::Bos, DeviceFamily::Ubos, DeviceFamily::Bitaxe];

    /// Position of this family in [`ALL`](Self::ALL), a stable grouping key for
    /// the summary and history.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            DeviceFamily::Bos => 0,
            DeviceFamily::Ubos => 1,
            DeviceFamily::Bitaxe => 2,
        }
    }
}

#[must_use]
pub fn family_label(family: DeviceFamily) -> &'static str {
    match family {
        DeviceFamily::Bos => "BOS",
        DeviceFamily::Ubos => "Braiins OS Libre",
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

/// The manifest credential slot a family authenticates with,
/// `None` for a family that needs no account.
#[must_use]
pub fn credential_slot(family: DeviceFamily) -> Option<&'static str> {
    match family {
        DeviceFamily::Bos => Some("bos"),
        DeviceFamily::Ubos => Some("ubos"),
        DeviceFamily::Bitaxe => None,
    }
}

/// The families to start over after a credential delivery.
/// A delivery that changes no binding is a rotated password,
/// and the view cannot name the account that moved,
/// so every authenticating family starts over.
#[must_use]
pub fn families_on_new_credentials(
    current: &credentials::Snapshot,
    previous: &credentials::Snapshot,
) -> Vec<DeviceFamily> {
    let rotated = current == previous;
    DeviceFamily::ALL
        .into_iter()
        .filter(|&family| {
            credential_slot(family)
                .is_some_and(|slot| rotated || current.get(slot) != previous.get(slot))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    #[cfg(test)]
    #[must_use]
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

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub id: DeviceId,
    pub family: DeviceFamily,
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// Consecutive failed poll passes a device must miss before it is reported
/// unreachable (red). Below this many, the last good reading is kept and a
/// previously-reachable device stays reachable, so a single missed pass on a
/// flaky network does not blank the device.
const UNREACHABLE_AFTER_FAILED_PASSES: usize = 3;

/// Why the most recent poll pass failed, for a device's surfaced status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollFailure {
    /// No HTTP response at all — connection refused, timed out, or DNS failed.
    Unreachable,
    /// The device answered, but with an error status (e.g. 503) not usable data.
    ApiError,
    /// The device answered its login but rejected the credentials
    /// (401/403, or 200 without a token) — present, but not authenticating.
    AuthError,
    /// Nothing was sent: the family's slot is unbound,
    /// or the host refused to spend its account on this device.
    AccountUnusable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnownDevice {
    pub identity: DeviceIdentity,
    pub model: Option<MinerModel>,
    pub telemetry: Option<TelemetrySnapshot>,
    pub reachable: bool,
    /// Poll passes that have failed in a row since the last success. Reset to 0
    /// by any reachable pass; once it reaches [`UNREACHABLE_AFTER_FAILED_PASSES`]
    /// the device flips to unreachable.
    pub consecutive_failures: usize,
    /// Whether the device has ever delivered valid telemetry: a proven miner,
    /// kept through an mDNS removal, a credential reset or an unreachable spell.
    /// Never cleared, and independent of [`Self::reachable`].
    pub confirmed: bool,
    /// Why the last failed pass failed, for the surfaced status of a device with
    /// no live telemetry. `None` after a reachable pass or before any poll.
    pub last_failure: Option<PollFailure>,
    /// Unix secs since the device went no-response unreachable
    /// (`None` while reachable or API-erroring);
    /// drives retirement of a long-gone device.
    pub unreachable_since: Option<i64>,
}

/// A snapshot of the fleet by confirmation and liveness, for tracing where the
/// device count goes — removal or unreachability — from ground truth
/// instead of a fast-rotating poll log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub total: usize,
    pub reachable: usize,
    pub confirmed: usize,
}

#[derive(Debug, PartialEq)]
pub enum MdnsRemoval {
    Kept,
    Removed { model: Option<MinerModel> },
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

    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// A monotonic counter bumped on every mutation (discovery, telemetry,
    /// removal). A derived view cached against this value is recomputed only
    /// when the fleet actually changed, never per render frame.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    #[cfg(test)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Insert a newly discovered device,
    /// or update the identity of an existing one with the same id.
    /// Reachability is left untouched: it is set only by telemetry polling,
    /// so a device is never counted online from an mDNS sighting alone.
    /// Returns `true` when the device was newly inserted,
    /// so callers can log first-discovery without firing on every re-announcement.
    pub fn upsert(&mut self, identity: DeviceIdentity) -> bool {
        self.seq += 1;
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|d| d.identity.id == identity.id)
        {
            existing.identity = identity;
            false
        } else {
            self.devices.push(KnownDevice {
                identity,
                model: None,
                telemetry: None,
                reachable: false,
                consecutive_failures: 0,
                confirmed: false,
                last_failure: None,
                unreachable_since: None,
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

    /// Remove a device that discovery reported as gone.
    pub fn remove(&mut self, id: &DeviceId) {
        let before = self.devices.len();
        self.devices.retain(|d| &d.identity.id != id);
        if self.devices.len() != before {
            self.seq += 1;
        }
    }

    /// Settle an mDNS removal, which also fires when a Wi-Fi multicast refresh is lost.
    /// A miner that has answered, or one waiting for its family's account,
    /// stays and is left to polling; only a device that never proved itself leaves.
    /// `None` when the id is not in the fleet.
    pub fn on_mdns_removed(&mut self, id: &DeviceId) -> Option<MdnsRemoval> {
        let dev = self.devices.iter().find(|d| &d.identity.id == id)?;
        if dev.confirmed || dev.last_failure == Some(PollFailure::AccountUnusable) {
            return Some(MdnsRemoval::Kept);
        }
        let model = dev.model.clone();
        self.remove(id);
        Some(MdnsRemoval::Removed { model })
    }

    /// Retire devices unreachable with no response (not an API error) for over
    /// `ttl_secs`, by host clock `now_secs`. Keep-confirmed spares mDNS churn;
    /// this drops a genuinely gone device. Returns the number removed.
    #[must_use]
    pub fn prune_gone(&mut self, now_secs: i64, ttl_secs: i64) -> usize {
        for dev in &mut self.devices {
            if !dev.reachable && dev.last_failure == Some(PollFailure::Unreachable) {
                dev.unreachable_since.get_or_insert(now_secs);
            } else {
                dev.unreachable_since = None;
            }
        }
        let before = self.devices.len();
        self.devices.retain(|dev| {
            dev.unreachable_since
                .is_none_or(|since| now_secs.saturating_sub(since) <= ttl_secs)
        });
        let removed = before - self.devices.len();
        if removed > 0 {
            self.seq += 1;
        }
        removed
    }

    pub fn iter(&self) -> impl Iterator<Item = &KnownDevice> {
        self.devices.iter()
    }

    #[cfg(test)]
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

    /// Count the fleet by confirmation and liveness for a diagnostic snapshot.
    #[must_use]
    pub fn census(&self) -> Census {
        let mut c = Census {
            total: self.devices.len(),
            ..Census::default()
        };
        for dev in &self.devices {
            c.reachable += usize::from(dev.reachable);
            c.confirmed += usize::from(dev.confirmed);
        }
        c
    }

    /// Stamp the latest telemetry reading and reachability onto a device. A
    /// returned reading is the positive miner test, so this confirms the device
    /// and clears any recorded failure.
    pub fn apply_telemetry(&mut self, id: &DeviceId, reading: TelemetryReading, reachable: bool) {
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.telemetry = Some(TelemetrySnapshot { reading });
            dev.reachable = reachable;
            dev.confirmed = true;
            dev.last_failure = None;
            self.seq += 1;
        }
    }

    /// Record why the current pass failed, for the surfaced status of a device
    /// with no live telemetry. Overwritten each failed pass; cleared by a reachable
    /// one. Paired with [`Self::record_pass`], which already advances the sequence.
    pub fn set_last_failure(&mut self, id: &DeviceId, failure: PollFailure) {
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.last_failure = Some(failure);
        }
    }

    /// Record the result of a completed poll pass. A reachable pass stores the
    /// fresh reading, marks the device reachable, and clears its failure streak.
    /// A failed pass increments the streak but keeps the last good reading and
    /// reachability until [`UNREACHABLE_AFTER_FAILED_PASSES`] passes have failed
    /// in a row, only then flipping the device to unreachable (red). A device
    /// never yet reached stays unreachable throughout, since it has no values to
    /// keep. Returns the resulting consecutive-failure streak (0 after a
    /// reachable pass), so the caller can log how long a device has been missing.
    pub fn record_pass(
        &mut self,
        id: &DeviceId,
        reading: TelemetryReading,
        pass_reachable: bool,
    ) -> usize {
        if pass_reachable {
            self.apply_telemetry(id, reading, true);
            if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
                dev.consecutive_failures = 0;
                dev.unreachable_since = None;
            }
            return 0;
        }
        match self.devices.iter_mut().find(|d| &d.identity.id == id) {
            Some(dev) => {
                dev.consecutive_failures = dev.consecutive_failures.saturating_add(1);
                if dev.consecutive_failures >= UNREACHABLE_AFTER_FAILED_PASSES {
                    dev.reachable = false;
                }
                let streak = dev.consecutive_failures;
                self.seq += 1;
                streak
            }
            None => 0,
        }
    }

    /// Stamp the most recently fetched model onto a device by id. Model and
    /// telemetry are updated independently; if a fetch fails the caller omits
    /// the call and the previous model is retained.
    pub fn apply_model(&mut self, id: &DeviceId, model: MinerModel) {
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.model = Some(model);
            self.seq += 1;
        }
    }

    /// Mark one family's devices as unpolled for want of a usable account.
    /// Their model is kept, so a miner read before the account went away stays in its group.
    pub fn mark_account_unusable(&mut self, family: DeviceFamily) {
        let mut marked = false;
        for dev in self
            .devices
            .iter_mut()
            .filter(|d| d.identity.family == family)
        {
            dev.telemetry = None;
            dev.reachable = false;
            dev.consecutive_failures = 0;
            dev.last_failure = Some(PollFailure::AccountUnusable);
            marked = true;
        }
        if marked {
            self.seq += 1;
        }
    }

    pub fn reset_for_rebind(&mut self, family: DeviceFamily, bound: bool) {
        if bound {
            self.clear_telemetry_for(family);
        } else {
            self.mark_account_unusable(family);
        }
    }

    /// Drop one family's telemetry and mark its devices unreachable,
    /// e.g. after that family's account changed; other families are left untouched.
    /// Devices stay listed and keep their model: an account change leaves the hardware as it was.
    /// Readings stay absent until the next telemetry pass.
    pub fn clear_telemetry_for(&mut self, family: DeviceFamily) {
        let mut cleared = false;
        for dev in self
            .devices
            .iter_mut()
            .filter(|d| d.identity.family == family)
        {
            dev.telemetry = None;
            dev.reachable = false;
            dev.consecutive_failures = 0;
            dev.last_failure = None;
            cleared = true;
        }
        if cleared {
            self.seq += 1;
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
    fn remove_drops_device_by_id() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.remove(&DeviceId::new("a._http._tcp.local."));
        assert!(list.is_empty());
    }

    #[test]
    fn an_mdns_removal_drops_a_miner_that_never_answered() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.apply_model(&id, model("S19"));

        assert_eq!(
            list.on_mdns_removed(&id),
            Some(MdnsRemoval::Removed {
                model: Some(model("S19"))
            })
        );
        assert!(list.is_empty());
    }

    #[test]
    fn an_mdns_removal_keeps_a_confirmed_miner() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);

        assert_eq!(list.on_mdns_removed(&id), Some(MdnsRemoval::Kept));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn an_mdns_removal_keeps_a_miner_waiting_for_its_account() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.mark_account_unusable(DeviceFamily::Bos);

        assert_eq!(
            list.on_mdns_removed(&id),
            Some(MdnsRemoval::Kept),
            "unpolled, it never confirms, so dropping it would churn it on a lossy network"
        );
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn an_mdns_removal_of_an_unknown_id_changes_nothing() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        let before = list.seq();

        assert_eq!(
            list.on_mdns_removed(&DeviceId::new("gone._http._tcp.local.")),
            None
        );
        assert_eq!(list.seq(), before);
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
    fn a_call_that_changes_nothing_leaves_seq_alone() {
        // A pass finishing for a device that discovery already dropped
        // mutates nothing, so it must not invalidate the cached summary.
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        let departed = DeviceId::new("gone._http._tcp.local.");

        let before = list.seq();
        list.apply_telemetry(&departed, TelemetryReading::default(), true);
        assert_eq!(list.seq(), before, "telemetry for a departed device");

        list.apply_model(&departed, model("BMM 101"));
        assert_eq!(list.seq(), before, "model for a departed device");

        assert_eq!(
            list.record_pass(&departed, TelemetryReading::default(), false),
            0
        );
        assert_eq!(list.seq(), before, "failed pass for a departed device");

        list.remove(&departed);
        assert_eq!(list.seq(), before, "removing what was never there");

        list.clear_telemetry_for(DeviceFamily::Bitaxe);
        assert_eq!(list.seq(), before, "clearing a family with no devices");
    }

    #[test]
    fn family_label_covers_all_families() {
        assert_eq!(family_label(DeviceFamily::Bos), "BOS");
        assert_eq!(family_label(DeviceFamily::Ubos), "Braiins OS Libre");
        assert_eq!(family_label(DeviceFamily::Bitaxe), "Bitaxe");
    }

    #[test]
    fn family_all_is_consistent_with_index() {
        for (i, family) in DeviceFamily::ALL.iter().enumerate() {
            assert_eq!(family.index(), i, "ALL order must match index()");
        }
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
    fn credential_slot_covers_each_family() {
        assert_eq!(credential_slot(DeviceFamily::Bos), Some("bos"));
        assert_eq!(credential_slot(DeviceFamily::Ubos), Some("ubos"));
        assert_eq!(
            credential_slot(DeviceFamily::Bitaxe),
            None,
            "AxeOS has no credentials and must never be reset by a credential edit"
        );
    }

    /// The host's view encoding: a `u32` slot count,
    /// then per slot its length-prefixed name, type id and account name.
    fn snapshot(slots: &[(&str, &str)]) -> credentials::Snapshot {
        let mut bytes = u32::try_from(slots.len())
            .expect("BUG: test size")
            .to_le_bytes()
            .to_vec();
        for (slot, account) in slots {
            for text in [*slot, "generic-userpass", *account] {
                let len = u16::try_from(text.len()).expect("BUG: test size");
                bytes.extend_from_slice(&len.to_le_bytes());
                bytes.extend_from_slice(text.as_bytes());
            }
        }
        credentials::Snapshot::from_bytes(&bytes)
    }

    #[test]
    fn binding_one_slot_restarts_only_its_family() {
        assert_eq!(
            families_on_new_credentials(&snapshot(&[("bos", "Fleet")]), &snapshot(&[])),
            [DeviceFamily::Bos]
        );
    }

    #[test]
    fn unbinding_a_slot_restarts_only_its_family() {
        assert_eq!(
            families_on_new_credentials(
                &snapshot(&[("bos", "Fleet")]),
                &snapshot(&[("bos", "Fleet"), ("ubos", "Old")])
            ),
            [DeviceFamily::Ubos]
        );
    }

    #[test]
    fn swapping_an_account_restarts_its_family() {
        let before = snapshot(&[("bos", "Fleet"), ("ubos", "Old")]);
        let after = snapshot(&[("bos", "Fleet"), ("ubos", "New")]);

        assert_eq!(
            families_on_new_credentials(&after, &before),
            [DeviceFamily::Ubos]
        );
    }

    #[test]
    fn an_unchanged_view_is_a_rotation_that_restarts_every_authenticating_family() {
        let view = snapshot(&[("bos", "Fleet")]);

        assert_eq!(
            families_on_new_credentials(&view, &view),
            [DeviceFamily::Bos, DeviceFamily::Ubos],
            "a rotated password cannot be attributed, and AxeOS has no account to rotate"
        );
    }

    #[test]
    fn mark_account_unusable_blanks_the_family_but_keeps_its_model() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.apply_model(&id, model("S19"));

        list.mark_account_unusable(DeviceFamily::Bos);

        let dev = list.iter().next().expect("BUG: present");
        assert!(dev.telemetry.is_none() && !dev.reachable);
        assert_eq!(dev.last_failure, Some(PollFailure::AccountUnusable));
        assert!(
            dev.model.is_some(),
            "the miner must stay in its model group"
        );
    }

    #[test]
    fn mark_account_unusable_leaves_other_families_intact() {
        let mut list = DeviceList::new();
        let axe = DeviceIdentity {
            family: DeviceFamily::Bitaxe,
            ..identity("axe._http._tcp.local.", "10.0.0.2")
        };
        let axe_id = axe.id.clone();
        list.upsert(axe);
        list.apply_telemetry(&axe_id, TelemetryReading::default(), true);

        list.mark_account_unusable(DeviceFamily::Bos);

        let axe = list.iter().next().expect("BUG: present");
        assert!(axe.reachable && axe.last_failure.is_none());
    }

    #[test]
    fn unbinding_an_account_keeps_the_family_in_its_model_groups() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.apply_model(&id, model("S19"));

        list.reset_for_rebind(DeviceFamily::Bos, false);

        let dev = list.iter().next().expect("BUG: present");
        assert!(
            dev.model.is_some() && dev.last_failure == Some(PollFailure::AccountUnusable),
            "an unbound family must read \"Check account\" without moving to Unknown"
        );
    }

    #[test]
    fn rebinding_an_account_starts_the_family_over() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.set_last_failure(&id, PollFailure::AccountUnusable);

        list.reset_for_rebind(DeviceFamily::Bos, true);

        let dev = list.iter().next().expect("BUG: present");
        assert!(
            dev.last_failure.is_none(),
            "a bound family must leave \"Check account\" until its next pass"
        );
    }

    #[test]
    fn rebinding_an_account_keeps_the_family_in_its_model_groups() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a._http._tcp.local.");
        list.upsert(identity(id.as_str(), "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.apply_model(&id, model("S19"));

        list.reset_for_rebind(DeviceFamily::Bos, true);

        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(
            dev.model.as_ref().map(|m| m.name.as_str()),
            Some("S19"),
            "a new account must not move the miner to Unknown"
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

    fn good_reading() -> TelemetryReading {
        TelemetryReading {
            current_hashrate_ths: Some(5.0),
            power_w: Some(3_000.0),
            ..TelemetryReading::default()
        }
    }

    #[test]
    fn record_pass_returns_the_consecutive_failure_count() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        assert_eq!(list.record_pass(&id, good_reading(), true), 0);
        assert_eq!(list.record_pass(&id, TelemetryReading::default(), false), 1);
        assert_eq!(list.record_pass(&id, TelemetryReading::default(), false), 2);
        assert_eq!(list.record_pass(&id, TelemetryReading::default(), false), 3);
        assert_eq!(
            list.record_pass(&id, good_reading(), true),
            0,
            "a successful pass resets the count to zero"
        );
    }

    #[test]
    fn record_pass_keeps_last_values_through_two_failures() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        // Two consecutive failures, below the threshold of three.
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        let dev = list.iter().next().expect("BUG: present");
        assert!(dev.reachable, "below threshold the device stays reachable");
        let snap = dev.telemetry.as_ref().expect("BUG: last reading kept");
        assert_eq!(
            snap.reading.power_w,
            Some(3_000.0),
            "the last good values are kept, not blanked"
        );
    }

    #[test]
    fn record_pass_marks_unreachable_on_third_consecutive_failure() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        assert!(
            list.iter().next().expect("BUG: present").reachable,
            "still reachable after two failures"
        );
        list.record_pass(&id, TelemetryReading::default(), false);
        assert!(
            !list.iter().next().expect("BUG: present").reachable,
            "the third consecutive failure turns the device red"
        );
    }

    #[test]
    fn record_pass_success_resets_the_failure_streak() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, good_reading(), true);
        // The streak reset means two further failures are again below threshold.
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        assert!(
            list.iter().next().expect("BUG: present").reachable,
            "a successful pass resets the streak, so two more failures stay reachable"
        );
    }

    #[test]
    fn record_pass_success_after_red_restores_reachable_with_new_reading() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        for _ in 0..3 {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        assert!(
            !list.iter().next().expect("BUG: present").reachable,
            "red after three failures"
        );
        let fresh = TelemetryReading {
            power_w: Some(1_234.0),
            ..TelemetryReading::default()
        };
        list.record_pass(&id, fresh, true);
        let dev = list.iter().next().expect("BUG: present");
        assert!(dev.reachable, "a success restores reachability");
        assert_eq!(
            dev.telemetry
                .as_ref()
                .expect("BUG: reading")
                .reading
                .power_w,
            Some(1_234.0),
            "the fresh reading replaces the old one"
        );
    }

    #[test]
    fn record_pass_never_reached_device_stays_not_reachable_below_threshold() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        // Upsert leaves a device reachable=false with no telemetry.
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        let dev = list.iter().next().expect("BUG: present");
        assert!(
            !dev.reachable,
            "a device never reached must not be shown green during the grace period"
        );
        assert!(dev.telemetry.is_none(), "there are no values to keep");
    }

    #[test]
    fn a_discovered_device_is_unconfirmed_until_it_answers() {
        let mut list = DeviceList::new();
        list.upsert(identity("a", "10.0.0.1"));
        let dev = list.iter().next().expect("BUG: present");
        assert!(!dev.confirmed, "unconfirmed until it answers");
    }

    #[test]
    fn a_reachable_pass_confirms_the_device() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        let dev = list.iter().next().expect("BUG: present");
        assert!(dev.confirmed, "answering a poll proves the miner");
    }

    #[test]
    fn census_counts_by_confirmation_and_liveness() {
        let mut list = DeviceList::new();
        let confirmed = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&confirmed, good_reading(), true);
        let silent = DeviceId::new("b");
        list.upsert(identity("b", "10.0.0.2"));
        list.record_pass(&silent, TelemetryReading::default(), false);
        let c = list.census();
        assert_eq!(c.total, 2);
        assert_eq!(c.confirmed, 1);
        assert_eq!(c.reachable, 1, "only the answered device is reachable");
    }

    #[test]
    fn confirmation_survives_a_credential_clear() {
        // A confirmed device stays confirmed (and reported) when a credential
        // change clears its telemetry — it must not drop out of the report.
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.clear_telemetry_for(DeviceFamily::Bos);
        let dev = list.iter().next().expect("BUG: present");
        assert!(
            dev.confirmed,
            "a confirmed device survives a telemetry clear"
        );
    }

    #[test]
    fn a_confirmed_device_is_never_demoted_when_red() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        for _ in 0..UNREACHABLE_AFTER_FAILED_PASSES + 2 {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        let dev = list.iter().next().expect("BUG: present");
        assert!(!dev.reachable);
        assert!(dev.confirmed, "a red miner stays a proven miner");
    }

    #[test]
    fn an_unanswering_device_stays_in_the_fleet() {
        // A uBOS whose API 503s, or a miner still booting, must not be dropped.
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        for _ in 0..UNREACHABLE_AFTER_FAILED_PASSES + 3 {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        let dev = list
            .iter()
            .next()
            .expect("BUG: an unanswering device must stay listed");
        assert!(!dev.confirmed, "and stays unconfirmed");
    }

    #[test]
    fn set_last_failure_records_the_reason_and_a_reachable_pass_clears_it() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, TelemetryReading::default(), false);
        list.set_last_failure(&id, PollFailure::ApiError);
        assert_eq!(
            list.iter().next().expect("BUG: present").last_failure,
            Some(PollFailure::ApiError)
        );
        list.record_pass(&id, good_reading(), true);
        assert_eq!(
            list.iter().next().expect("BUG: present").last_failure,
            None,
            "a reachable pass clears the recorded failure"
        );
    }

    #[test]
    fn clear_telemetry_for_resets_the_failure_streak() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true);
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        list.clear_telemetry_for(DeviceFamily::Bos);
        // After a credential-driven clear the streak starts fresh: two failures
        // are again below threshold rather than tipping straight to red.
        list.record_pass(&id, TelemetryReading::default(), false);
        list.record_pass(&id, TelemetryReading::default(), false);
        assert_eq!(
            list.iter()
                .next()
                .expect("BUG: present")
                .consecutive_failures,
            2,
            "the clear reset the streak before these two failures"
        );
    }

    // A device that answered, then went unreachable with no response.
    fn make_gone(list: &mut DeviceList, id_str: &str, failure: PollFailure) -> DeviceId {
        let id = DeviceId::new(id_str);
        list.upsert(identity(id_str, "10.0.0.9"));
        list.record_pass(&id, good_reading(), true);
        for _ in 0..UNREACHABLE_AFTER_FAILED_PASSES {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        list.set_last_failure(&id, failure);
        id
    }

    #[test]
    fn prune_gone_retires_a_device_unreachable_past_the_ttl() {
        let mut list = DeviceList::new();
        make_gone(
            &mut list,
            "bos/dead._http._tcp.local.",
            PollFailure::Unreachable,
        );
        // The first prune stamps the clock; still inside the window.
        assert_eq!(list.prune_gone(1_000, 300), 0);
        assert_eq!(list.len(), 1, "within the ttl the device is kept");
        // Past the window it retires from the fleet.
        assert_eq!(list.prune_gone(1_301, 300), 1);
        assert!(list.is_empty(), "a long-gone device is dropped");
    }

    #[test]
    fn prune_gone_spares_an_api_erroring_device() {
        let mut list = DeviceList::new();
        make_gone(
            &mut list,
            "ubos/busy._ubos._tcp.local.",
            PollFailure::ApiError,
        );
        // A 503 device is present but erroring — never retired, however long.
        assert_eq!(list.prune_gone(1_000, 300), 0);
        assert_eq!(list.prune_gone(9_999, 300), 0);
        assert_eq!(list.len(), 1, "a 503 device stays in the fleet");
    }

    #[test]
    fn prune_gone_spares_a_device_without_a_usable_account() {
        let mut list = DeviceList::new();
        make_gone(
            &mut list,
            "bos/unbound._http._tcp.local.",
            PollFailure::AccountUnusable,
        );
        assert_eq!(list.prune_gone(1_000, 300), 0);
        assert_eq!(
            list.prune_gone(9_999, 300),
            0,
            "an unpolled miner is not gone, it waits for its account"
        );
    }

    #[test]
    fn prune_gone_timer_resets_when_a_device_recovers() {
        let mut list = DeviceList::new();
        let id = make_gone(
            &mut list,
            "bos/flap._http._tcp.local.",
            PollFailure::Unreachable,
        );
        assert_eq!(list.prune_gone(1_000, 300), 0, "stamped at t=1000");
        // It answers again before the ttl — the timer must reset.
        list.record_pass(&id, good_reading(), true);
        assert_eq!(list.prune_gone(1_400, 300), 0, "recovered, so not retired");
        assert_eq!(list.len(), 1);
    }
}

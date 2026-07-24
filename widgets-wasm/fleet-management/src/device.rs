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

/// Failed poll passes a never-confirmed candidate is probed before it is treated
/// as a non-miner `_http._tcp` responder and left dormant (dropped from the poll
/// cursor). A real miner confirms on its first answered pass, well within this
/// budget; a mDNS re-announce after a genuine restart re-adds it as a fresh
/// candidate with a fresh budget.
const CANDIDATE_PROBE_PASSES: usize = 3;

/// How far a device has earned into the fleet, along one axis: how sure we are
/// it's a miner and whether it may be reported. Orthogonal to liveness
/// ([`KnownDevice::reachable`]) — a `Confirmed` device can still be unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// A base-type `_http._tcp` sighting on probation: polled within the probe
    /// budget, hidden until it answers. Might not be a miner at all.
    Candidate,
    /// A candidate that spent its probe budget without answering — treated as a
    /// non-miner responder: no longer polled, never reported.
    Dormant,
    /// Positively family-identified at discovery (uBOS type or AxeOS TXT).
    /// Polled indefinitely and shown — with a "not responding" status
    /// until it delivers telemetry.
    Identified,
    /// Has delivered valid telemetry at least once. Polled indefinitely and shown
    /// with live data. Terminal: never demoted, so a credential reset or an
    /// unreachable spell does not drop it from the report.
    Confirmed,
}

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
    /// How far the device has earned into the report and the poll cursor — see
    /// [`Membership`]; drives [`Self::is_reported`] and [`Self::is_pollable`].
    pub membership: Membership,
    /// Why the last failed pass failed, for the surfaced status of a device with
    /// no live telemetry. `None` after a reachable pass or before any poll.
    pub last_failure: Option<PollFailure>,
    /// Unix secs since the device went no-response unreachable
    /// (`None` while reachable or API-erroring);
    /// drives retirement of a long-gone device.
    pub unreachable_since: Option<i64>,
}

impl KnownDevice {
    /// Whether the poll driver should keep contacting this device — everything but
    /// a spent [`Membership::Dormant`] candidate. A confirmed or identified miner
    /// may recover or still be booting; a live candidate is within its budget.
    #[must_use]
    pub fn is_pollable(&self) -> bool {
        self.membership != Membership::Dormant
    }

    /// Whether the device has earned a place in the fleet report: positively
    /// identified or confirmed. A bare candidate stays out until it answers.
    #[must_use]
    pub fn is_reported(&self) -> bool {
        matches!(
            self.membership,
            Membership::Identified | Membership::Confirmed
        )
    }

    /// Whether the device has ever answered a poll — a proven miner,
    /// kept in the fleet even when mDNS drops its record
    /// (its liveness is polling-governed).
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.membership == Membership::Confirmed
    }
}

/// A snapshot of the fleet by membership and liveness, for tracing where the
/// device count goes — Dormant retirement, removal, or unreachability — from
/// ground truth instead of a fast-rotating poll log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub total: usize,
    pub reported: usize,
    pub reachable: usize,
    pub candidate: usize,
    pub dormant: usize,
    pub identified: usize,
    pub confirmed: usize,
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
                membership: Membership::Candidate,
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

    /// Ids of devices in `family` still worth polling ([`KnownDevice::is_pollable`]):
    /// every confirmed device plus candidates that have not yet spent their probe
    /// budget. The poll driver builds each pass from this, so a dead `_http._tcp`
    /// responder drops out instead of slowing every pass with a doomed login.
    #[must_use]
    pub fn pollable_ids_for_family(&self, family: DeviceFamily) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|d| d.identity.family == family && d.is_pollable())
            .map(|d| d.identity.id.clone())
            .collect()
    }

    /// Count the fleet by membership and liveness for a diagnostic snapshot.
    #[must_use]
    pub fn census(&self) -> Census {
        let mut c = Census {
            total: self.devices.len(),
            ..Census::default()
        };
        for dev in &self.devices {
            c.reported += usize::from(dev.is_reported());
            c.reachable += usize::from(dev.reachable);
            match dev.membership {
                Membership::Candidate => c.candidate += 1,
                Membership::Dormant => c.dormant += 1,
                Membership::Identified => c.identified += 1,
                Membership::Confirmed => c.confirmed += 1,
            }
        }
        c
    }

    /// Stamp the latest telemetry reading and reachability onto a device. A
    /// returned reading is the positive miner test, so this promotes the device to
    /// [`Membership::Confirmed`] and clears any recorded failure.
    pub fn apply_telemetry(&mut self, id: &DeviceId, reading: TelemetryReading, reachable: bool) {
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.telemetry = Some(TelemetrySnapshot { reading });
            dev.reachable = reachable;
            dev.membership = Membership::Confirmed;
            dev.last_failure = None;
            self.seq += 1;
        }
    }

    /// Promote a positively family-identified device (AxeOS TXT or uBOS dedicated type)
    /// to [`Membership::Identified`]: shown and polled indefinitely,
    /// though "not responding" until it delivers telemetry.
    /// Leaves a [`Membership::Confirmed`] device alone (never demotes)
    /// and no-ops for an unknown id.
    pub fn identify(&mut self, id: &DeviceId) {
        let changed = self
            .devices
            .iter_mut()
            .find(|d| &d.identity.id == id)
            .is_some_and(|dev| {
                let promote = matches!(dev.membership, Membership::Candidate | Membership::Dormant);
                if promote {
                    dev.membership = Membership::Identified;
                }
                promote
            });
        if changed {
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
                // A bare candidate that spends its probe budget is a non-miner
                // responder: retire it from the poll cursor.
                if dev.membership == Membership::Candidate
                    && dev.consecutive_failures >= CANDIDATE_PROBE_PASSES
                {
                    dev.membership = Membership::Dormant;
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

    /// Drop one family's telemetry and mark its devices unreachable (e.g. after
    /// that family's credentials changed). Other families are left untouched, so
    /// a credential edit does not blank a family whose credentials did not move.
    /// Devices stay listed; readings/model go back to absent and reachability is
    /// recomputed on the next telemetry pass.
    pub fn clear_telemetry_for(&mut self, family: DeviceFamily) {
        let mut cleared = false;
        for dev in self
            .devices
            .iter_mut()
            .filter(|d| d.identity.family == family)
        {
            dev.telemetry = None;
            dev.model = None;
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
    fn a_fresh_candidate_is_polled_but_not_reported() {
        let mut list = DeviceList::new();
        list.upsert(identity("a", "10.0.0.1"));
        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(dev.membership, Membership::Candidate);
        assert!(dev.is_pollable(), "a fresh candidate is polled");
        assert!(!dev.is_reported(), "but hidden until it answers");
        assert!(!dev.is_confirmed(), "and unconfirmed until it answers");
    }

    #[test]
    fn a_reachable_pass_confirms_the_device() {
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        assert!(!list.iter().next().expect("BUG: present").is_reported());
        list.record_pass(&id, good_reading(), true);
        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(
            dev.membership,
            Membership::Confirmed,
            "answering a poll confirms"
        );
        assert!(dev.is_reported());
        assert!(dev.is_confirmed(), "and is now a proven miner");
    }

    #[test]
    fn census_counts_by_membership_and_liveness() {
        let mut list = DeviceList::new();
        let confirmed = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&confirmed, good_reading(), true);
        let dormant = DeviceId::new("b");
        list.upsert(identity("b", "10.0.0.2"));
        for _ in 0..CANDIDATE_PROBE_PASSES {
            list.record_pass(&dormant, TelemetryReading::default(), false);
        }
        let c = list.census();
        assert_eq!(c.total, 2);
        assert_eq!(c.confirmed, 1);
        assert_eq!(c.reachable, 1, "only the answered device is reachable");
        assert_eq!(c.dormant, 1, "the spent candidate retired to dormant");
        assert_eq!(c.reported, 1, "the dormant candidate is not reported");
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
        assert_eq!(dev.membership, Membership::Confirmed);
        assert!(
            dev.is_reported(),
            "a confirmed device survives a telemetry clear"
        );
    }

    #[test]
    fn identify_shows_and_polls_but_does_not_confirm() {
        // A positively family-identified device (uBOS/AxeOS) is reported and
        // polled at once, but stays "identified, not confirmed" until it answers.
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.identify(&id);
        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(dev.membership, Membership::Identified);
        assert!(dev.is_reported(), "identified devices are shown");
        assert!(dev.is_pollable());
    }

    #[test]
    fn a_spent_candidate_goes_dormant_and_drops_out_of_the_poll_cursor() {
        // A never-answering candidate (a non-miner _http._tcp responder) is polled
        // only until it exhausts its probe budget, then retired: hidden and unpolled.
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        assert_eq!(list.pollable_ids_for_family(DeviceFamily::Bos).len(), 1);
        for _ in 0..CANDIDATE_PROBE_PASSES {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(dev.membership, Membership::Dormant);
        assert!(
            !dev.is_reported(),
            "a dead candidate never enters the report"
        );
        assert!(
            list.pollable_ids_for_family(DeviceFamily::Bos).is_empty(),
            "and stops being polled"
        );
    }

    #[test]
    fn a_confirmed_device_keeps_being_polled_when_red() {
        // A confirmed miner gone unreachable must stay in the cursor so it can
        // recover; only unconfirmed candidates go dormant.
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.record_pass(&id, good_reading(), true); // confirms it
        for _ in 0..CANDIDATE_PROBE_PASSES + 2 {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(
            dev.membership,
            Membership::Confirmed,
            "never demoted to dormant"
        );
        assert_eq!(
            list.pollable_ids_for_family(DeviceFamily::Bos).len(),
            1,
            "a confirmed device keeps being polled even when red"
        );
    }

    #[test]
    fn an_identified_device_is_polled_past_the_budget_but_not_confirmed() {
        // A positively family-identified device that never answers (a uBOS whose
        // API 503s) keeps being polled — a still-booting miner must not be dropped —
        // yet stays "identified, not confirmed" until it delivers telemetry.
        let mut list = DeviceList::new();
        let id = DeviceId::new("a");
        list.upsert(identity("a", "10.0.0.1"));
        list.identify(&id);
        for _ in 0..CANDIDATE_PROBE_PASSES + 3 {
            list.record_pass(&id, TelemetryReading::default(), false);
        }
        let dev = list.iter().next().expect("BUG: present");
        assert_eq!(
            dev.membership,
            Membership::Identified,
            "identified is never retired"
        );
        assert!(
            dev.is_pollable(),
            "and stays in the poll cursor past the budget"
        );
        assert_eq!(list.pollable_ids_for_family(DeviceFamily::Bos).len(), 1);
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

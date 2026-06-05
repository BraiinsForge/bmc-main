// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::model::MinerModel;
use crate::telemetry::{TelemetryReading, TelemetrySnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFamily {
    Bos,
    Ubos,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "part of the generic device model; constructed once uBOS and Bitaxe adapters land"
        )
    )]
    Bitaxe,
}

#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "part of the device model; consumed by tests and future per-device detail views"
    )
)]
#[must_use]
pub fn family_label(family: DeviceFamily) -> &'static str {
    match family {
        DeviceFamily::Bos => "BOS",
        DeviceFamily::Ubos => "uBOS",
        DeviceFamily::Bitaxe => "Bitaxe",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "exposed for logging and diagnostics; not yet consumed on-device"
        )
    )]
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
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Insert a newly discovered device, or update the identity of an existing
    /// one with the same id. Either way the device is marked reachable and
    /// stamped with a fresh discovery sequence.
    pub fn upsert(&mut self, identity: DeviceIdentity) {
        self.seq += 1;
        let seq = self.seq;
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|d| d.identity.id == identity.id)
        {
            existing.identity = identity;
            existing.last_seen_seq = seq;
            existing.reachable = true;
        } else {
            self.devices.push(KnownDevice {
                identity,
                model: None,
                telemetry: None,
                last_seen_seq: seq,
                reachable: true,
            });
        }
    }

    /// Insert or update a discovered device and apply an optional discovery
    /// model hint. A missing hint leaves any existing model intact, so later
    /// rediscovery does not erase a model learned from telemetry.
    pub fn upsert_with_model_hint(
        &mut self,
        identity: DeviceIdentity,
        model_hint: Option<MinerModel>,
    ) {
        let id = identity.id.clone();
        self.upsert(identity);
        if let Some(model) = model_hint {
            self.apply_model(&id, model);
        }
    }

    /// Bump the discovery sequence of a device still being announced.
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
            existing.reachable = true;
        }
    }

    /// Remove a device that discovery reported as gone.
    pub fn remove(&mut self, id: &DeviceId) {
        self.devices.retain(|d| &d.identity.id != id);
    }

    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "re-discovery hook; render now iterates all devices via iter()"
        )
    )]
    pub fn iter_reachable(&self) -> impl Iterator<Item = &KnownDevice> {
        self.devices.iter().filter(|d| d.reachable)
    }

    pub fn iter(&self) -> impl Iterator<Item = &KnownDevice> {
        self.devices.iter()
    }

    #[must_use]
    pub fn ids(&self) -> Vec<DeviceId> {
        self.devices.iter().map(|d| d.identity.id.clone()).collect()
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
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.model = Some(model);
        }
    }

    /// Drop every device's telemetry and mark it unreachable (e.g. after a
    /// credential change). Devices stay listed; their readings and model go
    /// back to absent and reachability is recomputed on the next telemetry pass.
    pub fn clear_all_telemetry(&mut self) {
        for dev in &mut self.devices {
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
        }
    }

    #[test]
    fn upsert_inserts_a_new_device() {
        let mut list = DeviceList::new();
        assert!(list.is_empty());
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        assert_eq!(list.len(), 1);
        assert!(!list.is_empty());
    }

    #[test]
    fn upsert_updates_existing_device_with_same_id() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.upsert(identity("a._http._tcp.local.", "10.0.0.9"));
        assert_eq!(list.len(), 1);
        let dev = list.iter_reachable().next().expect("device present");
        assert_eq!(dev.identity.host, "10.0.0.9");
    }

    #[test]
    fn remove_drops_device_by_id() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.remove(&DeviceId::new("a._http._tcp.local."));
        assert!(list.is_empty());
    }

    #[test]
    fn family_label_covers_all_families() {
        assert_eq!(family_label(DeviceFamily::Bos), "BOS");
        assert_eq!(family_label(DeviceFamily::Ubos), "uBOS");
        assert_eq!(family_label(DeviceFamily::Bitaxe), "Bitaxe");
    }

    #[test]
    fn device_id_exposes_its_string() {
        assert_eq!(
            DeviceId::new("miner-a._http._tcp.local.").as_str(),
            "miner-a._http._tcp.local."
        );
    }

    #[test]
    fn mark_seen_keeps_an_existing_device_reachable() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.mark_seen(&DeviceId::new("a._http._tcp.local."));
        assert_eq!(list.len(), 1);
        let dev = list.iter_reachable().next().expect("device present");
        assert!(dev.reachable);
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
        let dev = list.iter().next().expect("device present");
        assert!(dev.reachable);
        let snap = dev.telemetry.as_ref().expect("telemetry present");
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
        assert!(!list.iter().next().expect("present").reachable);
    }

    #[test]
    fn clear_all_telemetry_drops_readings() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_telemetry(
            &DeviceId::new("a._http._tcp.local."),
            TelemetryReading::default(),
            true,
        );
        list.clear_all_telemetry();
        let dev = list.iter().next().expect("present");
        assert!(dev.telemetry.is_none());
        assert!(!dev.reachable);
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
        let dev = list.iter().next().expect("device present");
        assert_eq!(dev.model.as_ref().map(|m| m.name.as_str()), Some("BMM 101"));
    }

    #[test]
    fn upsert_with_model_hint_stamps_model_onto_new_device() {
        let mut list = DeviceList::new();
        list.upsert_with_model_hint(
            identity("axe._http._tcp.local.", "10.0.0.8"),
            Some(model("Bitaxe Gamma 602")),
        );
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
        list.upsert_with_model_hint(identity("axe._http._tcp.local.", "10.0.0.9"), None);
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
    fn clear_all_telemetry_also_clears_model() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_model(&DeviceId::new("a._http._tcp.local."), model("BMM 101"));
        list.clear_all_telemetry();
        let dev = list.iter().next().expect("present");
        assert!(dev.model.is_none());
        assert!(!dev.reachable);
    }
}

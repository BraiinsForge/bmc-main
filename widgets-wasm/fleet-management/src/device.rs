// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::model::MinerModel;
use crate::telemetry::TelemetrySnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFamily {
    Bos,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "part of the generic device model; constructed once uBOS and Bitaxe adapters land"
        )
    )]
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

    pub fn iter_reachable(&self) -> impl Iterator<Item = &KnownDevice> {
        self.devices.iter().filter(|d| d.reachable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Rolling hashrate history for the dashboard chart and per-model sparklines.
//! One sample per telemetry cycle, gated on the telemetry sequence
//! so re-announcements and filter/params refolds add none. Departed devices
//! and model groups are pruned; the series caps to a recent window.

use std::collections::{HashMap, HashSet, VecDeque};

use bmc_wasm_sdk::Hashrate;

use crate::device::{DeviceFamily, DeviceId, DeviceList, KnownDevice};
use crate::summary::FleetSummary;

/// Samples kept per series — a recent window, not a wall-clock span.
const WINDOW: usize = 32;

#[derive(Debug, Default)]
struct Ring(VecDeque<f32>);

impl Ring {
    fn push(&mut self, value: f32) {
        if self.0.len() == WINDOW {
            self.0.pop_front();
        }
        self.0.push_back(value);
    }
}

/// Fleet-total, per-model, and per-device hashrate history.
/// Model series survive refolds via the (family, model-name) partition key;
/// device series key on the stable device id.
#[derive(Debug, Default)]
pub struct HashrateHistory {
    total: Ring,
    models: HashMap<(Option<usize>, String), Ring>,
    devices: HashMap<DeviceId, Ring>,
    last_seq: Option<u64>,
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "chart display values are fine at f32 precision"
)]
fn ths(value: Option<Hashrate>) -> f32 {
    value.map_or(0.0, |h| h.as_terahashes_per_second() as f32)
}

/// A device's current hashrate; unreachable folds to zero, like the summary.
fn device_hashrate(dev: &KnownDevice) -> f32 {
    if !dev.reachable {
        return 0.0;
    }
    dev.telemetry
        .as_ref()
        .and_then(|s| s.reading.current_hashrate_ths)
        .unwrap_or(0.0)
}

impl HashrateHistory {
    /// Prune series for departed devices and model groups, then append one sample
    /// per new telemetry cycle. Empty fleets are skipped so a series doesn't fill
    /// with zeros before any miner reports.
    pub(crate) fn record(
        &mut self,
        telemetry_seq: u64,
        summary: &FleetSummary,
        devices: &DeviceList,
    ) {
        self.prune_to(summary, devices);
        if self.last_seq == Some(telemetry_seq) || summary.groups.is_empty() {
            return;
        }
        self.last_seq = Some(telemetry_seq);
        self.total.push(ths(summary.total.hashrate));
        for g in &summary.groups {
            self.models
                .entry((g.family.map(DeviceFamily::index), g.label.clone()))
                .or_default()
                .push(ths(g.hashrate));
        }
        for dev in devices.iter() {
            self.devices
                .entry(dev.identity.id.clone())
                .or_default()
                .push(device_hashrate(dev));
        }
    }

    /// Drop series for devices and model groups no longer in the fleet;
    /// otherwise churning mDNS names grow the maps without bound over long uptime.
    fn prune_to(&mut self, summary: &FleetSummary, devices: &DeviceList) {
        let live_devices: HashSet<&DeviceId> = devices.iter().map(|d| &d.identity.id).collect();
        self.devices.retain(|id, _| live_devices.contains(id));
        let live_models: HashSet<(Option<usize>, &str)> = summary
            .groups
            .iter()
            .map(|g| (g.family.map(DeviceFamily::index), g.label.as_str()))
            .collect();
        self.models
            .retain(|(fam, label), _| live_models.contains(&(*fam, label.as_str())));
    }

    #[must_use]
    pub(crate) fn total_series(&self) -> Vec<f32> {
        self.total.0.iter().copied().collect()
    }

    #[must_use]
    pub(crate) fn model_series(&self, family: Option<DeviceFamily>, label: &str) -> Vec<f32> {
        self.models
            .get(&(family.map(DeviceFamily::index), label.to_owned()))
            .map_or_else(Vec::new, |ring| ring.0.iter().copied().collect())
    }

    #[must_use]
    pub(crate) fn device_series(&self, id: &DeviceId) -> Vec<f32> {
        self.devices
            .get(id)
            .map_or_else(Vec::new, |ring| ring.0.iter().copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::{ElectricPower, MiningEfficiency, Temperature};

    fn group(label: &str, hashrate: f64) -> crate::summary::GroupSummary {
        crate::summary::GroupSummary {
            label: label.to_owned(),
            family: Some(DeviceFamily::Bos),
            hashrate: Some(Hashrate::from_terahashes_per_second(hashrate)),
            power: Some(ElectricPower::from_watts(1.0)),
            efficiency: Some(MiningEfficiency::from_joules_per_terahash(1.0)),
            min_temperature: Some(Temperature::from_celsius(1.0)),
            avg_temperature: Some(Temperature::from_celsius(1.0)),
            max_temperature: Some(Temperature::from_celsius(1.0)),
            total_count: 1,
            ok_count: 1,
            off_count: 0,
            auth_error_count: 0,
        }
    }

    fn summary(seq_total: f64, m: f64) -> FleetSummary {
        FleetSummary {
            total: group("Total", seq_total),
            groups: vec![group("BOS BMM", m)],
        }
    }

    #[test]
    fn records_one_sample_per_new_sequence() {
        let mut h = HashrateHistory::default();
        h.record(1, &summary(10.0, 4.0), &DeviceList::new());
        h.record(1, &summary(99.0, 99.0), &DeviceList::new()); // same seq — ignored
        h.record(2, &summary(12.0, 5.0), &DeviceList::new());
        assert_eq!(h.total_series(), vec![10.0, 12.0]);
        assert_eq!(
            h.model_series(Some(DeviceFamily::Bos), "BOS BMM"),
            vec![4.0, 5.0]
        );
    }

    #[test]
    fn empty_fleet_is_not_recorded() {
        let mut h = HashrateHistory::default();
        h.record(
            1,
            &FleetSummary {
                total: group("Total", 0.0),
                groups: vec![],
            },
            &DeviceList::new(),
        );
        assert!(h.total_series().is_empty());
    }

    #[test]
    fn the_window_keeps_only_the_most_recent_samples() {
        let mut h = HashrateHistory::default();
        let mut seq = 0_u64;
        let mut value = 0.0_f64;
        for _ in 0..(WINDOW + 5) {
            seq += 1;
            h.record(seq, &summary(value, 0.0), &DeviceList::new());
            value += 1.0;
        }
        let series = h.total_series();
        assert_eq!(series.len(), WINDOW);
        assert!(
            (series[0] - 5.0).abs() < 1e-6,
            "the oldest five samples were dropped"
        );
    }

    #[test]
    fn an_unknown_model_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(h.model_series(Some(DeviceFamily::Bos), "nope").is_empty());
    }

    fn devices(specs: &[(&str, f32, bool)]) -> DeviceList {
        use crate::device::{DeviceIdentity, DeviceSource};
        use crate::telemetry::TelemetryReading;
        let mut list = DeviceList::new();
        for (name, hashrate, reachable) in specs {
            let id = DeviceId::new(*name);
            list.upsert(DeviceIdentity {
                id: id.clone(),
                family: DeviceFamily::Bos,
                name: (*name).to_owned(),
                host: "10.0.0.1".to_owned(),
                port: 80,
                source: DeviceSource::Discovered,
            });
            list.apply_telemetry(
                &id,
                TelemetryReading {
                    current_hashrate_ths: Some(*hashrate),
                    ..TelemetryReading::default()
                },
                *reachable,
            );
        }
        list
    }

    #[test]
    fn records_a_per_device_series() {
        let mut h = HashrateHistory::default();
        let a = DeviceId::new("bos/a");
        h.record(1, &summary(10.0, 4.0), &devices(&[("bos/a", 3.0, true)]));
        h.record(2, &summary(12.0, 5.0), &devices(&[("bos/a", 4.0, true)]));
        assert_eq!(h.device_series(&a), vec![3.0, 4.0]);
    }

    #[test]
    fn per_device_series_zeros_an_unreachable_device() {
        let mut h = HashrateHistory::default();
        let down = DeviceId::new("bos/b");
        h.record(
            1,
            &summary(10.0, 4.0),
            &devices(&[("bos/a", 3.0, true), ("bos/b", 9.0, false)]),
        );
        assert_eq!(
            h.device_series(&down),
            vec![0.0],
            "unreachable folds to zero"
        );
    }

    #[test]
    fn an_unknown_device_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(h.device_series(&DeviceId::new("bos/nope")).is_empty());
    }

    #[test]
    fn a_departed_device_is_pruned_from_history() {
        let mut h = HashrateHistory::default();
        h.record(1, &summary(10.0, 4.0), &devices(&[("bos/a", 3.0, true)]));
        assert_eq!(h.device_series(&DeviceId::new("bos/a")), vec![3.0]);
        // The next cycle no longer lists bos/a — its series is dropped.
        h.record(2, &summary(10.0, 4.0), &devices(&[("bos/b", 5.0, true)]));
        assert!(h.device_series(&DeviceId::new("bos/a")).is_empty());
        assert_eq!(h.device_series(&DeviceId::new("bos/b")), vec![5.0]);
    }

    #[test]
    fn a_vanished_model_group_is_pruned_from_history() {
        let mut h = HashrateHistory::default();
        h.record(1, &summary(10.0, 4.0), &DeviceList::new()); // group "BOS BMM"
        assert_eq!(
            h.model_series(Some(DeviceFamily::Bos), "BOS BMM"),
            vec![4.0]
        );
        // A later cycle summarizes a different model; the old group drops out.
        let other = FleetSummary {
            total: group("Total", 7.0),
            groups: vec![group("BOS BFM", 7.0)],
        };
        h.record(2, &other, &DeviceList::new());
        assert!(
            h.model_series(Some(DeviceFamily::Bos), "BOS BMM")
                .is_empty()
        );
        assert_eq!(
            h.model_series(Some(DeviceFamily::Bos), "BOS BFM"),
            vec![7.0]
        );
    }
}

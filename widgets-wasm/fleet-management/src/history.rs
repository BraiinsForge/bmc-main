// Copyright (C) 2026  Braiins Systems s.r.o.

//! Rolling hashrate history for the dashboard chart and per-model sparklines.
//! Each chart range (15 min … 24 h) is a consolidation tier: [`POINTS`] samples
//! taken at range/POINTS, all filled continuously. Picking a range just reads
//! the matching tier through [`HistoryView`] — nothing rebuilds. Departed
//! devices and model groups are pruned.

use std::collections::{HashMap, HashSet, VecDeque};

use bmc_wasm_sdk::Hashrate;

use crate::device::{DeviceFamily, DeviceId, DeviceList, KnownDevice};
use crate::summary::FleetSummary;

/// Samples kept per tier. Each range spreads across this many points, so a
/// tier's interval is `range / POINTS`; a fixed count bounds memory and the
/// number of points a chart draws, whatever range is chosen.
const POINTS: usize = 60;

/// The chart ranges in minutes — one consolidation tier each. Must match the
/// `chart_span_minutes` manifest param's options.
pub(crate) const TIER_MINUTES: [i32; 4] = [15, 60, 360, 1_440];

/// Sample interval (seconds) for a `span_minutes` range over [`POINTS`] points,
/// floored at 1 s so a tiny range can't stall the gate.
#[expect(
    clippy::integer_division,
    reason = "the interval is whole seconds; sub-second precision is irrelevant to the chart cadence"
)]
fn interval_secs(span_minutes: i32) -> i64 {
    let points = i64::try_from(POINTS).expect("BUG: POINTS fits in i64");
    (i64::from(span_minutes) * 60 / points).max(1)
}

#[derive(Debug, Default)]
struct Ring(VecDeque<f32>);

impl Ring {
    fn push(&mut self, value: f32) {
        if self.0.len() == POINTS {
            self.0.pop_front();
        }
        self.0.push_back(value);
    }

    fn to_vec(&self) -> Vec<f32> {
        self.0.iter().copied().collect()
    }
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

/// One consolidation tier: every series sampled at this tier's interval.
/// Model series survive refolds via the (family, model-name) partition key;
/// device series key on the stable device id.
#[derive(Debug, Default)]
struct Tier {
    interval_secs: i64,
    last_sample_at: Option<i64>,
    total: Ring,
    models: HashMap<(Option<usize>, String), Ring>,
    devices: HashMap<DeviceId, Ring>,
}

impl Tier {
    /// Prune departed series, then append one sample once this tier's interval
    /// has elapsed since the last (`now` is unix seconds). Empty fleets are
    /// skipped so a series doesn't fill with zeros before any miner reports.
    fn record(
        &mut self,
        now: i64,
        summary: &FleetSummary,
        devices: &DeviceList,
        live_devices: &HashSet<&DeviceId>,
        live_models: &HashSet<(Option<usize>, &str)>,
    ) {
        self.devices.retain(|id, _| live_devices.contains(id));
        self.models
            .retain(|(fam, label), _| live_models.contains(&(*fam, label.as_str())));
        if summary.groups.is_empty() {
            return;
        }
        if let Some(last) = self.last_sample_at
            && now - last < self.interval_secs
        {
            return;
        }
        self.last_sample_at = Some(now);
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
}

/// Fleet-total, per-model, and per-device hashrate history, one tier per chart
/// range. Read through [`HashrateHistory::view`].
#[derive(Debug)]
pub struct HashrateHistory {
    tiers: [Tier; TIER_MINUTES.len()],
}

impl Default for HashrateHistory {
    fn default() -> Self {
        Self {
            tiers: core::array::from_fn(|i| Tier {
                interval_secs: interval_secs(TIER_MINUTES[i]),
                ..Tier::default()
            }),
        }
    }
}

impl HashrateHistory {
    /// Feed one fold into every tier; each appends only if its own interval has
    /// elapsed. `now` is unix seconds.
    pub(crate) fn record(&mut self, now: i64, summary: &FleetSummary, devices: &DeviceList) {
        let live_devices: HashSet<&DeviceId> = devices.iter().map(|d| &d.identity.id).collect();
        let live_models: HashSet<(Option<usize>, &str)> = summary
            .groups
            .iter()
            .map(|g| (g.family.map(DeviceFamily::index), g.label.as_str()))
            .collect();
        for tier in &mut self.tiers {
            tier.record(now, summary, devices, &live_devices, &live_models);
        }
    }

    /// A read-only view at `span_minutes` — the tier serving that range,
    /// falling back to the shortest tier for an unknown range.
    #[must_use]
    pub(crate) fn view(&self, span_minutes: i32) -> HistoryView<'_> {
        let idx = TIER_MINUTES
            .iter()
            .position(|&m| m == span_minutes)
            .unwrap_or(0);
        HistoryView {
            tier: &self.tiers[idx],
        }
    }
}

/// A read-only view of the history at one chart range. The view builders read
/// series through this, so they never need to know which tier they draw.
#[derive(Debug)]
pub struct HistoryView<'a> {
    tier: &'a Tier,
}

impl HistoryView<'_> {
    #[must_use]
    pub(crate) fn total_series(&self) -> Vec<f32> {
        self.tier.total.to_vec()
    }

    #[must_use]
    pub(crate) fn model_series(&self, family: Option<DeviceFamily>, label: &str) -> Vec<f32> {
        self.tier
            .models
            .get(&(family.map(DeviceFamily::index), label.to_owned()))
            .map_or_else(Vec::new, Ring::to_vec)
    }

    #[must_use]
    pub(crate) fn device_series(&self, id: &DeviceId) -> Vec<f32> {
        self.tier
            .devices
            .get(id)
            .map_or_else(Vec::new, Ring::to_vec)
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
    fn records_one_sample_per_tier_interval() {
        // The 15-minute tier samples once every 15 s.
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        h.record(14, &summary(99.0, 99.0), &DeviceList::new()); // < 15 s — ignored
        h.record(15, &summary(12.0, 5.0), &DeviceList::new());
        assert_eq!(h.view(15).total_series(), vec![10.0, 12.0]);
        assert_eq!(
            h.view(15).model_series(Some(DeviceFamily::Bos), "BOS BMM"),
            vec![4.0, 5.0]
        );
    }

    #[test]
    fn each_tier_samples_at_its_own_rate() {
        // Over 60 s the 15-min tier (15 s) takes 5 points; the 1-h tier (60 s) 2.
        let mut h = HashrateHistory::default();
        let mut now = 0_i64;
        while now <= 60 {
            h.record(now, &summary(1.0, 1.0), &DeviceList::new());
            now += 1;
        }
        assert_eq!(h.view(15).total_series().len(), 5); // 0, 15, 30, 45, 60
        assert_eq!(h.view(60).total_series().len(), 2); // 0, 60
    }

    #[test]
    fn empty_fleet_is_not_recorded() {
        let mut h = HashrateHistory::default();
        h.record(
            0,
            &FleetSummary {
                total: group("Total", 0.0),
                groups: vec![],
            },
            &DeviceList::new(),
        );
        assert!(h.view(15).total_series().is_empty());
    }

    #[test]
    fn the_window_keeps_only_the_most_recent_samples() {
        let mut h = HashrateHistory::default();
        let mut now = 0_i64;
        let mut value = 0.0_f64;
        for _ in 0..(POINTS + 5) {
            h.record(now, &summary(value, 0.0), &DeviceList::new());
            now += 15; // one sample per 15-min-tier interval
            value += 1.0;
        }
        let series = h.view(15).total_series();
        assert_eq!(series.len(), POINTS);
        assert!(
            (series[0] - 5.0).abs() < 1e-6,
            "the oldest five samples were dropped"
        );
    }

    #[test]
    fn an_unknown_model_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(
            h.view(15)
                .model_series(Some(DeviceFamily::Bos), "nope")
                .is_empty()
        );
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
        h.record(0, &summary(10.0, 4.0), &devices(&[("bos/a", 3.0, true)]));
        h.record(15, &summary(12.0, 5.0), &devices(&[("bos/a", 4.0, true)]));
        assert_eq!(h.view(15).device_series(&a), vec![3.0, 4.0]);
    }

    #[test]
    fn per_device_series_zeros_an_unreachable_device() {
        let mut h = HashrateHistory::default();
        let down = DeviceId::new("bos/b");
        h.record(
            0,
            &summary(10.0, 4.0),
            &devices(&[("bos/a", 3.0, true), ("bos/b", 9.0, false)]),
        );
        assert_eq!(
            h.view(15).device_series(&down),
            vec![0.0],
            "unreachable folds to zero"
        );
    }

    #[test]
    fn an_unknown_device_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(
            h.view(15)
                .device_series(&DeviceId::new("bos/nope"))
                .is_empty()
        );
    }

    #[test]
    fn a_departed_device_is_pruned_from_history() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &devices(&[("bos/a", 3.0, true)]));
        assert_eq!(h.view(15).device_series(&DeviceId::new("bos/a")), vec![3.0]);
        // The next cycle no longer lists bos/a — its series is dropped.
        h.record(15, &summary(10.0, 4.0), &devices(&[("bos/b", 5.0, true)]));
        assert!(h.view(15).device_series(&DeviceId::new("bos/a")).is_empty());
        assert_eq!(h.view(15).device_series(&DeviceId::new("bos/b")), vec![5.0]);
    }

    #[test]
    fn a_vanished_model_group_is_pruned_from_history() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new()); // group "BOS BMM"
        assert_eq!(
            h.view(15).model_series(Some(DeviceFamily::Bos), "BOS BMM"),
            vec![4.0]
        );
        // A later cycle summarizes a different model; the old group drops out.
        let other = FleetSummary {
            total: group("Total", 7.0),
            groups: vec![group("BOS BFM", 7.0)],
        };
        h.record(15, &other, &DeviceList::new());
        assert!(
            h.view(15)
                .model_series(Some(DeviceFamily::Bos), "BOS BMM")
                .is_empty()
        );
        assert_eq!(
            h.view(15).model_series(Some(DeviceFamily::Bos), "BOS BFM"),
            vec![7.0]
        );
    }
}

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

//! Rolling hashrate history for the dashboard chart and per-model sparklines.
//! Each chart range (15 min … 24 h) is a consolidation tier: [`POINTS`] samples
//! taken at range/POINTS, all filled continuously. Picking a range just reads
//! the matching tier through [`HistoryView`] — nothing rebuilds. Each sample
//! carries its timestamp and a nullable value, so a restart gap or an
//! unreachable device draws as a break, not a false line to zero.
//!
//! The fleet-total and per-model tiers persist to the flash cache so the charts
//! survive a restart; per-device series are RAM-only, so a reused hostname can't
//! attribute one miner's past to another. A corrupt blob is dropped, not shown.

use std::collections::{HashMap, HashSet, VecDeque};

use bmc_wasm_sdk::types::Hashrate;

use crate::device::{DeviceFamily, DeviceId, DeviceList, KnownDevice};
use crate::summary::FleetSummary;

/// Samples kept per tier. Each range spreads across this many points, so a
/// tier's interval is `range / POINTS`; a fixed count bounds memory and the
/// number of points a chart draws, whatever range is chosen.
const POINTS: usize = 60;

/// A chart range — one consolidation tier each. Mirrors the `chart_span_minutes`
/// manifest options, but stays independent of that generated enum (wasm32-only);
/// the `From` impl below is the one checked crossing between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChartSpan {
    FifteenMinutes,
    OneHour,
    SixHours,
    TwentyFourHours,
}

impl ChartSpan {
    /// Every span, in tier order — one tier is built per entry.
    pub(crate) const ALL: [Self; 4] = [
        Self::FifteenMinutes,
        Self::OneHour,
        Self::SixHours,
        Self::TwentyFourHours,
    ];

    /// The range this span covers.
    pub(crate) const fn minutes(self) -> i32 {
        match self {
            Self::FifteenMinutes => 15,
            Self::OneHour => 60,
            Self::SixHours => 360,
            Self::TwentyFourHours => 1_440,
        }
    }

    /// This span's index into [`Self::ALL`], and so into the tier array.
    const fn tier(self) -> usize {
        match self {
            Self::FifteenMinutes => 0,
            Self::OneHour => 1,
            Self::SixHours => 2,
            Self::TwentyFourHours => 3,
        }
    }
}

/// The one crossing from the generated manifest enum to the tiers, and exhaustive
/// by design: a new `chart_span_minutes` option won't compile until it has a tier
/// here, so the two lists can never silently diverge.
#[cfg(target_arch = "wasm32")]
impl From<crate::manifest_params::ChartSpanMinutes> for ChartSpan {
    fn from(span: crate::manifest_params::ChartSpanMinutes) -> Self {
        use crate::manifest_params::ChartSpanMinutes;
        match span {
            ChartSpanMinutes::_15Minutes => Self::FifteenMinutes,
            ChartSpanMinutes::_1Hour => Self::OneHour,
            ChartSpanMinutes::_6Hours => Self::SixHours,
            ChartSpanMinutes::_24Hours => Self::TwentyFourHours,
        }
    }
}

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

/// One hashrate point: when it was taken (unix seconds) and its value, or `None`
/// for no data — an unreachable device, or the gap a restart leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, bincode::Encode, bincode::Decode)]
pub struct HistoryDatum {
    pub at: i64,
    pub value: Option<f32>,
}

/// The time span a chart draws: the right edge (`end`, render now in unix
/// seconds) back `span_secs`. Points place by timestamp within it, so a
/// still-filling history hugs the right edge rather than stretching to fill the
/// width, and a stale series leaves an honest gap on the right.
#[derive(Debug, Clone, Copy)]
pub struct ChartWindow {
    pub end: i64,
    pub span_secs: i64,
}

impl ChartWindow {
    /// The window that exactly spans a series — for fixtures and stories, whose
    /// baked points should fill the whole width.
    #[must_use]
    pub fn covering(series: &[HistoryDatum]) -> Self {
        let start = series.first().map_or(0, |s| s.at);
        let end = series.last().map_or(0, |s| s.at);
        Self {
            end,
            span_secs: (end - start).max(1),
        }
    }
}

#[derive(Debug, Default)]
struct Ring(VecDeque<HistoryDatum>);

impl Ring {
    /// A ring of restored samples, keeping only the newest [`POINTS`].
    /// The blob is decoded, not trusted: a longer series would sit over capacity
    /// forever, since evicting one per push holds the length rather than lowering it.
    fn restored(samples: Vec<HistoryDatum>) -> Self {
        let excess = samples.len().saturating_sub(POINTS);
        Self(samples.into_iter().skip(excess).collect())
    }

    fn push(&mut self, sample: HistoryDatum) {
        if self.0.len() >= POINTS {
            self.0.pop_front();
        }
        self.0.push_back(sample);
    }

    /// Start a new bucket at a slot boundary, else refresh the current bucket's
    /// value in place (keeping its timestamp) so the newest point tracks the
    /// live reading without over-sampling the ring.
    fn upsert(&mut self, at: i64, value: Option<f32>, new_slot: bool) {
        match self.0.back_mut() {
            Some(last) if !new_slot => last.value = value,
            _ => self.push(HistoryDatum { at, value }),
        }
    }

    /// End the series with a valueless sample, so whatever follows
    /// starts a new run rather than joining the last one.
    /// Only ever breaks after real data: a series that is empty, or already
    /// ends on a break, would otherwise spend a slot on every restart.
    fn mark_break(&mut self, at: i64) {
        if self.0.back().is_some_and(|last| last.value.is_some()) {
            self.push(HistoryDatum { at, value: None });
        }
    }

    fn to_vec(&self) -> Vec<HistoryDatum> {
        self.0.iter().copied().collect()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "chart display values are fine at f32 precision"
)]
fn ths(h: Hashrate) -> f32 {
    h.as_terahashes_per_second() as f32
}

/// A device's current hashrate, or `None` when it is unreachable or its reading
/// carries no hashrate field — the chart then breaks rather than dropping a line
/// to a hashrate the device isn't actually reporting.
fn device_hashrate(dev: &KnownDevice) -> Option<f32> {
    if !dev.reachable {
        return None;
    }
    dev.telemetry
        .as_ref()
        .and_then(|s| s.reading.current_hashrate_ths)
}

/// One consolidation tier: every series sampled at this tier's interval.
/// Model series survive refolds via the (family, model-name) partition key;
/// device series key on the stable device id.
#[derive(Debug, Default)]
struct Tier {
    interval_secs: i64,
    current_slot: Option<i64>,
    total: Ring,
    models: HashMap<(Option<usize>, String), Ring>,
    devices: HashMap<DeviceId, Ring>,
}

impl Tier {
    /// Prune departed series, then fold the reading into this tier: a new
    /// interval slot starts a fresh bucket, an ongoing one refreshes its value
    /// in place (`now` is unix seconds). Empty fleets are skipped so a series
    /// doesn't fill with zeros before any miner reports.
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
        #[expect(
            clippy::integer_division,
            reason = "the slot index buckets whole seconds into this tier's interval"
        )]
        let slot = now / self.interval_secs;
        let new_slot = self.current_slot != Some(slot);
        self.current_slot = Some(slot);
        self.total
            .upsert(now, summary.total.hashrate.map(ths), new_slot);
        for g in &summary.groups {
            self.models
                .entry((g.family.map(DeviceFamily::index), g.label.clone()))
                .or_default()
                .upsert(now, g.hashrate.map(ths), new_slot);
        }
        for dev in devices.iter() {
            self.devices
                .entry(dev.identity.id.clone())
                .or_default()
                .upsert(now, device_hashrate(dev), new_slot);
        }
    }
}

/// Fleet-total, per-model, and per-device hashrate history, one tier per chart
/// range. Read through [`HashrateHistory::view`].
#[derive(Debug)]
pub struct HashrateHistory {
    tiers: [Tier; ChartSpan::ALL.len()],
    last_snapshot_at: Option<i64>,
}

impl Default for HashrateHistory {
    fn default() -> Self {
        Self {
            tiers: core::array::from_fn(|i| Tier {
                interval_secs: interval_secs(ChartSpan::ALL[i].minutes()),
                ..Tier::default()
            }),
            last_snapshot_at: None,
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

    /// A read-only view at `span` — the tier serving that range.
    #[must_use]
    pub(crate) fn view(&self, span: ChartSpan) -> HistoryView<'_> {
        HistoryView {
            tier: &self.tiers[span.tier()],
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
    pub(crate) fn total_series(&self) -> Vec<HistoryDatum> {
        self.tier.total.to_vec()
    }

    #[must_use]
    pub(crate) fn model_series(
        &self,
        family: Option<DeviceFamily>,
        label: &str,
    ) -> Vec<HistoryDatum> {
        self.tier
            .models
            .get(&(family.map(DeviceFamily::index), label.to_owned()))
            .map_or_else(Vec::new, Ring::to_vec)
    }

    #[must_use]
    pub(crate) fn device_series(&self, id: &DeviceId) -> Vec<HistoryDatum> {
        self.tier
            .devices
            .get(id)
            .map_or_else(Vec::new, Ring::to_vec)
    }
}

// ── Flash persistence (aggregate tiers only) ─────────────────────────

/// Persist at most this often (seconds); a crash loses at most this much.
const SNAPSHOT_INTERVAL_SECS: i64 = 300;

/// Blob header: a magic byte and a format version. A mismatch on read means a
/// stale or foreign blob, so it's dropped rather than decoded.
const SNAPSHOT_MAGIC: u8 = 0xF1;
const SNAPSHOT_VERSION: u8 = 2;

/// Flash cache tag for the persisted history blob. Only the wasm build reads or
/// writes the cache; the snapshot codec itself is exercised natively in tests.
#[cfg(target_arch = "wasm32")]
pub(crate) const CACHE_TAG: &str = "hashrate_history";

/// The persisted shape of the total + per-model tiers (per-device is RAM-only).
#[derive(bincode::Encode, bincode::Decode)]
struct TierSnapshot {
    total: Vec<HistoryDatum>,
    /// `(family index, model label, series)` — a `Vec`, not a map, to stay
    /// `alloc`-only for bincode.
    models: Vec<(Option<u32>, String, Vec<HistoryDatum>)>,
}

impl HashrateHistory {
    /// A blob of the total + per-model tiers for the flash cache, if the
    /// snapshot interval has elapsed since the last — otherwise `None`. `now`
    /// is unix seconds.
    pub(crate) fn take_snapshot(&mut self, now: i64) -> Option<Vec<u8>> {
        // Nothing recorded yet — don't spend the first-snapshot slot on an empty
        // history (which would then rate-limit real data out for a whole
        // interval); the first blob written should carry the first samples.
        if self.tiers.iter().all(|t| t.total.is_empty()) {
            return None;
        }
        if let Some(last) = self.last_snapshot_at
            && now - last < SNAPSHOT_INTERVAL_SECS
        {
            return None;
        }
        self.last_snapshot_at = Some(now);
        let tiers: Vec<TierSnapshot> = self.tiers.iter().map(Tier::to_snapshot).collect();
        // Encoding failure here would be a logic bug, not bad input, so no
        // snapshot is the safe fallback.
        let Ok(mut body) = bincode::encode_to_vec(&tiers, bincode::config::standard()) else {
            return None;
        };
        let mut blob = vec![SNAPSHOT_MAGIC, SNAPSHOT_VERSION];
        blob.append(&mut body);
        Some(blob)
    }

    /// Restore the total + per-model tiers from a `saved_at`-stamped blob,
    /// dropping any tier whose downtime already covers its whole range (its
    /// points would all be off-screen). Returns `false` on a malformed blob so
    /// the caller can evict it; per-device series are never restored.
    pub(crate) fn restore(&mut self, bytes: &[u8], saved_at: i64, now: i64) -> bool {
        let Some(payload) = bytes.strip_prefix(&[SNAPSHOT_MAGIC, SNAPSHOT_VERSION]) else {
            return false;
        };
        let Ok((tiers, _)) = bincode::decode_from_slice::<Vec<TierSnapshot>, _>(
            payload,
            bincode::config::standard(),
        ) else {
            return false;
        };
        if tiers.len() != self.tiers.len() {
            return false;
        }
        for (i, (tier, snap)) in self.tiers.iter_mut().zip(tiers).enumerate() {
            let range_secs = i64::from(ChartSpan::ALL[i].minutes()) * 60;
            if now.saturating_sub(saved_at) >= range_secs {
                continue; // fully stale — leave empty, refills live
            }
            tier.restore(snap, saved_at);
        }
        true
    }
}

impl Tier {
    fn to_snapshot(&self) -> TierSnapshot {
        TierSnapshot {
            total: self.total.to_vec(),
            models: self
                .models
                .iter()
                .map(|((fam, label), ring)| {
                    let fam = fam.and_then(|i| u32::try_from(i).ok());
                    (fam, label.clone(), ring.to_vec())
                })
                .collect(),
        }
    }

    fn restore(&mut self, snap: TierSnapshot, saved_at: i64) {
        // `current_slot` stays unset, so the first live fold after a restore
        // opens a fresh bucket instead of refreshing a restored one.
        // That is also what keeps the break below from being overwritten.
        self.total = Ring::restored(snap.total);
        self.models = snap
            .models
            .into_iter()
            .map(|(fam, label, series)| {
                let fam = fam.map(|i| i as usize);
                ((fam, label), Ring::restored(series))
            })
            .collect();
        // Close every restored series with an explicit break at the snapshot time.
        // Downtime is a gap in the data, and leaving the chart's spacing heuristic
        // to infer it fails exactly when it matters: until a few live samples land,
        // the outage is the only spacing there is, so the line runs straight through.
        self.total.mark_break(saved_at);
        for ring in self.models.values_mut() {
            ring.mark_break(saved_at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::types::{ElectricPower, MiningEfficiency, Temperature};

    /// The `value`s of a series, for terse assertions.
    fn vals(series: &[HistoryDatum]) -> Vec<Option<f32>> {
        series.iter().map(|s| s.value).collect()
    }

    #[test]
    fn each_span_reads_the_tier_built_for_it() {
        for (i, span) in ChartSpan::ALL.iter().enumerate() {
            assert_eq!(
                span.tier(),
                i,
                "{span:?} indexes tier {} but is ALL[{i}]",
                span.tier(),
            );
        }
    }

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
    fn records_one_bucket_per_tier_interval() {
        // The 15-min tier holds one bucket per 15 s; a second reading inside the
        // same interval refreshes that bucket in place, a new interval opens one.
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        h.record(14, &summary(99.0, 99.0), &DeviceList::new()); // same bucket — refreshes
        h.record(15, &summary(12.0, 5.0), &DeviceList::new());
        assert_eq!(
            vals(&h.view(ChartSpan::FifteenMinutes).total_series()),
            vec![Some(99.0), Some(12.0)]
        );
        assert_eq!(
            vals(
                &h.view(ChartSpan::FifteenMinutes)
                    .model_series(Some(DeviceFamily::Bos), "BOS BMM")
            ),
            vec![Some(99.0), Some(5.0)]
        );
    }

    #[test]
    fn the_open_bucket_keeps_its_timestamp_while_its_value_refreshes() {
        // A within-interval refresh updates the value but not the bucket's
        // timestamp, so points stay evenly spaced at the tier interval.
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        h.record(14, &summary(99.0, 4.0), &DeviceList::new());
        let series = h.view(ChartSpan::FifteenMinutes).total_series();
        assert_eq!(series.len(), 1, "still one bucket");
        assert_eq!(
            series[0].at, 0,
            "timestamp is the bucket's, not the refresh's"
        );
        assert_eq!(series[0].value, Some(99.0), "value is the latest reading");
    }

    #[test]
    fn samples_carry_their_timestamp() {
        let mut h = HashrateHistory::default();
        h.record(100, &summary(10.0, 4.0), &DeviceList::new());
        h.record(115, &summary(12.0, 5.0), &DeviceList::new());
        let ats: Vec<i64> = h
            .view(ChartSpan::FifteenMinutes)
            .total_series()
            .iter()
            .map(|s| s.at)
            .collect();
        assert_eq!(ats, vec![100, 115]);
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
        assert_eq!(h.view(ChartSpan::FifteenMinutes).total_series().len(), 5); // 0, 15, 30, 45, 60
        assert_eq!(h.view(ChartSpan::OneHour).total_series().len(), 2); // 0, 60
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
        assert!(h.view(ChartSpan::FifteenMinutes).total_series().is_empty());
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
        let series = h.view(ChartSpan::FifteenMinutes).total_series();
        assert_eq!(series.len(), POINTS);
        assert_eq!(series[0].value, Some(5.0), "the oldest five were dropped");
    }

    #[test]
    fn an_unknown_model_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(
            h.view(ChartSpan::FifteenMinutes)
                .model_series(Some(DeviceFamily::Bos), "nope")
                .is_empty()
        );
    }

    fn devices(specs: &[(&str, f32, bool)]) -> DeviceList {
        use crate::device::DeviceIdentity;
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
        assert_eq!(
            vals(&h.view(ChartSpan::FifteenMinutes).device_series(&a)),
            vec![Some(3.0), Some(4.0)]
        );
    }

    #[test]
    fn a_none_total_samples_a_gap_not_zero() {
        let mut h = HashrateHistory::default();
        let mut s = summary(10.0, 4.0);
        s.total.hashrate = None;
        s.groups[0].hashrate = None;
        h.record(0, &s, &DeviceList::new());
        assert_eq!(
            vals(&h.view(ChartSpan::FifteenMinutes).total_series()),
            vec![None],
            "a summary with no hashrate is a gap, matching the device-series policy"
        );
        assert_eq!(
            vals(
                &h.view(ChartSpan::FifteenMinutes)
                    .model_series(Some(DeviceFamily::Bos), "BOS BMM")
            ),
            vec![None]
        );
    }

    #[test]
    fn an_unreachable_device_samples_a_gap_not_zero() {
        let mut h = HashrateHistory::default();
        let down = DeviceId::new("bos/b");
        h.record(
            0,
            &summary(10.0, 4.0),
            &devices(&[("bos/a", 3.0, true), ("bos/b", 9.0, false)]),
        );
        assert_eq!(
            vals(&h.view(ChartSpan::FifteenMinutes).device_series(&down)),
            vec![None],
            "unreachable is a gap, not a false zero"
        );
    }

    #[test]
    fn an_unknown_device_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(
            h.view(ChartSpan::FifteenMinutes)
                .device_series(&DeviceId::new("bos/nope"))
                .is_empty()
        );
    }

    #[test]
    fn a_departed_device_is_pruned_from_history() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &devices(&[("bos/a", 3.0, true)]));
        assert_eq!(
            vals(
                &h.view(ChartSpan::FifteenMinutes)
                    .device_series(&DeviceId::new("bos/a"))
            ),
            vec![Some(3.0)]
        );
        // The next cycle no longer lists bos/a — its series is dropped.
        h.record(15, &summary(10.0, 4.0), &devices(&[("bos/b", 5.0, true)]));
        assert!(
            h.view(ChartSpan::FifteenMinutes)
                .device_series(&DeviceId::new("bos/a"))
                .is_empty()
        );
        assert_eq!(
            vals(
                &h.view(ChartSpan::FifteenMinutes)
                    .device_series(&DeviceId::new("bos/b"))
            ),
            vec![Some(5.0)]
        );
    }

    #[test]
    fn a_vanished_model_group_is_pruned_from_history() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new()); // group "BOS BMM"
        assert_eq!(
            vals(
                &h.view(ChartSpan::FifteenMinutes)
                    .model_series(Some(DeviceFamily::Bos), "BOS BMM")
            ),
            vec![Some(4.0)]
        );
        let other = FleetSummary {
            total: group("Total", 7.0),
            groups: vec![group("BOS BFM", 7.0)],
        };
        h.record(15, &other, &DeviceList::new());
        assert!(
            h.view(ChartSpan::FifteenMinutes)
                .model_series(Some(DeviceFamily::Bos), "BOS BMM")
                .is_empty()
        );
        assert_eq!(
            vals(
                &h.view(ChartSpan::FifteenMinutes)
                    .model_series(Some(DeviceFamily::Bos), "BOS BFM")
            ),
            vec![Some(7.0)]
        );
    }

    // ── Persistence ──────────────────────────────────────────────────

    #[test]
    fn snapshot_round_trips_total_and_model_series() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &devices(&[("bos/a", 3.0, true)]));
        h.record(15, &summary(12.0, 5.0), &devices(&[("bos/a", 4.0, true)]));
        let blob = h
            .take_snapshot(0)
            .expect("BUG: first snapshot always fires");

        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 15, 20), "fresh blob restores");
        // The trailing `None` is the restart break; live samples land after it.
        assert_eq!(
            vals(&restored.view(ChartSpan::FifteenMinutes).total_series()),
            vec![Some(10.0), Some(12.0), None]
        );
        assert_eq!(
            vals(
                &restored
                    .view(ChartSpan::FifteenMinutes)
                    .model_series(Some(DeviceFamily::Bos), "BOS BMM")
            ),
            vec![Some(4.0), Some(5.0), None]
        );
        // Per-device series are RAM-only: never persisted.
        assert!(
            restored
                .view(ChartSpan::FifteenMinutes)
                .device_series(&DeviceId::new("bos/a"))
                .is_empty()
        );
    }

    #[test]
    fn a_stale_tier_is_dropped_on_restore() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");
        let mut restored = HashrateHistory::default();
        // Downtime of 20 min: exceeds the 15-min tier's range (dropped) but not
        // the 1-h tier's (kept).
        let saved_at = 0;
        let now = 20 * 60;
        restored.restore(&blob, saved_at, now);
        assert!(
            restored
                .view(ChartSpan::FifteenMinutes)
                .total_series()
                .is_empty(),
            "15-min tier fully stale"
        );
        assert_eq!(
            vals(&restored.view(ChartSpan::OneHour).total_series()),
            vec![Some(10.0), None],
            "1-h tier still in range, closed by the restart break"
        );
    }

    #[test]
    fn a_restored_series_ends_on_a_break_the_live_samples_cannot_join() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");

        // Back an hour later: one cached point, then live ones.
        // The outage is the only spacing in the series, so a median-of-deltas
        // rule would take it for the norm and draw straight through it.
        let mut restored = HashrateHistory::default();
        let downtime = 55 * 60;
        assert!(
            restored.restore(&blob, 0, downtime),
            "1-h tier still in range"
        );
        restored.record(downtime, &summary(12.0, 5.0), &DeviceList::new());

        let series = restored.view(ChartSpan::OneHour).total_series();
        assert_eq!(
            vals(&series),
            vec![Some(10.0), None, Some(12.0)],
            "the break separates cached data from live"
        );
    }

    #[test]
    fn a_restored_series_longer_than_the_ring_is_cut_to_the_newest() {
        // A blob this long can't come from our own snapshot — corrupted flash,
        // or a build with a bigger ring — which is why the bound belongs on the
        // way in, before every later push inherits the excess.
        let over: Vec<HistoryDatum> = (0..POINTS + 10)
            .map(|i| HistoryDatum {
                at: i64::try_from(i).expect("BUG: loop index fits i64"),
                value: None,
            })
            .collect();
        let ring = Ring::restored(over);
        assert_eq!(ring.0.len(), POINTS, "cut to capacity");
        assert_eq!(
            ring.0.front().map(|d| d.at),
            Some(10),
            "the oldest ten were dropped, not the newest"
        );
    }

    #[test]
    fn a_corrupt_blob_is_rejected() {
        let mut h = HashrateHistory::default();
        assert!(!h.restore(b"", 0, 0), "empty blob");
        assert!(
            !h.restore(&[SNAPSHOT_MAGIC, 0xEE, 1, 2, 3], 0, 0),
            "bad version"
        );
        assert!(
            !h.restore(&[SNAPSHOT_MAGIC, SNAPSHOT_VERSION, 0xFF, 0xFF], 0, 0),
            "garbage payload"
        );
        assert!(
            h.view(ChartSpan::FifteenMinutes).total_series().is_empty(),
            "nothing restored"
        );
    }

    #[test]
    fn an_empty_history_is_not_snapshotted() {
        let mut h = HashrateHistory::default();
        assert!(
            h.take_snapshot(0).is_none(),
            "nothing recorded — no blob, and the first-snapshot slot is kept"
        );
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        assert!(
            h.take_snapshot(0).is_some(),
            "the first data-bearing snapshot fires"
        );
    }

    #[test]
    fn snapshot_is_rate_limited() {
        let mut h = HashrateHistory::default();
        h.record(0, &summary(10.0, 4.0), &DeviceList::new());
        assert!(h.take_snapshot(0).is_some(), "first snapshot fires");
        assert!(
            h.take_snapshot(SNAPSHOT_INTERVAL_SECS - 1).is_none(),
            "within the interval — skipped"
        );
        assert!(
            h.take_snapshot(SNAPSHOT_INTERVAL_SECS).is_some(),
            "after the interval — fires"
        );
    }
}

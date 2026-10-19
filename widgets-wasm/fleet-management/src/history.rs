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
//! Each chart range (15 min … 24 h) is a consolidation tier of [`POINTS`] buckets,
//! one per range/POINTS slot, for the fleet total, each model and each device.
//! Picking a range just reads the matching tier through [`HistoryView`] — nothing rebuilds.
//! Each sample carries its timestamp and a nullable value, so an unreachable device draws
//! as a break, not a false line to zero; a restart longer than two slots leaves a gap.
//!
//! The fleet-total and per-model tiers persist to the flash cache so the charts
//! survive a restart; per-device series are RAM-only, so a reused hostname can't
//! attribute one miner's past to another. A corrupt blob is dropped, not shown.

use std::collections::HashMap;

use bmc_wasm_sdk::types::Hashrate;

use crate::device::{DeviceFamily, DeviceId};
use crate::summary::{FleetHashrate, UNKNOWN_GROUP};

mod series;

pub use series::ChartPoint;
use series::Series;

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

    pub(crate) fn range_secs(self) -> i64 {
        i64::from(self.minutes()) * 60
    }

    #[expect(
        clippy::integer_division,
        reason = "every range is whole minutes, so it splits evenly into POINTS whole seconds"
    )]
    pub(crate) fn interval_secs(self) -> i64 {
        let points = i64::try_from(POINTS).expect("BUG: POINTS fits in i64");
        self.range_secs() / points
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

/// One hashrate point: when it was taken (unix seconds) and its value, or `None`
/// for no data — an unreachable device, or a reading without a hashrate.
/// Snapshots from earlier builds also hold `None` restart breaks.
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
    pub fn covering(series: &[Option<ChartPoint>]) -> Self {
        let mut ats = series.iter().flatten().map(|p| p.at);
        let start = ats.next().unwrap_or(0);
        let end = ats.next_back().unwrap_or(start);
        Self {
            end,
            span_secs: (end - start).max(1),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "chart display values are fine at f32 precision"
)]
fn ths(h: Hashrate) -> f32 {
    h.as_terahashes_per_second() as f32
}

/// The fleet's consolidation tier for one chart range: the total and model series,
/// each model's keyed on the (family, model name) pair the fleet is grouped by.
#[derive(Debug)]
struct FleetTier {
    span: ChartSpan,
    total: Series,
    models: HashMap<(Option<usize>, String), Series>,
}

impl FleetTier {
    fn new(span: ChartSpan) -> Self {
        Self {
            span,
            total: Series::new(span),
            models: HashMap::new(),
        }
    }

    /// Record the reading into the total and each model's series (`now` is unix seconds).
    fn record(&mut self, now: i64, fleet: &FleetHashrate<'_>) {
        self.total.record(now, fleet.total.map(ths));
        let span = self.span;
        for g in &fleet.groups {
            self.models
                .entry((g.family.map(DeviceFamily::index), g.label.to_owned()))
                .or_insert_with(|| Series::new(span))
                .record(now, g.hashrate.map(ths));
        }
    }
}

/// Fleet-total, per-model, and per-device hashrate history, one tier per chart
/// range. Read through [`HashrateHistory::view`].
#[derive(Debug)]
pub struct HashrateHistory {
    fleet: [FleetTier; ChartSpan::ALL.len()],
    /// Each device's series in every tier, keyed on the stable device id
    /// and held in one entry so a poll hashes the id once.
    /// A device's series goes when it leaves the fleet.
    devices: HashMap<DeviceId, [Series; ChartSpan::ALL.len()]>,
    last_snapshot_at: Option<i64>,
    flash: Flash,
    /// The last clock reading the history has followed.
    /// A reading before it means the clock went back.
    latest_clock: Option<i64>,
    /// A restored blob newer than the clock, kept in flash and out of the tiers
    /// until the clock reaches it.
    pending: Option<PendingBlob>,
}

impl Default for HashrateHistory {
    fn default() -> Self {
        Self {
            fleet: ChartSpan::ALL.map(FleetTier::new),
            devices: HashMap::new(),
            last_snapshot_at: None,
            flash: Flash::Saved,
            latest_clock: None,
            pending: None,
        }
    }
}

impl HashrateHistory {
    /// Feed the fleet total and per-model hashrate into every tier,
    /// each series bucketing it by the tier's interval. `now` is unix seconds.
    /// Empty fleets are skipped, so a reading taken before discovery opens no bucket.
    ///
    /// A model series is dropped only once its newest sample has left the tier's range,
    /// not when its model drops out of the reading:
    /// after a restart the restored series wait for discovery to find their models again.
    /// The [`UNKNOWN_GROUP`] series goes as soon as it is absent,
    /// as it holds whichever devices had no model yet, not one model's history.
    pub(crate) fn record_fleet(&mut self, now: i64, fleet: &FleetHashrate<'_>) {
        self.follow_clock(now);
        let unknown_present = fleet.groups.iter().any(|g| g.label == UNKNOWN_GROUP);
        let mut pruned = false;
        for tier in &mut self.fleet {
            let range_secs = tier.span.range_secs();
            let before = tier.models.len();
            tier.models.retain(|(_, label), series| {
                let in_range = series
                    .newest_at()
                    .is_some_and(|at| now.saturating_sub(at) < range_secs);
                in_range && (unknown_present || label != UNKNOWN_GROUP)
            });
            pruned |= tier.models.len() != before;
        }
        if !fleet.groups.is_empty() {
            for tier in &mut self.fleet {
                tier.record(now, fleet);
            }
        }
        if (pruned || !fleet.groups.is_empty()) && self.flash == Flash::Saved {
            self.flash = Flash::Behind;
        }
    }

    /// Whether some tier has entered a slot that [`Self::record_fleet`] has not sampled yet.
    #[must_use]
    pub(crate) fn fleet_sample_due(&self, now: i64) -> bool {
        self.fleet
            .iter()
            .any(|tier| !tier.total.has_sampled_slot_of(now))
    }

    /// Record `value`, the hashrate of the device `id`, into its series in every tier.
    pub(crate) fn record_device(&mut self, now: i64, id: &DeviceId, value: Option<f32>) {
        self.follow_clock(now);
        let tiers = self
            .devices
            .entry(id.clone())
            .or_insert_with(|| ChartSpan::ALL.map(Series::new));
        for series in tiers {
            series.record(now, value);
        }
    }

    /// Bring the history in line with a clock reading of `now` before recording at it.
    /// A clock that went back was corrected, so every bucket stamped after `now`,
    /// in the tiers and in a waiting blob, is wrong and goes, from flash too.
    /// A waiting blob merges once the clock has reached it.
    fn follow_clock(&mut self, now: i64) {
        if self.latest_clock.is_some_and(|latest| now < latest) {
            self.drop_after(now);
        }
        self.latest_clock = Some(now);
        if let Some(pending) = self
            .pending
            .take_if(|pending| pending.newest.is_none_or(|newest| newest <= now))
        {
            self.merge(pending, now);
        }
    }

    fn drop_after(&mut self, now: i64) {
        for tier in &mut self.fleet {
            tier.drop_after(now);
        }
        for tiers in self.devices.values_mut() {
            for series in tiers {
                series.drop_after(now);
            }
        }
        if self
            .pending
            .as_mut()
            .is_some_and(|pending| !pending.drop_after(now))
        {
            self.pending = None;
        }
        self.flash = Flash::Wrong;
    }

    /// Drop the series of each device in `ids`, which left the fleet,
    /// so a miner listed later under the same id starts with an empty chart.
    pub(crate) fn forget_devices(&mut self, ids: &[DeviceId]) {
        for id in ids {
            self.devices.remove(id);
        }
    }

    #[cfg(test)]
    pub(crate) fn has_device_series(&self, id: &DeviceId) -> bool {
        self.devices.contains_key(id)
    }

    /// A read-only view at `span` — the tier serving that range — plotted over `window`.
    #[must_use]
    pub(crate) fn view(&self, span: ChartSpan, window: ChartWindow) -> HistoryView<'_> {
        HistoryView {
            history: self,
            tier_index: span.tier(),
            window,
        }
    }
}

/// A read-only view of the history at one chart range. The view builders read
/// series through this, so they never need to know which tier they draw.
#[derive(Debug)]
pub struct HistoryView<'a> {
    history: &'a HashrateHistory,
    tier_index: usize,
    window: ChartWindow,
}

impl HistoryView<'_> {
    fn fleet_tier(&self) -> &FleetTier {
        &self.history.fleet[self.tier_index]
    }

    /// The time span the series are plotted over.
    #[must_use]
    pub(crate) fn window(&self) -> ChartWindow {
        self.window
    }

    #[must_use]
    pub(crate) fn total_series(&self) -> Vec<Option<ChartPoint>> {
        self.fleet_tier().total.plot(self.window)
    }

    #[must_use]
    pub(crate) fn model_series(
        &self,
        family: Option<DeviceFamily>,
        label: &str,
    ) -> Vec<Option<ChartPoint>> {
        self.fleet_tier()
            .models
            .get(&(family.map(DeviceFamily::index), label.to_owned()))
            .map_or_else(Vec::new, |series| series.plot(self.window))
    }

    #[must_use]
    pub(crate) fn device_series(&self, id: &DeviceId) -> Vec<Option<ChartPoint>> {
        self.history
            .devices
            .get(id)
            .map_or_else(Vec::new, |tiers| tiers[self.tier_index].plot(self.window))
    }
}

// ── Flash persistence (fleet tiers only) ─────────────────────────

/// Persist at most this often (seconds); a crash loses at most this much.
const SNAPSHOT_INTERVAL_SECS: i64 = 300;

/// How the flash copy of the total + per-model tiers compares with the tiers in RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flash {
    /// Flash holds what the tiers hold.
    Saved,
    /// The tiers changed since the last write, which waits for the snapshot interval.
    Behind,
    /// Flash may hold buckets a clock set back dropped from the tiers, so it is rewritten at once.
    Wrong,
}

/// Blob header: a magic byte and a format version. A mismatch on read means a
/// stale or foreign blob, so it's dropped rather than decoded.
const SNAPSHOT_MAGIC: u8 = 0xF1;
const SNAPSHOT_VERSION: u8 = 2;

/// Flash cache tag for the persisted history blob. Only the wasm build reads or
/// writes the cache; the snapshot codec itself is exercised natively in tests.
#[cfg(target_arch = "wasm32")]
pub(crate) const CACHE_TAG: &str = "hashrate_history";

/// The persisted shape of the total + per-model tiers (per-device is RAM-only).
#[derive(Debug, bincode::Encode, bincode::Decode)]
struct FleetTierSnapshot {
    total: Vec<HistoryDatum>,
    /// `(family index, model label, series)` — a `Vec`, not a map, to stay
    /// `alloc`-only for bincode.
    models: Vec<(Option<u32>, String, Vec<HistoryDatum>)>,
}

impl HashrateHistory {
    /// A blob of the total + per-model tiers for the flash cache (`now` is unix seconds).
    /// `None` while flash holds what the tiers hold,
    /// within the snapshot interval of the last write unless a clock set back dropped buckets,
    /// and while a restored blob waits for the clock, so flash keeps that blob.
    pub(crate) fn take_snapshot(&mut self, now: i64) -> Option<Vec<u8>> {
        if self.pending.is_some() {
            return None;
        }
        match self.flash {
            Flash::Saved => return None,
            Flash::Behind => {
                if let Some(last) = self.last_snapshot_at
                    && now - last < SNAPSHOT_INTERVAL_SECS
                {
                    return None;
                }
            }
            Flash::Wrong => {}
        }
        let tiers: Vec<FleetTierSnapshot> = self.fleet.iter().map(FleetTier::to_snapshot).collect();
        // Encoding failure here would be a logic bug, not bad input, so no
        // snapshot is the safe fallback.
        let Ok(mut body) = bincode::encode_to_vec(&tiers, bincode::config::standard()) else {
            return None;
        };
        self.flash = Flash::Saved;
        self.last_snapshot_at = Some(now);
        let mut blob = vec![SNAPSHOT_MAGIC, SNAPSHOT_VERSION];
        blob.append(&mut body);
        Some(blob)
    }

    /// Restore the total + per-model tiers from a `saved_at`-stamped blob.
    /// Returns `false` on a malformed blob so the caller can evict it;
    /// per-device series are never restored.
    ///
    /// A blob newer than the clock waits out of the tiers, and flash keeps it,
    /// until the clock reaches it: usually the clock is behind until NTP sets it.
    /// A clock that never gets there leaves the charts unsaved.
    pub(crate) fn restore(&mut self, bytes: &[u8], saved_at: i64, now: i64) -> bool {
        let Some(payload) = bytes.strip_prefix(&[SNAPSHOT_MAGIC, SNAPSHOT_VERSION]) else {
            return false;
        };
        let Ok((tiers, _)) = bincode::decode_from_slice::<Vec<FleetTierSnapshot>, _>(
            payload,
            bincode::config::standard(),
        ) else {
            return false;
        };
        if tiers.len() != self.fleet.len() {
            return false;
        }
        self.latest_clock = Some(now);
        let pending = PendingBlob::new(tiers, saved_at);
        match pending.newest {
            Some(newest) if newest > now => self.pending = Some(pending),
            _ => self.merge(pending, now),
        }
        true
    }

    /// Put `blob` before the tiers' buckets in the slots after its newest.
    /// A tier whose downtime already covers its whole range takes none of the blob,
    /// as its points would all be off-screen, but still drops its buckets up to the cut.
    /// The buckets recorded before the clock reached the blob
    /// were stamped by a clock behind it, so they go.
    fn merge(&mut self, blob: PendingBlob, now: i64) {
        let cut = blob.newest.unwrap_or(i64::MIN);
        let stale = now.saturating_sub(blob.saved_at);
        for (tier, snap) in self.fleet.iter_mut().zip(blob.tiers) {
            let snap = (stale < tier.span.range_secs()).then_some(snap);
            tier.merge_older(snap, cut);
        }
    }
}

/// A decoded blob and the newest bucket stamped in it.
#[derive(Debug)]
struct PendingBlob {
    tiers: Vec<FleetTierSnapshot>,
    saved_at: i64,
    newest: Option<i64>,
}

impl PendingBlob {
    fn new(tiers: Vec<FleetTierSnapshot>, saved_at: i64) -> Self {
        let mut blob = Self {
            tiers,
            saved_at,
            newest: None,
        };
        blob.newest = blob.stamps().max();
        blob
    }

    fn stamps(&self) -> impl Iterator<Item = i64> + '_ {
        self.tiers.iter().flat_map(|tier| {
            tier.total
                .iter()
                .chain(tier.models.iter().flat_map(|(_, _, samples)| samples))
                .map(|s| s.at)
        })
    }

    /// Drop the buckets stamped after `now`; returns whether any are left.
    fn drop_after(&mut self, now: i64) -> bool {
        for tier in &mut self.tiers {
            tier.total.retain(|s| s.at <= now);
            for (_, _, samples) in &mut tier.models {
                samples.retain(|s| s.at <= now);
            }
            tier.models.retain(|(_, _, samples)| !samples.is_empty());
        }
        self.newest = self.stamps().max();
        self.newest.is_some()
    }
}

impl FleetTier {
    fn to_snapshot(&self) -> FleetTierSnapshot {
        FleetTierSnapshot {
            total: self.total.samples(),
            models: self
                .models
                .iter()
                .map(|((fam, label), series)| {
                    let fam = fam.and_then(|i| u32::try_from(i).ok());
                    (fam, label.clone(), series.samples())
                })
                .collect(),
        }
    }

    /// Put `snap`'s buckets before the ones in the slots after `cut`'s, dropping the rest.
    /// Without a snapshot only the cut applies.
    fn merge_older(&mut self, snap: Option<FleetTierSnapshot>, cut: i64) {
        let (total, models) = snap.map_or_else(Default::default, |snap| (snap.total, snap.models));
        self.total.merge_older(total, cut);
        let span = self.span;
        let mut restored: HashMap<(Option<usize>, String), Vec<HistoryDatum>> = models
            .into_iter()
            .map(|(fam, label, samples)| ((fam.map(|i| i as usize), label), samples))
            .collect();
        for (key, series) in &mut self.models {
            series.merge_older(restored.remove(key).unwrap_or_default(), cut);
        }
        for (key, samples) in restored {
            self.models.insert(key, Series::restored(span, samples));
        }
    }

    fn drop_after(&mut self, now: i64) {
        self.total.drop_after(now);
        for series in self.models.values_mut() {
            series.drop_after(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceList;
    use crate::summary::FleetSummary;
    use bmc_wasm_sdk::types::{ElectricPower, MiningEfficiency, Temperature};

    /// The `value`s of a series, for terse assertions.
    fn vals(series: &[HistoryDatum]) -> Vec<Option<f32>> {
        series.iter().map(|s| s.value).collect()
    }

    /// The fleet total's stored buckets at `span`.
    fn total(h: &HashrateHistory, span: ChartSpan) -> Vec<HistoryDatum> {
        h.fleet[span.tier()].total.samples()
    }

    /// A BOS model group's stored buckets at `span`.
    fn model(h: &HashrateHistory, span: ChartSpan, label: &str) -> Vec<HistoryDatum> {
        h.fleet[span.tier()]
            .models
            .get(&(Some(DeviceFamily::Bos.index()), label.to_owned()))
            .map_or_else(Vec::new, Series::samples)
    }

    /// A device's stored buckets at `span`.
    fn device(h: &HashrateHistory, span: ChartSpan, id: &DeviceId) -> Vec<HistoryDatum> {
        h.devices
            .get(id)
            .map_or_else(Vec::new, |tiers| tiers[span.tier()].samples())
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
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        h.record_fleet(14, &summary(99.0, 99.0).hashrate()); // same bucket — refreshes
        h.record_fleet(15, &summary(12.0, 5.0).hashrate());
        assert_eq!(
            vals(&total(&h, ChartSpan::FifteenMinutes)),
            vec![Some(99.0), Some(12.0)]
        );
        assert_eq!(
            vals(&model(&h, ChartSpan::FifteenMinutes, "BOS BMM")),
            vec![Some(99.0), Some(5.0)]
        );
    }

    #[test]
    fn the_open_bucket_keeps_its_timestamp_while_its_value_refreshes() {
        // A within-interval refresh updates the value but not the bucket's
        // timestamp, so points stay evenly spaced at the tier interval.
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        h.record_fleet(14, &summary(99.0, 4.0).hashrate());
        let series = total(&h, ChartSpan::FifteenMinutes);
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
        h.record_fleet(100, &summary(10.0, 4.0).hashrate());
        h.record_fleet(115, &summary(12.0, 5.0).hashrate());
        let ats: Vec<i64> = total(&h, ChartSpan::FifteenMinutes)
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
            h.record_fleet(now, &summary(1.0, 1.0).hashrate());
            now += 1;
        }
        assert_eq!(total(&h, ChartSpan::FifteenMinutes).len(), 5); // 0, 15, 30, 45, 60
        assert_eq!(total(&h, ChartSpan::OneHour).len(), 2); // 0, 60
    }

    #[test]
    fn empty_fleet_is_not_recorded() {
        let mut h = HashrateHistory::default();
        let empty = FleetSummary {
            total: group("Total", 0.0),
            groups: vec![],
        };
        h.record_fleet(0, &empty.hashrate());
        assert!(total(&h, ChartSpan::FifteenMinutes).is_empty());
        assert!(h.take_snapshot(0).is_none(), "nothing to save");
    }

    #[test]
    fn the_window_keeps_only_the_most_recent_samples() {
        let mut h = HashrateHistory::default();
        let mut now = 0_i64;
        let mut value = 0.0_f64;
        for _ in 0..(POINTS + 5) {
            h.record_fleet(now, &summary(value, 0.0).hashrate());
            now += 15; // one sample per 15-min-tier interval
            value += 1.0;
        }
        let series = total(&h, ChartSpan::FifteenMinutes);
        assert_eq!(series.len(), POINTS);
        assert_eq!(series[0].value, Some(5.0), "the oldest five were dropped");
    }

    #[test]
    fn an_unknown_model_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(model(&h, ChartSpan::FifteenMinutes, "nope").is_empty());
    }

    fn devices(specs: &[(&str, f32, bool)]) -> DeviceList {
        let mut list = DeviceList::new();
        for &(name, hashrate, reachable) in specs {
            list_device(&mut list, name, hashrate, reachable);
        }
        list
    }

    fn list_device(list: &mut DeviceList, name: &str, hashrate: f32, reachable: bool) {
        use crate::device::DeviceIdentity;
        use crate::telemetry::TelemetryReading;
        let id = DeviceId::new(name);
        list.upsert(DeviceIdentity {
            id: id.clone(),
            family: DeviceFamily::Bos,
            name: name.to_owned(),
            host: "10.0.0.1".to_owned(),
            port: 80,
        });
        list.apply_telemetry(
            &id,
            TelemetryReading {
                current_hashrate_ths: Some(hashrate),
                ..TelemetryReading::default()
            },
            reachable,
        );
    }

    /// Record every device in `list`, as polls landing at `now` would.
    fn poll_all(h: &mut HashrateHistory, now: i64, list: &DeviceList) {
        for dev in list.iter() {
            h.record_device(now, &dev.identity.id, dev.hashrate());
        }
    }

    #[test]
    fn records_a_per_device_series() {
        let mut h = HashrateHistory::default();
        let a = DeviceId::new("bos/a");
        poll_all(&mut h, 0, &devices(&[("bos/a", 3.0, true)]));
        poll_all(&mut h, 15, &devices(&[("bos/a", 4.0, true)]));
        assert_eq!(
            vals(&device(&h, ChartSpan::FifteenMinutes, &a)),
            vec![Some(3.0), Some(4.0)]
        );
    }

    #[test]
    fn a_second_poll_in_the_same_slot_refreshes_the_device_point() {
        let mut h = HashrateHistory::default();
        let a = DeviceId::new("bos/a");
        poll_all(&mut h, 0, &devices(&[("bos/a", 3.0, true)]));
        poll_all(&mut h, 10, &devices(&[("bos/a", 4.0, true)]));
        let series = device(&h, ChartSpan::FifteenMinutes, &a);
        assert_eq!(
            vals(&series),
            vec![Some(4.0)],
            "one 15 s bucket, newest value"
        );
        assert_eq!(
            series[0].at, 0,
            "the bucket keeps the first poll's timestamp"
        );
    }

    #[test]
    fn each_tier_samples_a_device_at_its_own_rate() {
        let mut h = HashrateHistory::default();
        let a = DeviceId::new("bos/a");
        let list = devices(&[("bos/a", 3.0, true)]);
        for now in 0..=60 {
            poll_all(&mut h, now, &list);
        }
        assert_eq!(
            device(&h, ChartSpan::FifteenMinutes, &a).len(),
            5,
            "the 15-min tier opens a bucket every 15 s: 0, 15, 30, 45, 60"
        );
        assert_eq!(
            device(&h, ChartSpan::OneHour, &a).len(),
            2,
            "the 1-h tier opens a bucket every 60 s: 0, 60"
        );
    }

    #[test]
    fn a_none_total_samples_a_gap_not_zero() {
        let mut h = HashrateHistory::default();
        let mut s = summary(10.0, 4.0);
        s.total.hashrate = None;
        s.groups[0].hashrate = None;
        h.record_fleet(0, &s.hashrate());
        assert_eq!(
            vals(&total(&h, ChartSpan::FifteenMinutes)),
            vec![None],
            "a summary with no hashrate is a gap, matching the device-series policy"
        );
        assert_eq!(
            vals(&model(&h, ChartSpan::FifteenMinutes, "BOS BMM")),
            vec![None]
        );
    }

    #[test]
    fn an_unreachable_device_samples_a_gap_not_zero() {
        let mut h = HashrateHistory::default();
        let down = DeviceId::new("bos/b");
        poll_all(
            &mut h,
            0,
            &devices(&[("bos/a", 3.0, true), ("bos/b", 9.0, false)]),
        );
        assert_eq!(
            vals(&device(&h, ChartSpan::FifteenMinutes, &down)),
            vec![None],
            "unreachable is a gap, not a false zero"
        );
    }

    #[test]
    fn an_unknown_device_has_an_empty_series() {
        let h = HashrateHistory::default();
        assert!(device(&h, ChartSpan::FifteenMinutes, &DeviceId::new("bos/nope")).is_empty());
    }

    #[test]
    fn a_device_listed_again_under_its_id_starts_a_fresh_series() {
        let mut h = HashrateHistory::default();
        let reused = DeviceId::new("bos/a");
        let mut list = devices(&[("bos/a", 3.0, true)]);
        poll_all(&mut h, 0, &list);
        list.remove(&reused);
        h.forget_devices(std::slice::from_ref(&reused));
        list_device(&mut list, "bos/a", 4.0, true);
        poll_all(&mut h, 15, &list);
        assert_eq!(
            vals(&device(&h, ChartSpan::FifteenMinutes, &reused)),
            vec![Some(4.0)],
            "the new miner does not inherit the points of the one that left"
        );
    }

    #[test]
    fn a_fleet_sample_is_due_once_per_slot() {
        let mut h = HashrateHistory::default();
        assert!(h.fleet_sample_due(0), "nothing sampled yet");
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        assert!(
            !h.fleet_sample_due(14),
            "still the 15-min tier's first slot"
        );
        assert!(h.fleet_sample_due(15), "the 15-min tier opened a new slot");
    }

    #[test]
    fn a_fleet_sample_is_due_once_a_restored_tier_enters_a_new_slot() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");
        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 0, 5), "restored within range");
        assert!(
            !restored.fleet_sample_due(5),
            "the restored bucket's slot counts as sampled"
        );
        assert!(
            restored.fleet_sample_due(15),
            "the 15-min tier opened a new slot"
        );
    }

    /// The `value`s of the "BOS BMM" series at `span`.
    fn bmm(h: &HashrateHistory, span: ChartSpan) -> Vec<Option<f32>> {
        vals(&model(h, span, "BOS BMM"))
    }

    /// The "BOS BMM" buckets at `span`, as (timestamp, value).
    fn bmm_buckets(h: &HashrateHistory, span: ChartSpan) -> Vec<(i64, Option<f32>)> {
        model(h, span, "BOS BMM")
            .iter()
            .map(|s| (s.at, s.value))
            .collect()
    }

    /// A reading with both "BOS BMM" and "BOS BFM".
    fn bmm_and_bfm(bmm: f64, bfm: f64) -> FleetSummary {
        FleetSummary {
            total: group("Total", bmm + bfm),
            groups: vec![group("BOS BMM", bmm), group("BOS BFM", bfm)],
        }
    }

    /// A reading where "BOS BMM" is absent.
    fn bfm_alone(bfm: f64) -> FleetSummary {
        FleetSummary {
            total: group("Total", bfm),
            groups: vec![group("BOS BFM", bfm)],
        }
    }

    #[test]
    fn a_model_returning_inside_an_open_slot_opens_its_own_bucket() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &bmm_and_bfm(4.0, 7.0).hashrate());
        h.record_fleet(15, &bfm_alone(7.0).hashrate());
        h.record_fleet(30, &bfm_alone(7.0).hashrate());
        h.record_fleet(31, &bmm_and_bfm(9.0, 7.0).hashrate());
        assert_eq!(
            bmm_buckets(&h, ChartSpan::FifteenMinutes),
            vec![(0, Some(4.0)), (31, Some(9.0))],
            "BFM opened the slot at 30, but BMM's own last bucket is the one at 0"
        );
    }

    #[test]
    fn a_restored_model_rediscovered_after_another_opens_its_own_bucket() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &bmm_and_bfm(4.0, 7.0).hashrate());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");
        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 0, 3), "restored within range");
        // Discovery finds BFM first, opening the next slot, and BMM a moment later in it.
        restored.record_fleet(16, &bfm_alone(7.0).hashrate());
        restored.record_fleet(20, &bmm_and_bfm(9.0, 7.0).hashrate());
        assert_eq!(
            bmm_buckets(&restored, ChartSpan::FifteenMinutes),
            vec![(0, Some(4.0)), (20, Some(9.0))],
            "BFM opened the slot at 15, but BMM's own last bucket is the restored one at 0"
        );
    }

    #[test]
    fn a_vanished_model_group_is_kept_until_it_leaves_the_range() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate()); // group "BOS BMM"
        let other = FleetSummary {
            total: group("Total", 7.0),
            groups: vec![group("BOS BFM", 7.0)],
        };
        h.record_fleet(15, &other.hashrate());
        assert_eq!(
            bmm(&h, ChartSpan::FifteenMinutes),
            vec![Some(4.0)],
            "gone from the reading, still inside the range"
        );
        h.record_fleet(15 * 60, &other.hashrate());
        assert!(
            bmm(&h, ChartSpan::FifteenMinutes).is_empty(),
            "its newest sample has left the 15-min range"
        );
        assert_eq!(
            bmm(&h, ChartSpan::OneHour),
            vec![Some(4.0)],
            "the 1-h range still draws it"
        );
    }

    #[test]
    fn the_unknown_group_is_dropped_once_absent_while_a_model_is_kept() {
        let mut h = HashrateHistory::default();
        let unknown = crate::summary::GroupSummary {
            family: None,
            ..group(UNKNOWN_GROUP, 3.0)
        };
        let both = FleetSummary {
            total: group("Total", 7.0),
            groups: vec![group("BOS BMM", 4.0), unknown],
        };
        h.record_fleet(0, &both.hashrate());
        h.record_fleet(15, &both.hashrate());
        assert_eq!(
            h.fleet[ChartSpan::FifteenMinutes.tier()]
                .models
                .get(&(None, UNKNOWN_GROUP.to_owned()))
                .map(|series| series.samples().len()),
            Some(2),
            "the catch-all keeps its series while a reading holds it"
        );
        h.record_fleet(30, &bfm_alone(7.0).hashrate());
        assert!(
            !h.fleet[ChartSpan::FifteenMinutes.tier()]
                .models
                .contains_key(&(None, UNKNOWN_GROUP.to_owned())),
            "the catch-all holds whatever had no model yet, so its past means nothing"
        );
        assert_eq!(
            bmm(&h, ChartSpan::FifteenMinutes),
            vec![Some(4.0), Some(4.0)],
            "a model that left keeps its series"
        );
    }

    #[test]
    fn a_restored_model_series_waits_for_its_model_to_be_rediscovered() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");
        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 0, 60), "restored within range");

        // The first sample after boot lands before discovery has found any miner.
        let empty = FleetSummary {
            total: group("Total", 0.0),
            groups: vec![],
        };
        restored.record_fleet(65, &empty.hashrate());
        assert_eq!(
            bmm(&restored, ChartSpan::FifteenMinutes),
            vec![Some(4.0)],
            "kept while discovery has not found the model yet"
        );
        assert!(
            restored.take_snapshot(65).is_none(),
            "nothing was recorded, so flash keeps the restored blob"
        );

        // Discovery finds another model first.
        restored.record_fleet(66, &bfm_alone(7.0).hashrate());
        let resaved = restored
            .take_snapshot(66)
            .expect("BUG: the first snapshot after a restore fires");
        let mut again = HashrateHistory::default();
        assert!(again.restore(&resaved, 66, 70), "restored within range");
        assert_eq!(
            bmm(&again, ChartSpan::FifteenMinutes),
            vec![Some(4.0)],
            "the snapshot taken after that sample still carries it"
        );

        again.record_fleet(75, &summary(12.0, 5.0).hashrate());
        let window = ChartWindow {
            end: 75,
            span_secs: 900,
        };
        assert_eq!(
            again
                .view(ChartSpan::FifteenMinutes, window)
                .model_series(Some(DeviceFamily::Bos), "BOS BMM"),
            vec![
                Some(ChartPoint { at: 0, value: 4.0 }),
                None,
                Some(ChartPoint { at: 75, value: 5.0 }),
            ],
            "rediscovered, the model continues across the downtime's gap"
        );
    }

    // ── Persistence ──────────────────────────────────────────────────

    #[test]
    fn snapshot_round_trips_total_and_model_series() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        poll_all(&mut h, 0, &devices(&[("bos/a", 3.0, true)]));
        h.record_fleet(15, &summary(12.0, 5.0).hashrate());
        poll_all(&mut h, 15, &devices(&[("bos/a", 4.0, true)]));
        let blob = h
            .take_snapshot(15)
            .expect("BUG: first snapshot always fires");

        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 15, 20), "fresh blob restores");
        assert_eq!(
            vals(&total(&restored, ChartSpan::FifteenMinutes)),
            vec![Some(10.0), Some(12.0)]
        );
        assert_eq!(
            vals(&model(&restored, ChartSpan::FifteenMinutes, "BOS BMM")),
            vec![Some(4.0), Some(5.0)]
        );
        // Per-device series are RAM-only: never persisted.
        assert!(
            device(
                &restored,
                ChartSpan::FifteenMinutes,
                &DeviceId::new("bos/a")
            )
            .is_empty()
        );
    }

    #[test]
    fn a_stale_tier_is_dropped_on_restore() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");
        let mut restored = HashrateHistory::default();
        // Downtime of 20 min: exceeds the 15-min tier's range (dropped) but not
        // the 1-h tier's (kept).
        let saved_at = 0;
        let now = 20 * 60;
        restored.restore(&blob, saved_at, now);
        assert!(
            total(&restored, ChartSpan::FifteenMinutes).is_empty(),
            "15-min tier fully stale"
        );
        assert_eq!(
            vals(&total(&restored, ChartSpan::OneHour)),
            vec![Some(10.0)],
            "1-h tier still in range"
        );
    }

    #[test]
    fn a_reading_in_the_restored_slot_leaves_the_saved_point_alone() {
        let mut h = HashrateHistory::default();
        h.record_fleet(2_900, &summary(120.0, 4.0).hashrate());
        let blob = h
            .take_snapshot(2_900)
            .expect("BUG: the first snapshot fires");

        // Back a minute later, inside the same 24-minute slot, before any miner was polled.
        let mut restored = HashrateHistory::default();
        assert!(
            restored.restore(&blob, 2_900, 2_960),
            "restored within range"
        );
        let mut unpolled = summary(0.0, 0.0);
        unpolled.total.hashrate = None;
        unpolled.groups[0].hashrate = None;
        restored.record_fleet(2_962, &unpolled.hashrate());
        assert_eq!(
            vals(&total(&restored, ChartSpan::TwentyFourHours)),
            vec![Some(120.0)],
            "the restored bucket keeps its value"
        );

        let resaved = restored
            .take_snapshot(2_962)
            .expect("BUG: the first snapshot after a restore fires");
        let mut again = HashrateHistory::default();
        assert!(
            again.restore(&resaved, 2_962, 2_970),
            "restored within range"
        );
        assert_eq!(
            vals(&total(&again, ChartSpan::TwentyFourHours)),
            vec![Some(120.0)],
            "and so does flash"
        );
    }

    #[test]
    fn a_restart_plots_as_a_break_by_time_alone() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        let blob = h.take_snapshot(0).expect("BUG: snapshot fires");

        // Back 55 minutes later: one cached point, then a live one, 55 slots apart.
        let mut restored = HashrateHistory::default();
        let downtime = 55 * 60;
        assert!(
            restored.restore(&blob, 0, downtime),
            "1-h tier still in range"
        );
        restored.record_fleet(downtime, &summary(12.0, 5.0).hashrate());

        let window = ChartWindow {
            end: downtime,
            span_secs: 3_600,
        };
        assert_eq!(
            restored.view(ChartSpan::OneHour, window).total_series(),
            vec![
                Some(ChartPoint { at: 0, value: 10.0 }),
                None,
                Some(ChartPoint {
                    at: downtime,
                    value: 12.0
                }),
            ],
            "the downtime separates cached data from live"
        );
    }

    #[test]
    fn a_break_saved_by_an_earlier_build_still_plots() {
        // Earlier builds ended every restored series with an explicit `None`,
        // so snapshots on devices carry them.
        let with_break = vec![
            HistoryDatum {
                at: 0,
                value: Some(10.0),
            },
            HistoryDatum {
                at: 15,
                value: None,
            },
            HistoryDatum {
                at: 30,
                value: Some(12.0),
            },
        ];
        let tiers: Vec<FleetTierSnapshot> = ChartSpan::ALL
            .iter()
            .map(|_| FleetTierSnapshot {
                total: with_break.clone(),
                models: vec![],
            })
            .collect();
        let mut blob = vec![SNAPSHOT_MAGIC, SNAPSHOT_VERSION];
        blob.extend(
            bincode::encode_to_vec(&tiers, bincode::config::standard())
                .expect("BUG: a snapshot encodes"),
        );
        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 30, 35), "restored within range");
        let window = ChartWindow {
            end: 35,
            span_secs: 900,
        };
        assert_eq!(
            restored
                .view(ChartSpan::FifteenMinutes, window)
                .total_series(),
            vec![
                Some(ChartPoint { at: 0, value: 10.0 }),
                None,
                Some(ChartPoint {
                    at: 30,
                    value: 12.0
                }),
            ],
            "the saved break splits the line though the slots alone would join it"
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
            total(&h, ChartSpan::FifteenMinutes).is_empty(),
            "nothing restored"
        );
    }

    #[test]
    fn snapshot_is_rate_limited() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        assert!(h.take_snapshot(0).is_some(), "first snapshot fires");
        h.record_fleet(15, &summary(11.0, 4.0).hashrate());
        assert!(
            h.take_snapshot(SNAPSHOT_INTERVAL_SECS - 1).is_none(),
            "within the interval — skipped"
        );
        assert!(
            h.take_snapshot(SNAPSHOT_INTERVAL_SECS).is_some(),
            "after the interval — fires"
        );
    }

    #[test]
    fn a_history_unchanged_since_the_last_snapshot_is_not_written_again() {
        let mut h = HashrateHistory::default();
        h.record_fleet(0, &summary(10.0, 4.0).hashrate());
        assert!(h.take_snapshot(0).is_some(), "first snapshot fires");
        assert!(
            h.take_snapshot(SNAPSHOT_INTERVAL_SECS).is_none(),
            "nothing was recorded since"
        );
    }

    #[test]
    fn a_series_an_empty_reading_prunes_leaves_flash_too() {
        let mut h = HashrateHistory::default();
        let unknown = crate::summary::GroupSummary {
            family: None,
            ..group(UNKNOWN_GROUP, 3.0)
        };
        let both = FleetSummary {
            total: group("Total", 7.0),
            groups: vec![group("BOS BMM", 4.0), unknown],
        };
        h.record_fleet(0, &both.hashrate());
        assert!(h.take_snapshot(0).is_some(), "first snapshot fires");
        let empty = FleetSummary {
            total: group("Total", 0.0),
            groups: vec![],
        };
        h.record_fleet(15, &empty.hashrate());
        let blob = h
            .take_snapshot(SNAPSHOT_INTERVAL_SECS)
            .expect("BUG: the pruned catch-all left the tiers, so flash is behind them");
        let mut restored = HashrateHistory::default();
        assert!(
            restored.restore(&blob, SNAPSHOT_INTERVAL_SECS, SNAPSHOT_INTERVAL_SECS + 5),
            "restored within range"
        );
        assert!(
            !restored.fleet[ChartSpan::FifteenMinutes.tier()]
                .models
                .contains_key(&(None, UNKNOWN_GROUP.to_owned())),
            "the saved history no longer holds the catch-all"
        );
    }

    // ── Clock steps ──────────────────────────────────────────────────

    /// The stamps of a series' buckets.
    fn stamps(series: &[HistoryDatum]) -> Vec<i64> {
        series.iter().map(|s| s.at).collect()
    }

    /// A blob saved at 1 000 s, holding buckets at 900 and 1 000 s.
    fn blob_saved_at_1000() -> Vec<u8> {
        let mut h = HashrateHistory::default();
        h.record_fleet(900, &summary(10.0, 4.0).hashrate());
        h.record_fleet(1_000, &summary(11.0, 4.0).hashrate());
        h.take_snapshot(1_000)
            .expect("BUG: the first snapshot fires")
    }

    #[test]
    fn a_clock_set_back_drops_what_was_stamped_after_it() {
        let mut h = HashrateHistory::default();
        let a = DeviceId::new("bos/a");
        let list = devices(&[("bos/a", 3.0, true)]);
        for now in [0, 15, 30, 45] {
            h.record_fleet(now, &summary(10.0, 4.0).hashrate());
            poll_all(&mut h, now, &list);
        }
        h.record_fleet(20, &summary(12.0, 5.0).hashrate());
        let total = total(&h, ChartSpan::FifteenMinutes);
        assert_eq!(
            stamps(&total),
            vec![0, 15],
            "the buckets at 30 and 45 s were stamped by a clock that was ahead"
        );
        assert_eq!(
            vals(&total),
            vec![Some(10.0), Some(10.0)],
            "the cut closes the bucket at 15 s, so the reading at 20 s in its slot is dropped"
        );
        assert_eq!(
            stamps(&device(&h, ChartSpan::FifteenMinutes, &a)),
            vec![0, 15],
            "device series are cut too"
        );
    }

    #[test]
    fn a_clock_set_back_rewrites_flash_at_once() {
        let mut h = HashrateHistory::default();
        for now in [0, 15, 30, 45] {
            h.record_fleet(now, &summary(10.0, 4.0).hashrate());
        }
        assert!(h.take_snapshot(45).is_some(), "the first snapshot fires");
        h.record_fleet(20, &summary(12.0, 5.0).hashrate());
        let blob = h
            .take_snapshot(20)
            .expect("BUG: the step back lifts the rate limit, so flash loses the buckets after it");
        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 20, 25), "restored within range");
        assert_eq!(
            stamps(&total(&restored, ChartSpan::FifteenMinutes)),
            vec![0, 15]
        );
    }

    #[test]
    fn a_clock_set_back_on_an_empty_fleet_still_rewrites_flash() {
        let mut h = HashrateHistory::default();
        for now in [0, 15, 30, 45] {
            h.record_fleet(now, &summary(10.0, 4.0).hashrate());
        }
        assert!(h.take_snapshot(45).is_some(), "the first snapshot fires");
        let empty = FleetSummary {
            total: group("Total", 0.0),
            groups: vec![],
        };
        h.record_fleet(20, &empty.hashrate());
        let blob = h
            .take_snapshot(20)
            .expect("BUG: flash holds the dropped buckets whatever the reading held");
        let mut restored = HashrateHistory::default();
        assert!(restored.restore(&blob, 20, 25), "restored within range");
        assert_eq!(
            stamps(&total(&restored, ChartSpan::FifteenMinutes)),
            vec![0, 15]
        );
    }

    #[test]
    fn a_clock_set_back_first_seen_by_a_device_poll_cuts_every_series() {
        let mut h = HashrateHistory::default();
        let a = DeviceId::new("bos/a");
        let list = devices(&[("bos/a", 3.0, true)]);
        for now in [0, 15, 30, 45] {
            h.record_fleet(now, &summary(10.0, 4.0).hashrate());
            poll_all(&mut h, now, &list);
        }
        poll_all(&mut h, 20, &list);
        assert_eq!(
            stamps(&device(&h, ChartSpan::FifteenMinutes, &a)),
            vec![0, 15],
            "the poll follows the clock before it records"
        );
        assert_eq!(
            stamps(&total(&h, ChartSpan::FifteenMinutes)),
            vec![0, 15],
            "the fleet series are cut by the same step"
        );
    }

    #[test]
    fn a_blob_newer_than_the_clock_waits_out_of_the_tiers() {
        let mut h = HashrateHistory::default();
        assert!(
            h.restore(&blob_saved_at_1000(), 1_000, 50),
            "a well-formed blob"
        );
        assert!(
            total(&h, ChartSpan::FifteenMinutes).is_empty(),
            "a clock behind the blob cannot place it yet"
        );
        h.record_fleet(50, &summary(12.0, 5.0).hashrate());
        assert!(
            !total(&h, ChartSpan::FifteenMinutes).is_empty(),
            "live readings record meanwhile"
        );
        assert!(
            h.take_snapshot(50).is_none(),
            "nothing overwrites the blob while it waits"
        );
    }

    #[test]
    fn a_waiting_blob_merges_once_the_clock_reaches_it() {
        let mut h = HashrateHistory::default();
        assert!(
            h.restore(&blob_saved_at_1000(), 1_000, 50),
            "a well-formed blob"
        );
        h.record_fleet(50, &summary(12.0, 5.0).hashrate());
        h.record_fleet(1_010, &summary(13.0, 5.0).hashrate());
        assert_eq!(
            stamps(&total(&h, ChartSpan::FifteenMinutes)),
            vec![900, 1_000, 1_010],
            "the blob, then live; the reading at 50 s was stamped by a clock behind it"
        );
        assert_eq!(
            vals(&model(&h, ChartSpan::FifteenMinutes, "BOS BMM")),
            vec![Some(4.0), Some(4.0), Some(5.0)],
            "model series merge the same way"
        );
        assert!(
            h.take_snapshot(1_010).is_some(),
            "with the blob merged, flash is written again"
        );
    }

    #[test]
    fn a_waiting_blob_skips_the_tiers_its_downtime_left_behind() {
        let mut h = HashrateHistory::default();
        assert!(
            h.restore(&blob_saved_at_1000(), 1_000, 50),
            "a well-formed blob"
        );
        let now = 1_000 + 20 * 60;
        h.record_fleet(now, &summary(13.0, 5.0).hashrate());
        assert_eq!(
            stamps(&total(&h, ChartSpan::FifteenMinutes)),
            vec![now],
            "20 minutes after the save, the 15-min tier would draw none of it"
        );
        assert_eq!(
            stamps(&total(&h, ChartSpan::OneHour)),
            vec![900, 1_000, now],
            "the 1-h tier still draws it"
        );
    }

    #[test]
    fn a_clock_set_back_cuts_a_waiting_blob() {
        let mut h = HashrateHistory::default();
        assert!(
            h.restore(&blob_saved_at_1000(), 1_000, 950),
            "a well-formed blob"
        );
        h.record_fleet(960, &summary(12.0, 5.0).hashrate());
        h.record_fleet(940, &summary(13.0, 5.0).hashrate());
        assert_eq!(
            stamps(&total(&h, ChartSpan::FifteenMinutes)),
            vec![900, 940],
            "a corrected clock keeps the blob's past and drops the rest, so the blob merges"
        );
        assert!(h.take_snapshot(940).is_some(), "and flash is written again");
    }

    #[test]
    fn a_clock_set_back_into_a_waiting_blob_keeps_one_bucket_per_slot() {
        let mut h = HashrateHistory::default();
        assert!(
            h.restore(&blob_saved_at_1000(), 1_000, 905),
            "a well-formed blob"
        );
        h.record_fleet(910, &summary(12.0, 5.0).hashrate());
        h.record_fleet(990, &summary(13.0, 5.0).hashrate());
        h.record_fleet(950, &summary(14.0, 5.0).hashrate());
        assert_eq!(
            stamps(&total(&h, ChartSpan::FifteenMinutes)),
            vec![900, 950],
            "the cut blob ends at 900, and the live bucket at 910 shares its slot"
        );
    }

    #[test]
    fn a_clock_set_back_before_a_waiting_blob_drops_it() {
        let mut h = HashrateHistory::default();
        assert!(
            h.restore(&blob_saved_at_1000(), 1_000, 50),
            "a well-formed blob"
        );
        h.record_fleet(60, &summary(12.0, 5.0).hashrate());
        h.record_fleet(40, &summary(13.0, 5.0).hashrate());
        assert_eq!(
            stamps(&total(&h, ChartSpan::FifteenMinutes)),
            vec![40],
            "every bucket of the blob is after the corrected clock"
        );
        assert!(
            h.take_snapshot(40).is_some(),
            "flash is written again, overwriting the blob"
        );
    }
}

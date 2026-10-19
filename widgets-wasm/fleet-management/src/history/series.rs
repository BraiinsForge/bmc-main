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

//! One chart series at one tier's resolution, and how a chart draws it.

use std::collections::VecDeque;

#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::{log_error, ufmt};

use super::{ChartSpan, ChartWindow, HistoryDatum, POINTS};

/// Buckets more than this many slots apart draw as a break.
/// Counted in the tier's slots, not fitted to how often the data arrives.
const MAX_SLOT_STEP: i64 = 2;

/// A sample the chart draws: when it was taken (unix seconds) and its value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartPoint {
    pub at: i64,
    pub value: f32,
}

/// A bucket for each slot of the tier's interval that a sample landed in,
/// holding the newest value seen in that slot and stamped with the time of its first sample.
#[derive(Debug)]
pub(super) struct Series {
    interval_secs: i64,
    newest: Newest,
    /// At most [`POINTS`] buckets, oldest first.
    samples: VecDeque<HistoryDatum>,
}

/// Whether a live sample in the newest bucket's slot may still refresh it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Newest {
    Open,
    /// A restored bucket, or the newest one a cut leaves:
    /// a sample in its slot is dropped, so live readings never change saved history,
    /// and the slot still counts as sampled, so a restart costs no extra bucket.
    Closed,
}

impl Series {
    pub(super) fn new(span: ChartSpan) -> Self {
        Self {
            interval_secs: span.interval_secs(),
            newest: Newest::Closed,
            samples: VecDeque::new(),
        }
    }

    /// A series of decoded samples, keeping only the newest [`POINTS`].
    pub(super) fn restored(span: ChartSpan, samples: Vec<HistoryDatum>) -> Self {
        let mut series = Self::new(span);
        series.merge_older(samples, i64::MAX);
        series
    }

    /// Put `older`, a decoded blob's buckets, before this series' buckets
    /// in the slots after `cut`'s, dropping the rest,
    /// so the blob and the live buckets never share a slot. Keep the newest [`POINTS`].
    ///
    /// The blob is decoded, not trusted.
    /// Earlier builds saved buckets in the order a stepping clock stamped them, so this sorts them.
    /// A series over capacity would stay over it forever,
    /// since evicting one per push holds the length rather than lowering it.
    pub(super) fn merge_older(&mut self, mut older: Vec<HistoryDatum>, cut: i64) {
        older.sort_by_key(|s| s.at);
        let cut_slot = slot_of(cut, self.interval_secs);
        self.samples
            .retain(|s| slot_of(s.at, self.interval_secs) > cut_slot);
        let live_kept = !self.samples.is_empty();
        let mut merged: VecDeque<HistoryDatum> = older.into();
        merged.append(&mut self.samples);
        let excess = merged.len().saturating_sub(POINTS);
        merged.drain(..excess);
        self.samples = merged;
        if !live_kept {
            self.newest = Newest::Closed;
        }
    }

    /// Drop the buckets stamped after `at`, which a clock that went back no longer reaches.
    pub(super) fn drop_after(&mut self, at: i64) {
        let len = self.samples.len();
        while self.samples.back().is_some_and(|s| s.at > at) {
            self.samples.pop_back();
        }
        if self.samples.len() != len {
            self.newest = Newest::Closed;
        }
    }

    /// Record a sample at `at`.
    /// A sample older than the newest bucket is dropped, so the buckets stay in time order.
    pub(super) fn record(&mut self, at: i64, value: Option<f32>) {
        let newest_at = self.newest_at().unwrap_or(at);
        if at < newest_at {
            #[cfg(target_arch = "wasm32")]
            log_error!(
                "fleet: chart sample at {} dropped, older than the newest bucket at {}",
                at,
                newest_at
            );
            return;
        }
        let slot = slot_of(at, self.interval_secs);
        match self.samples.back_mut() {
            Some(newest) if slot_of(newest.at, self.interval_secs) == slot => {
                if self.newest == Newest::Open {
                    newest.value = value;
                }
            }
            _ => {
                self.push(HistoryDatum { at, value });
                self.newest = Newest::Open;
            }
        }
    }

    fn push(&mut self, sample: HistoryDatum) {
        if self.samples.len() >= POINTS {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub(super) fn newest_at(&self) -> Option<i64> {
        self.samples.back().map(|s| s.at)
    }

    /// Whether the newest bucket lies in the slot of `at`.
    pub(super) fn has_sampled_slot_of(&self, at: i64) -> bool {
        self.newest_at().is_some_and(|newest| {
            slot_of(newest, self.interval_secs) == slot_of(at, self.interval_secs)
        })
    }

    /// The stored buckets, oldest first.
    pub(super) fn samples(&self) -> Vec<HistoryDatum> {
        self.samples.iter().copied().collect()
    }

    /// The buckets a chart draws over `window`, oldest first: a point per bucket with a value,
    /// and `None` where the line breaks, at a bucket without a value
    /// or between buckets more than [`MAX_SLOT_STEP`] slots apart.
    /// A break only ever follows a point: consecutive ones collapse, and none leads.
    /// Of the buckets before the window, only the newest is kept,
    /// and only while it joins the first one inside, so the line enters from the left edge.
    pub(super) fn plot(&self, window: ChartWindow) -> Vec<Option<ChartPoint>> {
        let start = window.end - window.span_secs;
        let first = self.samples.iter().rposition(|s| s.at < start).unwrap_or(0);
        let mut out: Vec<Option<ChartPoint>> = Vec::with_capacity(self.samples.len() - first);
        let mut prev_slot: Option<i64> = None;
        for s in self.samples.iter().skip(first) {
            let slot = slot_of(s.at, self.interval_secs);
            if prev_slot.is_some_and(|prev| slot - prev > MAX_SLOT_STEP) {
                push_break(&mut out);
            }
            match s.value {
                Some(value) => out.push(Some(ChartPoint { at: s.at, value })),
                None => push_break(&mut out),
            }
            prev_slot = Some(slot);
        }
        let enters_alone = matches!(out.first(), Some(Some(p)) if p.at < start)
            && !matches!(out.get(1), Some(Some(_)));
        if enters_alone {
            out.drain(..out.len().min(2));
        }
        out
    }
}

fn push_break(out: &mut Vec<Option<ChartPoint>>) {
    if matches!(out.last(), Some(Some(_))) {
        out.push(None);
    }
}

#[expect(
    clippy::integer_division,
    reason = "the slot index buckets whole seconds into a tier's interval"
)]
fn slot_of(at: i64, interval_secs: i64) -> i64 {
    at / interval_secs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const SPAN: ChartSpan = ChartSpan::FifteenMinutes;

    fn datum(at: i64, value: Option<f32>) -> HistoryDatum {
        HistoryDatum { at, value }
    }

    /// A 15-min series holding exactly `samples`, as a restore would leave it.
    fn holding(samples: &[(i64, Option<f32>)]) -> Series {
        Series::restored(
            SPAN,
            samples
                .iter()
                .map(|&(at, value)| datum(at, value))
                .collect(),
        )
    }

    #[test]
    fn a_sample_in_the_open_slot_refreshes_its_bucket_and_keeps_its_time() {
        let mut s = Series::new(SPAN);
        s.record(0, Some(4.0));
        s.record(14, Some(9.0));
        assert_eq!(s.samples(), vec![datum(0, Some(9.0))]);
    }

    #[test]
    fn a_sample_older_than_the_newest_bucket_is_dropped() {
        let mut s = Series::new(SPAN);
        s.record(20, Some(1.0));
        s.record(10, Some(2.0));
        assert_eq!(s.samples(), vec![datum(20, Some(1.0))]);
    }

    #[test]
    fn a_sample_in_a_later_slot_opens_a_bucket() {
        let mut s = Series::new(SPAN);
        s.record(0, Some(4.0));
        s.record(15, Some(9.0));
        assert_eq!(s.samples(), vec![datum(0, Some(4.0)), datum(15, Some(9.0))]);
    }

    #[test]
    fn a_live_sample_never_changes_a_restored_bucket() {
        let mut s = holding(&[(0, Some(4.0))]);
        s.record(5, None);
        assert_eq!(
            s.samples(),
            vec![datum(0, Some(4.0))],
            "a sample in the restored bucket's slot is dropped"
        );
        s.record(15, Some(9.0));
        assert_eq!(
            s.samples(),
            vec![datum(0, Some(4.0)), datum(15, Some(9.0))],
            "the next slot opens a bucket, so a restart costs no extra one"
        );
    }

    #[test]
    fn dropping_the_buckets_after_a_time_keeps_the_newest_slot_sampled() {
        let mut s = holding(&[(0, Some(1.0)), (15, Some(2.0)), (30, Some(3.0))]);
        s.drop_after(20);
        assert_eq!(s.samples(), vec![datum(0, Some(1.0)), datum(15, Some(2.0))]);
        assert!(
            s.has_sampled_slot_of(20),
            "the newest bucket's slot still counts as sampled"
        );
    }

    #[test]
    fn an_older_blob_goes_before_the_buckets_after_the_cut() {
        let mut s = Series::new(SPAN);
        s.record(50, Some(1.0));
        s.record(1_010, Some(2.0));
        s.merge_older(vec![datum(900, Some(3.0)), datum(1_000, Some(4.0))], 1_000);
        assert_eq!(
            s.samples(),
            vec![
                datum(900, Some(3.0)),
                datum(1_000, Some(4.0)),
                datum(1_010, Some(2.0))
            ],
            "the bucket at 50 s was stamped by a clock behind the blob"
        );
        s.record(1_011, Some(5.0));
        assert_eq!(
            s.samples().last(),
            Some(&datum(1_010, Some(5.0))),
            "the live bucket stays open"
        );
    }

    #[test]
    fn a_live_bucket_in_the_slot_of_the_cut_is_dropped() {
        let mut s = Series::new(SPAN);
        s.record(910, Some(1.0));
        s.merge_older(vec![datum(900, Some(3.0))], 900);
        assert_eq!(
            s.samples(),
            vec![datum(900, Some(3.0))],
            "900 and 910 share a 15 s slot"
        );
        assert!(
            s.has_sampled_slot_of(910),
            "the blob's bucket takes the slot"
        );
        s.record(912, Some(9.0));
        assert_eq!(
            s.samples(),
            vec![datum(900, Some(3.0))],
            "the blob's bucket stays closed to live samples in its slot"
        );
    }

    #[test]
    fn a_restored_break_is_never_refreshed() {
        let mut s = holding(&[(0, Some(4.0)), (5, None)]);
        s.record(10, Some(9.0));
        assert_eq!(
            s.samples(),
            vec![datum(0, Some(4.0)), datum(5, None)],
            "a restart break saved by an earlier build stays, like any restored bucket"
        );
    }

    #[test]
    fn the_oldest_bucket_is_evicted_at_capacity() {
        let mut s = Series::new(SPAN);
        for i in 0..=POINTS {
            let at = i64::try_from(i).expect("BUG: loop index fits i64") * 15;
            s.record(at, Some(1.0));
        }
        let samples = s.samples();
        assert_eq!(samples.len(), POINTS);
        assert_eq!(samples[0].at, 15, "the bucket at 0 was evicted");
    }

    #[test]
    fn a_cut_closes_the_newest_bucket() {
        let mut s = Series::new(SPAN);
        s.record(0, Some(1.0));
        s.record(15, None);
        s.record(30, Some(3.0));
        s.drop_after(20);
        s.record(20, Some(2.0));
        assert_eq!(
            s.samples(),
            vec![datum(0, Some(1.0)), datum(15, None)],
            "the sample at 20 s shares the closed bucket's slot, so it is dropped"
        );
        s.record(30, Some(3.0));
        assert_eq!(
            s.samples(),
            vec![datum(0, Some(1.0)), datum(15, None), datum(30, Some(3.0))],
            "the next slot opens a bucket"
        );
    }

    #[test]
    fn a_restored_series_out_of_time_order_is_sorted() {
        let s = holding(&[(30, Some(3.0)), (0, Some(1.0)), (15, Some(2.0))]);
        assert_eq!(
            s.samples(),
            vec![
                datum(0, Some(1.0)),
                datum(15, Some(2.0)),
                datum(30, Some(3.0))
            ],
            "an earlier build saved buckets as a stepping clock stamped them"
        );
        assert!(
            s.has_sampled_slot_of(30),
            "the newest bucket is the one whose slot counts as sampled"
        );
    }

    #[test]
    fn a_restored_series_longer_than_capacity_is_cut_to_the_newest() {
        let over: Vec<HistoryDatum> = (0..POINTS + 10)
            .map(|i| datum(i64::try_from(i).expect("BUG: loop index fits i64"), None))
            .collect();
        let samples = Series::restored(SPAN, over).samples();
        assert_eq!(samples.len(), POINTS, "cut to capacity");
        assert_eq!(
            samples[0].at, 10,
            "the oldest ten were dropped, not the newest"
        );
    }

    #[test]
    fn a_slot_counts_as_sampled_only_once_a_sample_lands_in_it() {
        let mut s = Series::new(SPAN);
        assert!(!s.has_sampled_slot_of(0), "nothing recorded yet");
        s.record(0, Some(4.0));
        assert!(s.has_sampled_slot_of(14));
        assert!(!s.has_sampled_slot_of(15), "the next slot");
        assert!(
            holding(&[(0, Some(4.0))]).has_sampled_slot_of(14),
            "a restored series counts its newest bucket's slot as sampled"
        );
        assert!(
            holding(&[(0, None)]).has_sampled_slot_of(14),
            "a restored break's slot too"
        );
    }

    /// A small deterministic generator, so the reference test replays exactly.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, n: u64) -> i64 {
            i64::try_from(self.next() % n).expect("BUG: below a u64 bound that fits i64")
        }
    }

    /// The buckets `record` must leave for time-ordered `samples`, built the slow way:
    /// one map entry per slot holding its first time and its last value,
    /// none for the newest restored bucket's slot, which stays closed,
    /// and the newest `POINTS` kept.
    fn reference(
        restored: &[HistoryDatum],
        samples: &[(i64, Option<f32>)],
        interval: i64,
    ) -> Vec<HistoryDatum> {
        let mut by_slot: BTreeMap<i64, HistoryDatum> = BTreeMap::new();
        for &(at, value) in samples {
            by_slot
                .entry(slot_of(at, interval))
                .or_insert(datum(at, value))
                .value = value;
        }
        if let Some(newest) = restored.last() {
            by_slot.remove(&slot_of(newest.at, interval));
        }
        let mut all: Vec<HistoryDatum> = restored.to_vec();
        all.extend(by_slot.into_values());
        let excess = all.len().saturating_sub(POINTS);
        all.into_iter().skip(excess).collect()
    }

    #[test]
    fn record_matches_a_slot_map_over_generated_sequences() {
        let mut rng = Lcg(0x5EED);
        for case in 0..2_000 {
            let span = ChartSpan::ALL[usize::try_from(rng.below(4)).expect("BUG: below 4")];
            let interval = span.interval_secs();
            let step = u64::try_from(interval).expect("BUG: an interval is positive");
            let restored: Vec<HistoryDatum> = (0..rng.below(4))
                .map(|i| datum(i * 7, (rng.below(3) != 0).then_some(1.0)))
                .collect();
            let restored_end = restored.last().map_or(-1, |d| d.at);
            // Start close enough to share a slot with the last restored bucket.
            let mut at = restored_end.max(0) + rng.below(step);
            let mut samples = Vec::new();
            // Enough samples, about one slot apart, to evict on every tier.
            for _ in 0..rng.below(300) {
                // Mostly within the slot or the next, sometimes an outage.
                at += if rng.below(20) == 0 {
                    rng.below(100 * step)
                } else {
                    rng.below(2 * step)
                };
                let value = (rng.below(5) != 0).then(|| {
                    #[expect(clippy::cast_precision_loss, reason = "small test values")]
                    let v = rng.below(1_000) as f32;
                    v
                });
                samples.push((at, value));
            }

            let mut series = Series::restored(span, restored.clone());
            for &(at, value) in &samples {
                series.record(at, value);
            }
            let got = series.samples();
            assert_eq!(got, reference(&restored, &samples, interval), "case {case}");
            assert!(got.len() <= POINTS, "case {case}: over capacity");
            let live_slots: Vec<i64> = got
                .iter()
                .filter(|d| d.at > restored_end)
                .map(|d| slot_of(d.at, interval))
                .collect();
            assert!(
                live_slots.windows(2).all(|w| w[0] < w[1]),
                "case {case}: a slot holds two live buckets"
            );
            assert!(
                live_slots.first().is_none_or(
                    |&first| restored.is_empty() || first > slot_of(restored_end, interval)
                ),
                "case {case}: a live bucket shares the restored bucket's slot"
            );
        }
    }

    fn present(ats: &[i64]) -> Series {
        holding(&ats.iter().map(|&at| (at, Some(1.0))).collect::<Vec<_>>())
    }

    fn breaks(plot: &[Option<ChartPoint>]) -> usize {
        plot.iter().filter(|p| p.is_none()).count()
    }

    fn ats(plot: &[Option<ChartPoint>]) -> Vec<i64> {
        plot.iter().flatten().map(|p| p.at).collect()
    }

    /// The 15-min chart's window over the first 15 minutes.
    const WINDOW: ChartWindow = ChartWindow {
        end: 900,
        span_secs: 900,
    };

    /// The 15-min chart's window over the second 15 minutes.
    const LATER_WINDOW: ChartWindow = ChartWindow {
        end: 1_800,
        span_secs: 900,
    };

    #[test]
    fn polls_up_to_two_intervals_apart_plot_one_run() {
        // A device polled every 25 s leaves one empty 15 s slot between some of its buckets.
        let s = present(&[0, 25, 50, 75, 100, 125, 150, 175]);
        assert_eq!(breaks(&s.plot(WINDOW)), 0);
    }

    #[test]
    fn one_empty_slot_between_buckets_joins_the_line() {
        assert_eq!(breaks(&present(&[0, 30]).plot(WINDOW)), 0);
    }

    #[test]
    fn two_empty_slots_between_buckets_break_the_line() {
        assert_eq!(breaks(&present(&[0, 45]).plot(WINDOW)), 1);
    }

    #[test]
    fn a_jump_over_two_intervals_breaks_the_plot() {
        let s = present(&[0, 15, 30, 45, 60, 180, 195, 210]);
        assert_eq!(
            breaks(&s.plot(WINDOW)),
            1,
            "two minutes without a sample read as downtime"
        );
    }

    #[test]
    fn samples_sparser_than_the_chart_break_between_most_buckets() {
        // A device polled every 40 s on the 15 s chart leaves two empty slots
        // between most of its buckets, and the chart shows them as missing.
        let s = present(&[0, 40, 80, 120]);
        assert_eq!(breaks(&s.plot(WINDOW)), 2);
    }

    #[test]
    fn a_bucket_without_a_value_breaks_the_plot() {
        let s = holding(&[(0, Some(1.0)), (15, None), (30, Some(2.0))]);
        assert_eq!(
            s.plot(WINDOW),
            vec![
                Some(ChartPoint { at: 0, value: 1.0 }),
                None,
                Some(ChartPoint { at: 30, value: 2.0 }),
            ]
        );
    }

    #[test]
    fn breaks_next_to_each_other_collapse_into_one() {
        let s = holding(&[
            (0, Some(1.0)),
            (15, Some(1.0)),
            (30, Some(1.0)),
            (45, None),
            (600, Some(2.0)),
        ]);
        assert_eq!(
            breaks(&s.plot(WINDOW)),
            1,
            "the empty bucket and the downtime after it"
        );
    }

    #[test]
    fn a_plot_never_starts_on_a_break() {
        let one = |at| vec![Some(ChartPoint { at, value: 1.0 })];
        assert_eq!(
            holding(&[(0, None), (15, Some(1.0))]).plot(WINDOW),
            one(15),
            "a lone point stays lone, so the scaled chart still waits for a second one"
        );
        assert_eq!(
            holding(&[(885, None), (900, Some(1.0))]).plot(LATER_WINDOW),
            one(900),
            "an entering bucket without a value leaves no break behind"
        );
    }

    #[test]
    fn the_newest_bucket_before_the_window_draws_the_line_in_from_the_edge() {
        let s = present(&[870, 885, 900, 915]);
        assert_eq!(
            ats(&s.plot(LATER_WINDOW)),
            vec![885, 900, 915],
            "885 enters the window, 870 is never drawn"
        );
    }

    #[test]
    fn a_bucket_before_the_window_cut_off_by_a_gap_is_not_plotted() {
        let s = present(&[600, 960, 975]);
        assert_eq!(s.plot(LATER_WINDOW), {
            let p = |at| Some(ChartPoint { at, value: 1.0 });
            vec![p(960), p(975)]
        });
    }

    #[test]
    fn a_series_that_ended_before_the_window_plots_nothing() {
        assert!(present(&[600, 615]).plot(LATER_WINDOW).is_empty());
    }

    #[test]
    fn a_bucket_before_the_window_followed_by_a_break_is_not_plotted() {
        let s = holding(&[(885, Some(1.0)), (900, None), (915, Some(1.0))]);
        assert_eq!(
            s.plot(LATER_WINDOW),
            vec![Some(ChartPoint {
                at: 915,
                value: 1.0
            })]
        );
    }

    #[test]
    fn plot_keeps_its_invariants_over_generated_series() {
        let mut rng = Lcg(0x917);
        for case in 0..2_000 {
            let mut at = 0;
            let samples: Vec<(i64, Option<f32>)> = (0..rng.below(61))
                .map(|_| {
                    // Up to four slots apart, so lines both join and break.
                    at += 1 + rng.below(60);
                    (at, (rng.below(4) != 0).then_some(1.0))
                })
                .collect();
            let window = ChartWindow {
                end: at + rng.below(600),
                span_secs: 900,
            };
            let start = window.end - window.span_secs;
            let plot = holding(&samples).plot(window);
            let drawn = ats(&plot);

            assert!(
                !matches!(plot.first(), Some(None)),
                "case {case}: starts on a break"
            );
            assert!(
                plot.windows(2).all(|w| w[0].is_some() || w[1].is_some()),
                "case {case}: two breaks in a row"
            );
            assert!(
                drawn.windows(2).all(|w| w[0] < w[1]),
                "case {case}: out of time order"
            );
            assert!(
                samples
                    .iter()
                    .filter(|&&(at, value)| at >= start && value.is_some())
                    .all(|(at, _)| drawn.contains(at)),
                "case {case}: a point inside the window is missing"
            );
            let before: Vec<usize> = plot
                .iter()
                .enumerate()
                .filter(|(_, p)| matches!(p, Some(q) if q.at < start))
                .map(|(i, _)| i)
                .collect();
            assert!(
                before.is_empty() || (before == [0] && matches!(plot.get(1), Some(Some(_)))),
                "case {case}: a point before the window is not a line entering from the edge"
            );
        }
    }
}

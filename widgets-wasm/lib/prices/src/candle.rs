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

//! The Nexus candle envelope and its host-pure parser.

use crate::format::push_uint;

/// One OHLC bucket. `t_secs` is the bucket start in seconds (wire ms ÷ 1000).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandleBar {
    pub t_secs: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: Option<f64>,
}

/// A parsed series, ascending by `t_secs`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candles {
    pub bars: Vec<CandleBar>,
    /// The currency the series is priced in, when the source reports it
    /// (e.g. `USD`). `None` when the wire omits it.
    pub quote_currency: Option<String>,
}

/// Host-testable lookup over a parsed JSON document. The wasm build implements
/// it for [`bmc_wasm_sdk::json::JsonDoc`]; tests implement it for a map.
pub trait JsonLookup {
    fn str(&self, path: &str) -> Option<String>;
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
    /// Defaults to `None` (no boolean at `path`) so lookups over documents
    /// without booleans need not implement it.
    fn bool(&self, _path: &str) -> Option<bool> {
        None
    }
}

/// Cap on parsed candles, guarding a malformed/oversized response. When a
/// response carries more, the newest bars are kept.
pub const MAX_CANDLES: usize = 2_048;

/// Hard bound on parse-loop iterations, guarding unbounded work on a
/// malformed response. Payloads that extend beyond it are rejected.
const PARSE_GUARD: usize = 4096;

/// Parse the Nexus envelope `{ data: { instrument, candle_size, candles: [{ t,
/// o, h, l, c, v? }] }, ttl_secs }` into ascending [`Candles`]. `t` is divided
/// by 1000 (ms → s). Returns `None` if no usable candle is found.
///
/// Candles arrive in a contiguous ascending array, so a wholly missing bucket
/// marks the end. A present bucket missing a usable timestamp or required OHLC
/// field is skipped and parsing continues. `v` is optional.
#[must_use]
pub fn parse_candles(json: &impl JsonLookup) -> Option<Candles> {
    let mut bars = Vec::new();
    let mut path = String::new();
    let mut reached_end = false;
    for i in 0..PARSE_GUARD {
        candle_path(&mut path, i, "t");
        let Some(t_ms) = timestamp_ms(json, &path) else {
            candle_path(&mut path, i, "c");
            if json.f64(&path).is_some() {
                continue;
            }
            reached_end = true;
            break;
        };
        candle_path(&mut path, i, "o");
        let open = json.f64(&path);
        candle_path(&mut path, i, "h");
        let high = json.f64(&path);
        candle_path(&mut path, i, "l");
        let low = json.f64(&path);
        candle_path(&mut path, i, "c");
        let close = json.f64(&path);
        let (Some(open), Some(high), Some(low), Some(close)) = (open, high, low, close) else {
            // Present but incomplete bucket: skip it, keep reading the rest.
            continue;
        };
        candle_path(&mut path, i, "v");
        let volume = json.f64(&path);
        bars.push(CandleBar {
            t_secs: t_ms / 1_000,
            open,
            high,
            low,
            close,
            volume,
        });
    }

    if !reached_end {
        candle_path(&mut path, PARSE_GUARD, "t");
        let has_timestamp = timestamp_ms(json, &path).is_some();
        candle_path(&mut path, PARSE_GUARD, "c");
        if has_timestamp || json.f64(&path).is_some() {
            return None;
        }
    }

    if bars.len() > MAX_CANDLES {
        bars.drain(..bars.len() - MAX_CANDLES);
    }

    if bars.is_empty() {
        None
    } else {
        Some(Candles {
            bars,
            quote_currency: json.str("/data/quote_currency").filter(|c| !c.is_empty()),
        })
    }
}

fn timestamp_ms(json: &impl JsonLookup, path: &str) -> Option<i64> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

    json.i64(path).or_else(|| {
        let value = json.f64(path)?;
        if value.is_finite()
            && value.fract() == 0.0
            && (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value)
        {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "finite integral value is bounded to f64's exact integer range"
            )]
            Some(value as i64)
        } else {
            None
        }
    })
}

/// Build `/data/candles/<i>/<field>` into `buf` without an allocating format
/// macro (this runs on the wasm render path, where the `no-fmt-in-wasm` gate
/// bans `std::format!`).
fn candle_path(buf: &mut String, index: usize, field: &str) {
    buf.clear();
    buf.push_str("/data/candles/");
    push_uint(
        buf,
        u64::try_from(index).expect("BUG: candle index fits u64"),
    );
    buf.push('/');
    buf.push_str(field);
}

#[cfg(target_arch = "wasm32")]
impl JsonLookup for bmc_wasm_sdk::json::JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        self.str(path)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        self.i64(path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }

    fn bool(&self, path: &str) -> Option<bool> {
        self.bool(path)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<String, String>,
        pub(crate) ints: BTreeMap<String, i64>,
        pub(crate) floats: BTreeMap<String, f64>,
        pub(crate) bools: BTreeMap<String, bool>,
    }

    impl MapJson {
        /// Insert a full OHLC bucket at `index` (t in ms, as the wire carries).
        /// Uses the production path builder so the keys match `parse_candles`
        /// and no `format!` macro trips the `no-fmt-in-wasm` gate.
        pub(crate) fn bar(&mut self, index: usize, t_ms: i64, ohlc: f64) {
            let mut key = String::new();
            super::candle_path(&mut key, index, "t");
            self.ints.insert(key.clone(), t_ms);
            for field in ["o", "h", "l", "c"] {
                super::candle_path(&mut key, index, field);
                self.floats.insert(key.clone(), ohlc);
            }
        }
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).cloned()
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }

        fn bool(&self, path: &str) -> Option<bool> {
            self.bools.get(path).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::MapJson;
    use super::*;

    #[test]
    fn parses_ascending_bars_and_converts_ms_to_seconds() {
        let mut json = MapJson::default();
        json.strings
            .insert("/data/quote_currency".into(), "USD".into());
        json.bar(0, 1_000_000, 100.0);
        json.bar(1, 1_003_600_000, 101.0);
        json.floats.insert("/data/candles/1/v".into(), 5.0);

        let candles = parse_candles(&json).expect("BUG: fixture has two valid bars");
        assert_eq!(candles.quote_currency.as_deref(), Some("USD"));
        assert_eq!(candles.bars.len(), 2);
        assert_eq!(candles.bars[0].t_secs, 1_000); // 1_000_000 ms → 1000 s
        assert_eq!(candles.bars[0].volume, None);
        assert_eq!(candles.bars[1].volume, Some(5.0));
        assert_eq!(candles.bars.last().map(|b| b.close), Some(101.0));
    }

    #[test]
    fn an_interior_incomplete_bucket_is_skipped_not_truncated() {
        // A single bad bucket in the middle must not drop the rest of the
        // series.
        let mut json = MapJson::default();
        json.bar(0, 0, 10.0);
        json.bar(1, 3_600_000, 11.0);
        // index 2: t present, but the close is missing → skip
        json.ints.insert("/data/candles/2/t".into(), 7_200_000);
        json.bar(3, 10_800_000, 13.0);

        let candles = parse_candles(&json).expect("BUG: fixture has three valid bars");
        assert_eq!(candles.bars.len(), 3);
        assert_eq!(candles.bars.last().map(|b| b.close), Some(13.0));
    }

    #[test]
    fn float_encoded_timestamp_and_timestamp_gap_do_not_truncate_the_tail() {
        let mut json = MapJson::default();
        json.bar(0, 0, 10.0);
        json.bar(1, 3_600_000, 11.0);
        json.ints.remove("/data/candles/1/t");
        json.floats
            .insert("/data/candles/1/t".to_owned(), 3_600_000.0);
        json.floats.insert("/data/candles/2/o".to_owned(), 12.0);
        json.floats.insert("/data/candles/2/h".to_owned(), 12.0);
        json.floats.insert("/data/candles/2/l".to_owned(), 12.0);
        json.floats.insert("/data/candles/2/c".to_owned(), 12.0);
        json.bar(3, 10_800_000, 13.0);

        let candles = parse_candles(&json).expect("BUG: valid tail must survive");
        assert_eq!(candles.bars.len(), 3);
        assert_eq!(candles.bars[1].t_secs, 3_600);
        assert_eq!(candles.bars.last().map(|bar| bar.close), Some(13.0));
    }

    #[test]
    fn missing_t_marks_the_end_of_the_array() {
        let mut json = MapJson::default();
        json.bar(0, 0, 10.0);
        json.bar(1, 3_600_000, 11.0);
        // no index 2 at all → stop here
        let candles = parse_candles(&json).expect("BUG: fixture has two bars");
        assert_eq!(candles.bars.len(), 2);
    }

    #[test]
    fn empty_response_is_none() {
        let json = MapJson::default();
        assert_eq!(parse_candles(&json), None);
    }

    #[test]
    fn cap_keeps_the_newest_candles() {
        // An oversized response must drop the oldest bars, not the live
        // tail: the chart and the current price come from the series end.
        let mut json = MapJson::default();
        for i in 0..(MAX_CANDLES + 50) {
            json.bar(i, i as i64 * 1_000, 1.0);
        }
        let candles = parse_candles(&json).expect("BUG: fixture parses to capped bars");
        assert_eq!(candles.bars.len(), MAX_CANDLES);
        assert_eq!(candles.bars[0].t_secs, 50);
        assert_eq!(
            candles
                .bars
                .last()
                .expect("BUG: just asserted non-empty")
                .t_secs,
            i64::try_from(MAX_CANDLES + 50 - 1).expect("BUG: small constant")
        );
    }

    #[test]
    fn payload_beyond_parse_guard_is_rejected() {
        let mut json = MapJson::default();
        for i in 0..=PARSE_GUARD {
            json.bar(i, i as i64 * 1_000, 1.0);
        }
        assert_eq!(parse_candles(&json), None);
    }

    #[test]
    fn full_history_preserves_2_048_monthly_candles() {
        const EXPECTED_HISTORY: usize = 2_048;

        let mut json = MapJson::default();
        for i in 0..EXPECTED_HISTORY {
            json.bar(i, i as i64 * 1_000, 1.0);
        }

        let candles = parse_candles(&json).expect("BUG: full-history fixture has valid bars");
        assert_eq!(candles.bars.len(), EXPECTED_HISTORY);
        assert_eq!(candles.bars.first().map(|bar| bar.t_secs), Some(0));
        assert_eq!(candles.bars.last().map(|bar| bar.t_secs), Some(2_047));
    }
}

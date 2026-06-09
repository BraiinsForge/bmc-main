// Copyright (C) 2026  Braiins Systems s.r.o.

//! The Nexus candle envelope, its host-pure parser, and the market-open
//! recency heuristic.

use crate::period::Candle;

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
    pub instrument: String,
    pub candle_size: String,
    pub bars: Vec<CandleBar>,
}

impl Candles {
    /// The current price: the last bucket close.
    #[must_use]
    pub fn current_price(&self) -> Option<f64> {
        self.bars.last().map(|b| b.close)
    }
}

/// Host-testable lookup over a parsed JSON document. The wasm build implements
/// it for [`bmc_wasm_sdk::json::JsonDoc`]; tests implement it for a map.
pub trait JsonLookup {
    fn str(&self, path: &str) -> Option<String>;
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}

/// Cap on parsed candles, guarding a malformed/oversized response.
pub const MAX_CANDLES: usize = 512;

/// Parse the Nexus envelope `{ data: { instrument, candle_size, candles: [{ t,
/// o, h, l, c, v? }] }, ttl_secs }` into ascending [`Candles`]. `t` is divided
/// by 1000 (ms → s). Returns `None` if no usable candle is found.
///
/// Candles arrive in a contiguous ascending array, so a missing `t` at index
/// `i` marks the end. A candle present but missing a required OHLC field is
/// skipped and parsing continues (matching deckfeeder, which skips NaN entries
/// rather than truncating). `v` is optional.
#[must_use]
pub fn parse_candles(json: &impl JsonLookup) -> Option<Candles> {
    let instrument = json.str("/data/instrument").unwrap_or_default();
    let candle_size = json.str("/data/candle_size").unwrap_or_default();

    let mut bars = Vec::new();
    let mut path = String::new();
    for i in 0..MAX_CANDLES {
        candle_path(&mut path, i, "t");
        let Some(t_ms) = json.i64(&path) else {
            // End of the contiguous candle array.
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

    if bars.is_empty() {
        None
    } else {
        Some(Candles {
            instrument,
            candle_size,
            bars,
        })
    }
}

/// Market-open recency heuristic: the freshest candle is younger than ~2.5
/// bucket widths. `now_secs` is the current wall clock in seconds. Returns
/// `false` for an empty series. Mirrors deckfeeder's
/// `now - lastTs < intervalSeconds * 2.5`.
#[must_use]
pub fn market_open(bars: &[CandleBar], candle: Candle, now_secs: i64) -> bool {
    match bars.last() {
        Some(last) => {
            #[expect(
                clippy::cast_precision_loss,
                reason = "bucket widths and candle ages are small; f64 is exact enough for a 2.5x threshold"
            )]
            {
                let age_secs = (now_secs - last.t_secs) as f64;
                let threshold = candle.width_secs() as f64 * 2.5;
                age_secs < threshold
            }
        }
        None => false,
    }
}

/// Build `/data/candles/<i>/<field>` into `buf` without an allocating format
/// macro (this runs on the wasm render path, where the `no-fmt-in-wasm` gate
/// bans `std::format!`).
fn candle_path(buf: &mut String, index: usize, field: &str) {
    buf.clear();
    buf.push_str("/data/candles/");
    push_usize(buf, index);
    buf.push('/');
    buf.push_str(field);
}

fn push_usize(buf: &mut String, mut n: usize) {
    if n == 0 {
        buf.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + u8::try_from(n % 10).expect("BUG: n % 10 is a single decimal digit");
        n /= 10;
    }
    for &d in &digits[i..] {
        buf.push(char::from(d));
    }
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
            .insert("/data/instrument".into(), "BTC/USD".into());
        json.strings.insert("/data/candle_size".into(), "1h".into());
        json.bar(0, 1_000_000, 100.0);
        json.bar(1, 1_003_600_000, 101.0);
        json.floats.insert("/data/candles/1/v".into(), 5.0);

        let candles = parse_candles(&json).expect("two valid bars");
        assert_eq!(candles.instrument, "BTC/USD");
        assert_eq!(candles.candle_size, "1h");
        assert_eq!(candles.bars.len(), 2);
        assert_eq!(candles.bars[0].t_secs, 1_000); // 1_000_000 ms → 1000 s
        assert_eq!(candles.bars[0].volume, None);
        assert_eq!(candles.bars[1].volume, Some(5.0));
        assert_eq!(candles.current_price(), Some(101.0));
    }

    #[test]
    fn an_interior_incomplete_bucket_is_skipped_not_truncated() {
        // A single bad bucket in the middle must NOT drop the rest of the
        // series — deckfeeder skips NaN entries and keeps going.
        let mut json = MapJson::default();
        json.bar(0, 0, 10.0);
        json.bar(1, 3_600_000, 11.0);
        // index 2: t present, but the close is missing → skip
        json.ints.insert("/data/candles/2/t".into(), 7_200_000);
        json.bar(3, 10_800_000, 13.0);

        let candles = parse_candles(&json).expect("three valid bars");
        assert_eq!(candles.bars.len(), 3);
        assert_eq!(candles.current_price(), Some(13.0));
    }

    #[test]
    fn missing_t_marks_the_end_of_the_array() {
        let mut json = MapJson::default();
        json.bar(0, 0, 10.0);
        json.bar(1, 3_600_000, 11.0);
        // no index 2 at all → stop here
        let candles = parse_candles(&json).expect("two bars");
        assert_eq!(candles.bars.len(), 2);
    }

    #[test]
    fn empty_response_is_none() {
        let json = MapJson::default();
        assert_eq!(parse_candles(&json), None);
    }

    #[test]
    fn caps_at_max_candles() {
        let mut json = MapJson::default();
        for i in 0..(MAX_CANDLES + 50) {
            json.bar(i, i as i64 * 1_000, 1.0);
        }
        let candles = parse_candles(&json).expect("capped bars");
        assert_eq!(candles.bars.len(), MAX_CANDLES);
    }

    #[test]
    fn market_open_distinguishes_a_live_feed_from_a_frozen_market() {
        // The freshest candle within 2.5 bucket widths means new data is still
        // arriving (live); older than that means the feed froze (closed market).
        let candle = Candle::H1; // 3600 s buckets
        let now = 1_000_000;
        let fresh = [CandleBar {
            t_secs: now - 3_600, // one bucket old → live
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: None,
        }];
        let stale = [CandleBar {
            t_secs: now - 3_600 * 3, // three buckets old → frozen
            ..fresh[0]
        }];
        assert!(market_open(&fresh, candle, now));
        assert!(!market_open(&stale, candle, now));
        assert!(!market_open(&[], candle, now));
    }
}

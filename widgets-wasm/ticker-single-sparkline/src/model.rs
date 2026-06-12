// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host-pure data model: the price series, the change/price formatting, and the
//! currency-icon table. No SDK draw types, so it all unit-tests on the host.

use prices::candle::{self, Candles};
use prices::instrument::base_symbol;
use prices::period::Candle;

/// The rendered series: the close line, the current price, the period change,
/// and whether the feed looks live.
pub struct Series {
    pub closes: Vec<f64>,
    pub current: f64,
    pub change_pct: f64,
    pub market_open: bool,
}

impl Series {
    /// Build from parsed candles. `change_pct` is the first-to-last close
    /// move; `market_open` is the recency heuristic against `now_secs`, using
    /// `liveness` as the staleness bucket (the period's update cadence, not the
    /// chart candle). Returns `None` if there are no candles or the first close
    /// is non-positive (which would make `change_pct` non-finite).
    #[must_use]
    pub fn from_candles(candles: &Candles, liveness: Candle, now_secs: i64) -> Option<Series> {
        let first = candles.bars.first()?.close;
        if first <= 0.0 {
            return None;
        }
        let closes: Vec<f64> = candles.bars.iter().map(|b| b.close).collect();
        let current = *closes.last()?;
        let change_pct = (current - first) / first * 100.0;
        let market_open = candle::market_open(&candles.bars, liveness, now_secs);
        Some(Series {
            closes,
            current,
            change_pct,
            market_open,
        })
    }

    #[must_use]
    pub fn is_positive(&self) -> bool {
        self.change_pct >= 0.0
    }
}

// Price/percent formatting is shared with ticker-list, so it lives in
// `lib/prices::format`. Re-exported here so the render path's call sites stay
// unchanged.
pub use prices::format::{MIN_PRICE, PricePrecision, change_text, price_precision};

/// The currency icon: a glyph and a representative solid disc color. `None` for
/// `^`-prefixed indices (which deckfeeder renders without an icon).
pub struct IconStyle {
    pub glyph: String,
    pub rgb: (u8, u8, u8),
}

/// Pick the icon for a symbol. Known bases map to a glyph + disc color (the
/// gradient's start color); an unknown base falls back to its first character
/// on a neutral gray; a `^`-prefixed index has no icon. Mirrors deckfeeder's
/// `getCurrencyIcon` / `SYMBOL_ICONS`.
#[must_use]
pub fn icon_for(symbol: &str) -> Option<IconStyle> {
    if symbol.starts_with('^') {
        return None;
    }
    let base = base_symbol(symbol);
    let known: Option<(&str, (u8, u8, u8))> = match base.as_str() {
        "BTC" => Some(("\u{20bf}", (0xf7, 0x93, 0x1a))),
        "ETH" => Some(("\u{39e}", (0x62, 0x7e, 0xea))),
        "USD" => Some(("$", (0x85, 0xbb, 0x65))),
        "EUR" => Some(("\u{20ac}", (0x00, 0x33, 0x99))),
        "GBP" => Some(("\u{a3}", (0x01, 0x21, 0x69))),
        "JPY" => Some(("\u{a5}", (0xbc, 0x00, 0x2d))),
        "CHF" => Some(("F", (0xd5, 0x2b, 0x1e))),
        "AUD" => Some(("$", (0x00, 0x00, 0x8b))),
        "CAD" => Some(("$", (0xff, 0x00, 0x00))),
        "CNY" => Some(("\u{a5}", (0xde, 0x29, 0x10))),
        "INR" => Some(("\u{20b9}", (0xff, 0x99, 0x33))),
        "CZK" => Some(("K\u{10d}", (0x11, 0x45, 0x7e))),
        "PLN" => Some(("z\u{142}", (0xdc, 0x14, 0x3c))),
        _ => None,
    };
    Some(match known {
        Some((glyph, rgb)) => IconStyle {
            glyph: glyph.to_owned(),
            rgb,
        },
        None => IconStyle {
            glyph: base.chars().next().map(String::from).unwrap_or_default(),
            rgb: (0x66, 0x66, 0x66),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prices::candle::CandleBar;

    fn bars(closes: &[f64], step: i64) -> Candles {
        let bars = closes
            .iter()
            .enumerate()
            .map(|(i, &c)| CandleBar {
                t_secs: i as i64 * step,
                open: c,
                high: c,
                low: c,
                close: c,
                volume: None,
            })
            .collect();
        Candles {
            instrument: "BTC/USD".into(),
            candle_size: "1h".into(),
            bars,
        }
    }

    #[test]
    fn series_current_is_last_close_and_change_is_first_to_last() {
        let s = Series::from_candles(&bars(&[100.0, 110.0], 3_600), Candle::H1, 9_999_999_999)
            .expect("two bars");
        assert!((s.current - 110.0).abs() < 1e-9);
        assert!((s.change_pct - 10.0).abs() < 1e-9);
        assert!(s.is_positive());
    }

    #[test]
    fn falling_series_is_not_positive() {
        let s = Series::from_candles(&bars(&[100.0, 90.0], 3_600), Candle::H1, 9_999_999_999)
            .expect("two bars");
        assert!((s.change_pct - (-10.0)).abs() < 1e-9);
        assert!(!s.is_positive());
    }

    #[test]
    fn flat_series_is_zero_change_and_positive() {
        let s = Series::from_candles(&bars(&[100.0, 100.0], 3_600), Candle::H1, 9_999_999_999)
            .expect("two bars");
        assert!(s.change_pct.abs() < 1e-9);
        assert!(s.is_positive());
    }

    #[test]
    fn non_positive_first_close_is_rejected() {
        assert!(Series::from_candles(&bars(&[0.0, 5.0], 3_600), Candle::H1, 0).is_none());
        assert!(Series::from_candles(&bars(&[-1.0, 5.0], 3_600), Candle::H1, 0).is_none());
    }

    #[test]
    fn icon_known_base_unknown_base_and_index() {
        let btc = icon_for("BTC-USD").expect("known");
        assert_eq!(btc.glyph, "\u{20bf}");
        assert_eq!(btc.rgb, (0xf7, 0x93, 0x1a));

        let goog = icon_for("GOOG").expect("fallback");
        assert_eq!(goog.glyph, "G");
        assert_eq!(goog.rgb, (0x66, 0x66, 0x66));

        assert!(icon_for("^GSPC").is_none());
    }
}

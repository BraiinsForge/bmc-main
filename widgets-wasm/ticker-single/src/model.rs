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

//! Host-pure data model: the price series, the change/price formatting, and the
//! currency-icon table. No SDK draw types, so it all unit-tests on the host.

use prices::candle::{CandleBar, Candles};
use prices::instrument::{base_symbol, split_pair};

/// The rendered series: the OHLCV bars, the current price, the period change,
/// the quote currency, and whether the market is open.
pub struct Series {
    pub bars: Vec<CandleBar>,
    pub current: f64,
    pub change_pct: f64,
    /// The currency the prices are in, when Nexus reports it. Shown next to
    /// the symbol so a bare code (`BTC`) still names its pricing currency.
    pub quote_currency: Option<String>,
    pub market_open: bool,
}

impl Series {
    /// Build from parsed candles. `change_pct` is the move from the first
    /// candle's open (the window's start price) to the last candle's close,
    /// computed via the shared [`prices::format::price_change`] helper;
    /// `market_open` comes from the Nexus envelope — only an explicit `false`
    /// marks the market closed, an omitted flag counts as open. Returns `None`
    /// only when the candles list is empty.
    #[must_use]
    pub fn from_candles(candles: Candles) -> Option<Series> {
        let first = candles.bars.first()?.open;
        let current = candles.bars.last()?.close;
        let change_pct = prices::format::price_change(first, current);
        Some(Series {
            bars: candles.bars,
            current,
            change_pct,
            quote_currency: candles.quote_currency,
            market_open: true,
        })
    }

    pub fn set_market_open(&mut self, is_market_open: Option<bool>) {
        self.market_open = is_market_open != Some(false);
    }

    /// The sparkline price series: the window's opening price (the first
    /// candle's open, the same anchor as `change_pct`) followed by each
    /// candle's close, so the line starts where the change badge measures from.
    #[must_use]
    pub fn sparkline_series(&self) -> Vec<f64> {
        let mut points = Vec::with_capacity(self.bars.len() + 1);
        if let Some(first) = self.bars.first() {
            points.push(first.open);
        }
        points.extend(self.bars.iter().map(|b| b.close));
        points
    }

    #[must_use]
    pub fn is_positive(&self) -> bool {
        self.change_pct >= 0.0
    }

    /// A market-closed instrument that is not a `^`-prefixed index is dimmed
    /// with a stop marker. `Series` has no symbol, so the caller passes it.
    #[must_use]
    pub fn is_closed_marked(&self, symbol: &str) -> bool {
        !self.market_open && !symbol.starts_with('^')
    }

    /// The header symbol and the dim quote-currency tag beside it. A pair
    /// splits into its own base and quote (`BTC-USD` → `BTC` + `USD`); a bare
    /// code keeps its name and tags the payload's quote currency, when
    /// reported — so the pricing currency reads the same either way.
    #[must_use]
    pub fn header_symbol<'a>(&'a self, symbol: &'a str) -> (&'a str, Option<&'a str>) {
        match split_pair(symbol) {
            Some((base, quote)) => (base, Some(quote)),
            None => (symbol, self.quote_currency.as_deref()),
        }
    }
}

pub use prices::format::{MIN_PRICE, PricePrecision, change_text, price_precision};

/// The currency icon: a glyph and a representative solid disc color. `None` for
/// `^`-prefixed indices, which have no icon.
pub struct IconStyle {
    pub glyph: String,
    pub rgb: (u8, u8, u8),
}

/// Pick the icon for a symbol. Known bases map to a glyph + disc color (the
/// gradient's start color); an unknown base falls back to its first character
/// on a neutral gray; a `^`-prefixed index has no icon.
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
            bars,
            quote_currency: None,
        }
    }

    #[test]
    fn series_current_is_last_close_and_change_is_first_to_last() {
        let s = Series::from_candles(bars(&[100.0, 110.0], 3_600))
            .expect("BUG: two bars build a series");
        assert!((s.current - 110.0).abs() < 1e-9);
        assert!((s.change_pct - 10.0).abs() < 1e-9);
        assert!(s.is_positive());
        assert_eq!(s.bars.len(), 2);
        // open[0]==close[0]==100 here, so the leading open just repeats.
        assert_eq!(s.sparkline_series(), vec![100.0, 100.0, 110.0]);
    }

    #[test]
    fn series_reuses_the_parsed_bar_allocation() {
        let candles = bars(&[100.0, 110.0], 3_600);
        let parsed_bars = candles.bars.as_ptr();
        let s = Series::from_candles(candles).expect("BUG: two bars build a series");

        assert_eq!(s.bars.as_ptr(), parsed_bars);
    }

    #[test]
    fn sparkline_series_leads_with_the_window_open() {
        // The line must start at the first candle's open (the same anchor as
        // the change badge), then follow each candle's close.
        let candles = Candles {
            bars: vec![
                CandleBar {
                    t_secs: 0,
                    open: 90.0,
                    high: 105.0,
                    low: 88.0,
                    close: 100.0,
                    volume: None,
                },
                CandleBar {
                    t_secs: 3_600,
                    open: 100.0,
                    high: 115.0,
                    low: 99.0,
                    close: 110.0,
                    volume: None,
                },
            ],
            quote_currency: None,
        };
        let s = Series::from_candles(candles).expect("BUG: candles build a series");
        assert_eq!(s.sparkline_series(), vec![90.0, 100.0, 110.0]);
    }

    #[test]
    fn change_anchors_on_first_open_not_first_close() {
        // The first candle opens at 100 and closes at 120; the period change
        // must measure from the open (the window's start price), so the badge
        // reads 100 -> 150 (+50%), not 120 -> 150 (+25%).
        let candles = Candles {
            bars: vec![
                CandleBar {
                    t_secs: 0,
                    open: 100.0,
                    high: 130.0,
                    low: 90.0,
                    close: 120.0,
                    volume: None,
                },
                CandleBar {
                    t_secs: 3_600,
                    open: 120.0,
                    high: 160.0,
                    low: 115.0,
                    close: 150.0,
                    volume: None,
                },
            ],
            quote_currency: None,
        };
        let s = Series::from_candles(candles).expect("BUG: candles build a series");
        assert!((s.change_pct - 50.0).abs() < 1e-9);
        assert!((s.current - 150.0).abs() < 1e-9);
    }

    #[test]
    fn falling_series_is_not_positive() {
        let s = Series::from_candles(bars(&[100.0, 90.0], 3_600))
            .expect("BUG: two bars build a series");
        assert!((s.change_pct - (-10.0)).abs() < 1e-9);
        assert!(!s.is_positive());
    }

    #[test]
    fn flat_series_is_zero_change_and_positive() {
        let s = Series::from_candles(bars(&[100.0, 100.0], 3_600))
            .expect("BUG: two bars build a series");
        assert!(s.change_pct.abs() < 1e-9);
        assert!(s.is_positive());
    }

    #[test]
    fn non_positive_first_close_keeps_the_row() {
        let zero =
            Series::from_candles(bars(&[0.0, 5.0], 3_600)).expect("BUG: bars build a series");
        assert!((zero.change_pct - 0.0).abs() < 1e-9);
        assert!((zero.current - 5.0).abs() < 1e-9);
        let neg =
            Series::from_candles(bars(&[-1.0, 5.0], 3_600)).expect("BUG: bars build a series");
        assert!(neg.change_pct.is_finite());
    }

    #[test]
    fn only_an_explicit_false_reads_as_closed() {
        // Per the Nexus contract an omitted flag must count as open — a
        // provider that cannot report market state must never dim the tile.
        let open =
            Series::from_candles(bars(&[1.0, 2.0], 3_600)).expect("BUG: bars build a series");
        assert!(open.market_open);

        let mut closed =
            Series::from_candles(bars(&[1.0, 2.0], 3_600)).expect("BUG: bars build a series");
        closed.set_market_open(Some(false));
        assert!(!closed.market_open);
    }

    #[test]
    fn header_symbol_names_the_quote_for_pairs_and_bare_codes_alike() {
        // The pricing currency must read the same whether the user typed the
        // pair or a bare code: a pair supplies its own quote tag, a bare code
        // borrows the payload's, and the `-` never reaches the screen.
        let mut candles = bars(&[1.0, 2.0], 3_600);
        candles.quote_currency = Some("USD".to_owned());
        let s = Series::from_candles(candles).expect("BUG: bars build a series");
        assert_eq!(s.header_symbol("BTC-USD"), ("BTC", Some("USD")));
        assert_eq!(s.header_symbol("BTC"), ("BTC", Some("USD")));
        assert_eq!(s.header_symbol("BRK-B"), ("BRK-B", Some("USD")));

        let bare =
            Series::from_candles(bars(&[1.0, 2.0], 3_600)).expect("BUG: bars build a series");
        assert_eq!(bare.header_symbol("BTC"), ("BTC", None));
        assert_eq!(bare.header_symbol("ETH-EUR"), ("ETH", Some("EUR")));
    }

    #[test]
    fn closed_marked_excludes_indices() {
        let mut s =
            Series::from_candles(bars(&[1.0, 2.0], 3_600)).expect("BUG: bars build a series");
        s.set_market_open(Some(false));
        assert!(s.is_closed_marked("AAPL"));
        assert!(!s.is_closed_marked("^GSPC"));
    }

    #[test]
    fn icon_known_base_unknown_base_and_index() {
        let btc = icon_for("BTC-USD").expect("BUG: BTC-USD has a known icon");
        assert_eq!(btc.glyph, "\u{20bf}");
        assert_eq!(btc.rgb, (0xf7, 0x93, 0x1a));

        let goog = icon_for("GOOG").expect("BUG: a base symbol has a fallback icon");
        assert_eq!(goog.glyph, "G");
        assert_eq!(goog.rgb, (0x66, 0x66, 0x66));

        assert!(icon_for("^GSPC").is_none());
    }
}

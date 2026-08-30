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

//! Host-pure per-row data model and the company-name truncation. No SDK draw
//! types, so it all unit-tests on the host.

use prices::candle::Candles;
use prices::fetch::PriceMiss;

/// One row's lifecycle. The company name is **not** held here — it lives in a
/// parallel names cache so a period change (which resets price state) cannot
/// drop already-fetched names.
pub enum RowState {
    /// No price reply yet.
    Loading,
    /// Price series parsed. Whether the held series is still current is not
    /// tracked here — the render reads the row poll's `is_stale` grace, the
    /// same staleness definition ticker-single uses.
    Resolved { data: TickerRow },
    /// 404/400 — symbol not found / invalid; the row keeps polling normally.
    /// Nothing to draw, remembering why so that a later reference verdict
    /// may overturn a missing resource but never a refused request.
    InputError { miss: PriceMiss },
    /// The instrument resolved but the window has no candles; the row keeps
    /// polling, so it fills in when the market reopens.
    NoData { market_closed: bool },
    /// Any failed reply (including 503) with nothing ever loaded for this
    /// row; the poll keeps running, so the row recovers on a later reply.
    Failed,
}

pub struct TickerRow {
    pub symbol: String,
    pub price: f64,
    pub change_pct: f64,
    /// The sparkline series: the window's opening price followed by each
    /// candle's close, so the line starts where `change_pct` is measured from.
    pub series: Vec<f64>,
    pub market_open: bool,
}

impl TickerRow {
    /// Build from parsed candles. Returns `None` only when there are no usable
    /// candles. `change_pct` is the move from the first candle's open (the
    /// window's start price) to the last close, via
    /// [`prices::format::price_change`]: a zero start price yields `0.0`; a
    /// negative one (malformed data) yields a finite best-effort percentage.
    /// `market_open` comes from the Nexus envelope — only an explicit `false`
    /// marks the market closed, an omitted flag counts as open.
    #[must_use]
    pub fn from_candles(symbol: &str, candles: &Candles) -> Option<TickerRow> {
        let first = candles.bars.first()?.open;
        let closes: Vec<f64> = candles.bars.iter().map(|b| b.close).collect();
        let current = *closes.last()?;
        let change_pct = prices::format::price_change(first, current);
        let mut series = Vec::with_capacity(closes.len() + 1);
        series.push(first);
        series.extend_from_slice(&closes);
        Some(TickerRow {
            symbol: symbol.to_owned(),
            price: current,
            change_pct,
            series,
            market_open: true,
        })
    }

    pub fn set_market_open(&mut self, is_market_open: Option<bool>) {
        self.market_open = is_market_open != Some(false);
    }

    #[must_use]
    pub fn is_positive(&self) -> bool {
        self.change_pct >= 0.0
    }

    /// A closed non-index gets a pause marker and a grey sparkline.
    #[must_use]
    pub fn is_closed_marked(&self) -> bool {
        !self.market_open && !self.symbol.starts_with('^')
    }
}

/// Truncate a company name to `max_chars` characters, appending an ellipsis
/// when it overflows (the SDK has no text-overflow ellipsis). Unicode-safe.
#[must_use]
pub fn truncate_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = name.chars().take(keep).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use prices::candle::CandleBar;

    fn candles(closes: &[f64], step: i64) -> Candles {
        Candles {
            bars: closes
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
                .collect(),
            quote_currency: None,
        }
    }

    #[test]
    fn row_change_is_first_to_last_and_sign_drives_positive() {
        let up = TickerRow::from_candles("AAPL", &candles(&[100.0, 110.0], 3_600))
            .expect("BUG: candles build a row");
        assert!((up.change_pct - 10.0).abs() < 1e-9);
        assert!(up.is_positive());
        let down = TickerRow::from_candles("AAPL", &candles(&[100.0, 90.0], 3_600))
            .expect("BUG: candles build a row");
        assert!(!down.is_positive());
    }

    #[test]
    fn change_anchors_on_first_open_not_first_close() {
        // The first candle opens at 100 and closes at 120; the period change
        // measures from the open (the window's start price), so 100 -> 150
        // (+50%), not 120 -> 150 (+25%).
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
        let row = TickerRow::from_candles("BTC-USD", &candles).expect("BUG: candles build a row");
        assert!((row.change_pct - 50.0).abs() < 1e-9);
        assert!((row.price - 150.0).abs() < 1e-9);
    }

    #[test]
    fn series_leads_with_the_window_open() {
        // The sparkline must start at the first candle's open (the change
        // anchor), then follow each candle's close.
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
        let row = TickerRow::from_candles("BTC-USD", &candles).expect("BUG: candles build a row");
        assert_eq!(row.series, vec![90.0, 100.0, 110.0]);
    }

    #[test]
    fn zero_first_close_keeps_the_row_with_zero_change() {
        let row = TickerRow::from_candles("AAPL", &candles(&[0.0, 5.0], 3_600))
            .expect("BUG: candles build a row");
        assert!((row.change_pct - 0.0).abs() < 1e-9);
        assert!((row.price - 5.0).abs() < 1e-9);
    }

    #[test]
    fn no_candles_is_none() {
        assert!(TickerRow::from_candles("AAPL", &candles(&[], 3_600)).is_none());
    }

    #[test]
    fn negative_first_close_keeps_row_with_finite_change() {
        let row = TickerRow::from_candles("AAPL", &candles(&[-1.0, 5.0], 3_600))
            .expect("BUG: candles build a row");
        assert!(row.change_pct.is_finite());
        assert!((row.price - 5.0).abs() < 1e-9);
    }

    #[test]
    fn only_an_explicit_false_reads_as_closed_and_indices_are_exempt() {
        // Per the Nexus contract an omitted flag counts as open — a provider
        // that cannot report market state must never dim the row.
        let open = TickerRow::from_candles("AAPL", &candles(&[1.0, 2.0], 3_600))
            .expect("BUG: candles build a row");
        assert!(open.market_open);
        assert!(!open.is_closed_marked());

        let mut closed = TickerRow::from_candles("AAPL", &candles(&[1.0, 2.0], 3_600))
            .expect("BUG: candles build a row");
        closed.set_market_open(Some(false));
        assert!(closed.is_closed_marked());
        let mut idx = TickerRow::from_candles("^GSPC", &candles(&[1.0, 2.0], 3_600))
            .expect("BUG: candles build a row");
        idx.set_market_open(Some(false));
        assert!(!idx.is_closed_marked());
    }

    #[test]
    fn truncate_appends_ellipsis_only_when_overflowing() {
        assert_eq!(truncate_name("Apple Inc.", 20), "Apple Inc.");
        assert_eq!(
            truncate_name("Advanced Micro Devices, Inc.", 18),
            "Advanced Micro De\u{2026}"
        );
        assert_eq!(truncate_name("", 5), "");
    }
}

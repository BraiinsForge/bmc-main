// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host-pure per-row data model and the company-name truncation. No SDK draw
//! types, so it all unit-tests on the host.

use prices::candle::{self, Candles};
use prices::period::Candle;

/// One row's lifecycle. The company name is **not** held here — it lives in a
/// parallel names cache so a period change (which resets price state) cannot
/// drop already-fetched names.
pub enum RowState {
    /// No price reply yet.
    Loading,
    /// Price series parsed.
    Resolved(TickerRow),
    /// 404/400 — symbol not found / invalid; the row's poll is disabled.
    InputError,
    /// Any failed reply (including 503) with nothing ever loaded for this
    /// row; the poll keeps running, so the row recovers on a later reply.
    Failed,
}

pub struct TickerRow {
    pub symbol: String,
    pub price: f64,
    pub change_pct: f64,
    pub series: Vec<f64>,
    pub market_open: bool,
}

impl TickerRow {
    /// Build from parsed candles. Unlike the sparkline's `Series`, a
    /// zero/negative first close does **not** drop the row: the change is `0.0`
    /// and the price still shows (deckfeeder `first != 0 ? … : 0`). Returns
    /// `None` only when there are no usable candles. Prices are assumed
    /// non-negative; a negative first close (malformed data) yields a
    /// best-effort percentage rather than dropping the row.
    #[must_use]
    pub fn from_candles(
        symbol: &str,
        candles: &Candles,
        liveness: Candle,
        now_secs: i64,
    ) -> Option<TickerRow> {
        let first = candles.bars.first()?.close;
        let closes: Vec<f64> = candles.bars.iter().map(|b| b.close).collect();
        let current = *closes.last()?;
        let change_pct = if first == 0.0 {
            0.0
        } else {
            (current - first) / first * 100.0
        };
        let market_open = candle::market_open(&candles.bars, liveness, now_secs);
        Some(TickerRow {
            symbol: symbol.to_owned(),
            price: current,
            change_pct,
            series: closes,
            market_open,
        })
    }

    #[must_use]
    pub fn is_positive(&self) -> bool {
        self.change_pct >= 0.0
    }

    /// A market-closed row (and not a `^` index) is dimmed with a stop marker.
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
            instrument: "X".into(),
            candle_size: "1h".into(),
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
        }
    }

    #[test]
    fn row_change_is_first_to_last_and_sign_drives_positive() {
        let up = TickerRow::from_candles(
            "AAPL",
            &candles(&[100.0, 110.0], 3_600),
            Candle::H1,
            9_999_999_999,
        )
        .expect("row");
        assert!((up.change_pct - 10.0).abs() < 1e-9);
        assert!(up.is_positive());
        let down = TickerRow::from_candles(
            "AAPL",
            &candles(&[100.0, 90.0], 3_600),
            Candle::H1,
            9_999_999_999,
        )
        .expect("row");
        assert!(!down.is_positive());
    }

    #[test]
    fn zero_first_close_keeps_the_row_with_zero_change() {
        // deckfeeder: a zero first price yields change 0, not a dropped row.
        let row = TickerRow::from_candles("AAPL", &candles(&[0.0, 5.0], 3_600), Candle::H1, 0)
            .expect("row");
        assert!((row.change_pct - 0.0).abs() < 1e-9);
        assert!((row.price - 5.0).abs() < 1e-9);
    }

    #[test]
    fn no_candles_is_none() {
        assert!(TickerRow::from_candles("AAPL", &candles(&[], 3_600), Candle::H1, 0).is_none());
    }

    #[test]
    fn index_is_never_closed_marked() {
        let idx = TickerRow::from_candles(
            "^GSPC",
            &candles(&[1.0, 2.0], 3_600),
            Candle::H1,
            9_999_999_999,
        )
        .expect("row");
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

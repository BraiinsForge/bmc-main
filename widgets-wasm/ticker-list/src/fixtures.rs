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

//! Lists for the gallery and the tests: the manifest's default symbols
//! under the names and seven-day moves the capture fixture recorded,
//! in every state a row can take.

use bmc_wasm_sdk::SystemTime;
use prices::fetch::PriceMiss;
use prices::format::price_change;

use crate::model::{RowState, TickerRow};

/// The gallery clock starts at zero, so this reads as twelve minutes old.
const STALE_SINCE: SystemTime = SystemTime {
    unix_secs: -12 * 60,
};

/// Symbol, name, and a series from the recorded seven-day open to its close.
type Recorded = (&'static str, &'static str, [f64; 7]);

const RECORDED: [Recorded; 8] = [
    (
        "NVDA",
        "NVIDIA Corporation",
        [208.2, 211.4, 209.8, 215.3, 219.9, 218.1, 225.05],
    ),
    (
        "AAPL",
        "Apple Inc.",
        [313.64, 311.2, 314.0, 309.5, 307.8, 308.9, 305.69],
    ),
    (
        "TSLA",
        "Tesla, Inc.",
        [399.1, 385.4, 390.2, 371.8, 360.5, 352.3, 339.34],
    ),
    (
        "MSTR",
        "Strategy Inc.",
        [94.78, 96.1, 93.9, 95.7, 98.2, 96.9, 97.66],
    ),
    (
        "JPM",
        "JPMorgan Chase & Co.",
        [327.0, 331.5, 338.2, 336.9, 347.4, 355.8, 361.2],
    ),
    (
        "META",
        "Meta Platforms, Inc. Class A Common Stock",
        [652.0, 640.3, 628.7, 615.2, 598.4, 581.9, 569.01],
    ),
    (
        "SPY",
        "SPDR S&P 500 ETF Trust",
        [750.91, 754.2, 752.8, 760.4, 765.1, 768.9, 772.68],
    ),
    (
        "NFLX",
        "Netflix, Inc.",
        [72.635, 73.4, 72.9, 74.2, 75.1, 74.6, 76.03],
    ),
];

/// The S&P 500, which Nexus reports under market hours
/// but the widget never marks closed.
const INDEX: Recorded = (
    "^GSPC",
    "S&P 500",
    [
        7_735.18, 7_712.4, 7_698.9, 7_721.3, 7_740.2, 7_731.8, 7_745.06,
    ],
);

/// Everything `render::view` draws, one entry per configured symbol.
pub struct List {
    pub symbols: Vec<String>,
    pub states: Vec<RowState>,
    pub names: Vec<Option<String>>,
    pub stale: Vec<Option<SystemTime>>,
}

impl List {
    fn push(&mut self, symbol: &str, name: Option<&str>, state: RowState) {
        self.symbols.push(symbol.to_owned());
        self.names.push(name.map(str::to_owned));
        self.states.push(state);
        self.stale.push(None);
    }

    fn push_row(&mut self, (symbol, name, series): Recorded, market_open: bool) {
        self.push(symbol, Some(name), resolved(symbol, series, market_open));
    }
}

fn empty() -> List {
    List {
        symbols: Vec::new(),
        states: Vec::new(),
        names: Vec::new(),
        stale: Vec::new(),
    }
}

fn resolved(symbol: &str, series: [f64; 7], market_open: bool) -> RowState {
    let first = series[0];
    let price = series[series.len() - 1];
    RowState::Resolved {
        data: TickerRow {
            symbol: symbol.to_owned(),
            price,
            change_pct: price_change(first, price),
            series: series.to_vec(),
            market_open,
        },
    }
}

/// Every default symbol loaded, every market open.
#[must_use]
pub fn healthy() -> List {
    let mut list = empty();
    for row in RECORDED {
        list.push_row(row, true);
    }
    list
}

/// Every state a row can take. The first four, all Large and BMM101 seat,
/// put a shut market's empty window and a symbol Nexus lacks between live rows.
#[must_use]
pub fn mixed() -> List {
    let [nvda, aapl, tsla, mstr, jpm, meta, spy, _] = RECORDED;
    let mut list = empty();
    list.push_row(nvda, true);
    list.push(
        tsla.0,
        Some(tsla.1),
        RowState::NoData {
            market_closed: true,
        },
    );
    list.push(
        "NONEXS",
        None,
        RowState::InputError {
            miss: PriceMiss::NotFound,
        },
    );
    list.push_row(aapl, true);
    list.push_row(mstr, false);
    list.push(jpm.0, Some(jpm.1), RowState::Failed);
    list.push(meta.0, None, RowState::Loading);
    list.push(
        spy.0,
        Some(spy.1),
        RowState::NoData {
            market_closed: false,
        },
    );
    list
}

/// No price reply yet on any row.
#[must_use]
pub fn loading() -> List {
    let mut list = empty();
    for (symbol, _, _) in RECORDED {
        list.push(symbol, None, RowState::Loading);
    }
    list
}

/// Every first fetch failed.
#[must_use]
pub fn failed() -> List {
    let mut list = empty();
    for (symbol, name, _) in RECORDED {
        list.push(symbol, Some(name), RowState::Failed);
    }
    list
}

/// Every row holding a series its refreshes have since failed to replace.
#[must_use]
pub fn stale() -> List {
    let mut list = healthy();
    list.stale.fill(Some(STALE_SINCE));
    list
}

/// Markets shut with the window still holding bars,
/// the index among them to show it goes unmarked.
#[must_use]
pub fn closed_markets() -> List {
    let mut list = empty();
    list.push_row(INDEX, false);
    for row in RECORDED.into_iter().take(7) {
        list.push_row(row, false);
    }
    list
}

/// Every symbol slot left empty.
#[must_use]
pub fn no_symbols() -> List {
    empty()
}

/// Fewer symbols than any size seats.
#[must_use]
pub fn one_symbol() -> List {
    let mut list = empty();
    list.push_row(RECORDED[0], true);
    list
}

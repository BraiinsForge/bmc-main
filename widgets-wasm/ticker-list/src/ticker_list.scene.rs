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

use bmc_gallery::prelude::*;
use prices::format::price_change;
use ticker_list::model::{RowState, TickerRow};
use ticker_list::render;

scene_meta! { title: "Widgets / Tickers / Ticker List" }

#[derive(Clone, Copy)]
enum Viewport {
    Variant(SizeVariant),
    Dimensions(u32, u32),
}

const VIEWPORTS: [(Viewport, &str); 5] = [
    (Viewport::Variant(SizeVariant::Full), "Fullscreen"),
    (Viewport::Variant(SizeVariant::Large), "Large"),
    (Viewport::Variant(SizeVariant::Medium), "Medium"),
    (Viewport::Variant(SizeVariant::Small), "Small"),
    (Viewport::Dimensions(480, 320), "BMM101"),
];

const ROWS: [(&str, &str, [f64; 7], bool); 8] = [
    (
        "AAPL",
        "Apple Inc.",
        [296.0, 299.0, 298.0, 302.0, 304.0, 303.0, 306.0],
        true,
    ),
    (
        "TSLA",
        "Tesla, Inc.",
        [348.0, 344.0, 345.0, 340.0, 337.0, 338.0, 334.0],
        false,
    ),
    (
        "MSFT",
        "Microsoft Corp.",
        [510.0, 512.0, 511.0, 515.0, 518.0, 517.0, 521.0],
        true,
    ),
    (
        "META",
        "Meta Platforms, Inc.",
        [575.0, 572.0, 568.0, 570.0, 565.0, 561.0, 559.0],
        true,
    ),
    (
        "JPM",
        "JPMorgan Chase & Co.",
        [356.0, 357.0, 360.0, 359.0, 362.0, 364.0, 366.0],
        false,
    ),
    (
        "NVDA",
        "NVIDIA Corp.",
        [186.0, 184.0, 185.0, 181.0, 179.0, 176.0, 174.0],
        true,
    ),
    (
        "SPY",
        "SPDR S&P 500 ETF Trust",
        [766.0, 768.0, 767.0, 771.0, 774.0, 773.0, 777.0],
        true,
    ),
    (
        "NFLX",
        "Netflix, Inc.",
        [82.0, 81.0, 79.0, 80.0, 77.0, 75.0, 74.0],
        false,
    ),
];

fn fixture() -> (Vec<String>, Vec<RowState>, Vec<Option<String>>) {
    let mut symbols = Vec::with_capacity(ROWS.len());
    let mut states = Vec::with_capacity(ROWS.len());
    let mut names = Vec::with_capacity(ROWS.len());
    for (symbol, name, series, market_open) in ROWS {
        let first = series[0];
        let price = series[series.len() - 1];
        symbols.push(symbol.to_owned());
        names.push(Some(name.to_owned()));
        states.push(RowState::Resolved {
            data: TickerRow {
                symbol: symbol.to_owned(),
                price,
                change_pct: price_change(first, price),
                series: series.to_vec(),
                market_open,
            },
        });
    }
    (symbols, states, names)
}

fn render_viewport(ctx: &mut SceneCtx, ui: &mut Ui, (viewport, label): (Viewport, &str)) {
    let size = match viewport {
        Viewport::Variant(variant) => WidgetSize {
            variant,
            width: variant.width(),
            height: variant.height(),
        },
        Viewport::Dimensions(width, height) => WidgetSize::from_dimensions(width, height),
    };
    let (width, height) = (size.width, size.height);
    ui.heading(label);
    ctx.node_stage(ui, (width, height), || {
        let (symbols, states, names) = fixture();
        let stale: [Option<SystemTime>; ROWS.len()] = [None; ROWS.len()];
        render::view(&symbols, &states, &names, &stale, size)
    });
}

#[scene(default)]
fn ticker_list(ctx: &mut SceneCtx, ui: &mut Ui) {
    render_viewport(ctx, ui, VIEWPORTS[0]);
    ui.columns(2, |columns| {
        for (index, viewport) in VIEWPORTS[1..].iter().copied().enumerate() {
            render_viewport(ctx, &mut columns[index % 2], viewport);
        }
    });
}

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

//! Ticker Single's sparkline and candlestick views at every design size.

use bmc_gallery::prelude::*;
use prices::candle::{CandleBar, Candles};
use prices::period::Period;
use ticker_single::model::Series;
use ticker_single::render::{candlestick, sparkline};

scene_meta! { title: "Widgets / Ticker Single" }

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

fn sample_series(market_open: bool, falling: bool) -> Series {
    let bars = if falling {
        [
            (112.0, 113.0, 108.0, 109.0),
            (109.0, 111.0, 105.0, 106.0),
            (106.0, 108.0, 103.0, 105.0),
            (105.0, 106.0, 101.0, 102.0),
            (102.0, 104.0, 98.0, 100.0),
            (100.0, 102.0, 97.0, 99.0),
        ]
    } else {
        [
            (98.0, 103.0, 97.0, 101.0),
            (101.0, 106.0, 100.0, 105.0),
            (105.0, 107.0, 102.0, 103.0),
            (103.0, 109.0, 102.0, 108.0),
            (108.0, 112.0, 106.0, 110.0),
            (110.0, 113.0, 107.0, 109.0),
        ]
    }
    .into_iter()
    .zip(0_i32..)
    .map(|((open, high, low, close), hour)| CandleBar {
        t_secs: 1_700_000_000 + i64::from(hour) * 3_600,
        open,
        high,
        low,
        close,
        volume: Some(1_000.0 + f64::from(hour) * 100.0),
    })
    .collect();
    let mut series = Series::from_candles(Candles {
        bars,
        quote_currency: Some("USD".to_owned()),
    })
    .expect("BUG: the gallery fixture contains candles");
    series.set_market_open(Some(market_open));
    series
}

fn render_viewport(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    market_open: bool,
    candlestick: bool,
    falling: bool,
    (viewport, label): (Viewport, &str),
) {
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
        let series = sample_series(market_open, falling);
        if candlestick {
            candlestick::series_view(&series, "AAPL", Period::D7, "7d", size)
        } else {
            sparkline::series_view(&series, "AAPL", "1D", size)
        }
    });
}

#[scene(default)]
fn ticker_single(ctx: &mut SceneCtx, ui: &mut Ui) {
    let market_open = ctx.toggle("Market open", true);
    let candlestick = ctx.radio("View", &["Sparkline", "Candlestick"], 0) == 1;
    let falling = ctx.radio("Trend", &["Rising", "Falling"], 0) == 1;
    render_viewport(ctx, ui, market_open, candlestick, falling, VIEWPORTS[0]);
    ui.columns(2, |columns| {
        for (index, viewport) in VIEWPORTS[1..].iter().copied().enumerate() {
            render_viewport(
                ctx,
                &mut columns[index % 2],
                market_open,
                candlestick,
                falling,
                viewport,
            );
        }
    });
}

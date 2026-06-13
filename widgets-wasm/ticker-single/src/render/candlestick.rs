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

//! The candlestick view: header row on top, a canvas below drawing the
//! candle plot with a dashed price grid, a right-edge price axis with a
//! current-price badge, a volume strip, and time labels on the x axis.

#[expect(
    clippy::wildcard_imports,
    reason = "the shared render module's SDK imports and palette"
)]
use super::*;

use bmc_wasm_sdk::calendar::tz_convert;

use crate::chart_layout::{
    candle_shapes, clamp_center_y, dash_spans, label_indices, label_kind, label_min_px, label_text,
    max_candles, merge_bars, month_leads, nice_ticks, price_range, price_to_y, volume_heights,
};
use prices::period::Period;

const DASH_ON: f32 = 4.0;
const DASH_OFF: f32 = 4.0;
const WICK_W: f32 = 1.0;
const GRID_ALPHA: f32 = 0.3;
const VOLUME_ALPHA: f32 = 0.3;
const PLOT_LEFT_INSET: f32 = 4.0;
const Y_TICK_TARGET: usize = 4;
const X_LABEL_MIN_PX: f32 = 70.0;

/// The loaded view: header on top, candle chart on a canvas below.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "viewport dimensions, font sizes, and bar counts are small integers, exact in f32"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one linear paint pass: grid, candles, price line, volume, labels"
)]
pub fn series_view(
    series: &Series,
    symbol: &str,
    period: Period,
    period_label: &str,
    ws: WidgetSize,
) -> Node {
    let band = band_for(ws.variant).scaled(ws.fit());
    let (w, h) = (ws.width as f32, ws.height as f32);
    let trend = if series.is_positive() {
        TREND_UP
    } else {
        TREND_DOWN
    };
    let alpha = if series.is_closed_marked(symbol) {
        CLOSED_ALPHA
    } else {
        1.0
    };

    let header_h = header_height(&band);
    let canvas_h = (h - header_h).max(0.0);
    let plot_x = PLOT_LEFT_INSET;
    let plot_w = (w - band.axis_width - band.axis_gap - plot_x).max(1.0);

    let bars = merge_bars(&series.bars, max_candles(plot_w));
    let volumes = volume_heights(&bars, band.volume_height);
    let has_volume = volumes.iter().any(Option::is_some);
    let labels_h = if band.show_x_labels {
        band.x_label_height
    } else {
        0.0
    };
    let volume_h = if has_volume { band.volume_height } else { 0.0 };
    let plot_h = (canvas_h - volume_h - labels_h).max(1.0);

    let mut draws = Vec::new();
    if let Some((min, max)) = price_range(&bars, series.current) {
        // The one derivation of per-bar slot geometry: candles, volume
        // bars, and x-axis labels all read their centers/widths from it,
        // so they cannot drift out of horizontal alignment.
        let shapes = candle_shapes(&bars, plot_x, plot_w, 0.0, plot_h, min, max);
        let grid = SYMBOL_COLOR.with_alpha(GRID_ALPHA * alpha);
        for tick in nice_ticks(min, max, Y_TICK_TARGET) {
            let y = price_to_y(tick, min, max, 0.0, plot_h);
            for (x0, x1) in dash_spans(plot_x, plot_x + plot_w, DASH_ON, DASH_OFF) {
                draws.push(Draw::rect(x0, y, x1 - x0, 1.0, grid));
            }
            let label_y = clamp_center_y(y, plot_h, band.axis_font as f32);
            draws.push(Draw::text(
                w - band.axis_width,
                label_y,
                price_text(symbol, tick),
                style!(
                    size: band.axis_font,
                    color: SYMBOL_COLOR.with_alpha(alpha),
                    valign: VerticalAlign::Center,
                ),
            ));
        }

        for shape in &shapes {
            let color = if shape.up { TREND_UP } else { TREND_DOWN }.with_alpha(alpha);
            draws.push(Draw::rect(
                shape.x_center - WICK_W / 2.0,
                shape.wick_top,
                WICK_W,
                shape.wick_h,
                color,
            ));
            draws.push(Draw::rect(
                shape.x_center - shape.body_w / 2.0,
                shape.body_top,
                shape.body_w,
                shape.body_h,
                color,
            ));
        }

        let price_line_y = price_to_y(series.current, min, max, 0.0, plot_h);
        for (x0, x1) in dash_spans(plot_x, plot_x + plot_w, DASH_ON, DASH_OFF) {
            draws.push(Draw::rect(
                x0,
                price_line_y,
                x1 - x0,
                1.0,
                trend.with_alpha(alpha),
            ));
        }
        let badge_h = band.axis_font as f32 + 8.0;
        let price_y = clamp_center_y(price_line_y, plot_h, badge_h);
        draws.push(Draw::rect(
            w - band.axis_width - band.axis_gap / 2.0,
            price_y - badge_h / 2.0,
            band.axis_width + band.axis_gap / 2.0,
            badge_h,
            trend.with_alpha(alpha),
        ));
        draws.push(Draw::text(
            w - band.axis_width,
            price_y,
            price_text(symbol, series.current),
            style!(
                size: band.axis_font,
                weight: FontWeight::BOLD,
                color: BACKGROUND,
                valign: VerticalAlign::Center,
            ),
        ));
        if has_volume {
            let volume_top = plot_h;
            for (shape, height) in shapes.iter().zip(&volumes) {
                let Some(height) = height else { continue };
                let color = if shape.up { TREND_UP } else { TREND_DOWN };
                draws.push(Draw::rect(
                    shape.x_center - shape.body_w / 2.0,
                    volume_top + (band.volume_height - height),
                    shape.body_w,
                    *height,
                    color.with_alpha(VOLUME_ALPHA * alpha),
                ));
            }
        }

        if band.show_x_labels {
            let tz_name = system::current().timezone().unwrap_or("UTC").to_owned();
            let time_format = system::current().time_format().unwrap_or_default();
            let month_first = month_leads(system::current().date_format());
            let kind = label_kind(period);
            let labels_y = plot_h + volume_h + labels_h / 2.0;
            let texts: Vec<String> = bars
                .iter()
                .map(|b| {
                    tz_convert(b.t_secs, &tz_name)
                        .map(|ldt| label_text(kind, &ldt, time_format, month_first))
                        .unwrap_or_default()
                })
                .collect();
            let max_chars = texts.iter().map(|t| t.chars().count()).max().unwrap_or(0);
            let min_px = label_min_px(max_chars, band.axis_font, X_LABEL_MIN_PX);
            for i in label_indices(&texts, plot_w, min_px) {
                draws.push(Draw::text(
                    shapes[i].x_center,
                    labels_y,
                    texts[i].clone(),
                    style!(
                        size: band.axis_font,
                        color: SYMBOL_COLOR.with_alpha(alpha),
                        align: TextAlign::Center,
                        valign: VerticalAlign::Center,
                    ),
                ));
            }
        }
    }

    let header = super::header_row(series, symbol, period_label, &band, trend, alpha, header_h);
    col(
        props!(background: BACKGROUND, width: w, height: h),
        [header, canvas(props!(width: w, height: canvas_h), draws)],
    )
}

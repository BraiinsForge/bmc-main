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

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "the shared render module's SDK imports and palette"
    )
)]
use super::*;

#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::calendar::tz_convert as calendar_label_time;

use crate::chart_layout::{
    candle_shapes, clamp_center_y, label_indices, label_kind, label_min_px, label_text,
    max_candles, merge_bars, month_leads, nice_ticks, price_range, price_to_y, volume_heights,
};
use prices::period::Period;

// Timezone conversion needs wasm32 host FFI, so native renders omit calendar labels.
#[cfg(not(target_arch = "wasm32"))]
fn calendar_label_time(_unix_secs: i64, _timezone: &str) -> Option<LocalDateTime> {
    None
}

const DASH_ON: f32 = 4.0;
const DASH_OFF: f32 = 4.0;
const DASH_STROKE_W: f32 = 1.0;
const WICK_W: f32 = 1.0;
const GRID_ALPHA: f32 = 0.3;
const VOLUME_ALPHA: f32 = 0.3;
const PLOT_LEFT_INSET: f32 = 4.0;
const Y_TICK_TARGET: usize = 4;
const X_LABEL_MIN_PX: f32 = 70.0;

// bmc-render's round caps overhang each run and would narrow every gap.
fn dashed_line(x: f32, y: f32, width: f32, color: Color) -> Draw {
    let half_stroke = DASH_STROKE_W / 2.0;
    path!(
        vec![(x + half_stroke, y + half_stroke), (x + width - half_stroke, y + half_stroke)],
        stroke: DASH_STROKE_W,
        color: color,
        dashed: (DASH_ON - DASH_STROKE_W, DASH_OFF + DASH_STROKE_W)
    )
}

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
    let closed = series.is_closed_marked(symbol);
    let alpha = if closed { CLOSED_CHART_ALPHA } else { 1.0 };

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
    let axis_box_h = band.axis_font as f32 * PRICE_BOX_LINES;

    let mut draws = Vec::new();
    if let Some((min, max)) = price_range(&bars, series.current) {
        // The one derivation of per-bar slot geometry: candles, volume
        // bars, and x-axis labels all read their centers/widths from it,
        // so they cannot drift out of horizontal alignment.
        let shapes = candle_shapes(&bars, plot_x, plot_w, 0.0, plot_h, min, max);
        let grid = SYMBOL_COLOR.with_alpha(GRID_ALPHA * alpha);
        for tick in nice_ticks(min, max, Y_TICK_TARGET) {
            let y = price_to_y(tick, min, max, 0.0, plot_h);
            draws.push(dashed_line(plot_x, y, plot_w, grid));
            let label_y = clamp_center_y(y, plot_h, band.axis_font as f32);
            draws.push(Draw::autofit_text(
                w - band.axis_width,
                label_y - axis_box_h / 2.0,
                band.axis_width,
                axis_box_h,
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
        draws.push(dashed_line(
            plot_x,
            price_line_y,
            plot_w,
            trend.with_alpha(alpha),
        ));
        let badge_h = band.axis_font as f32 + 8.0;
        let price_y = clamp_center_y(price_line_y, plot_h, badge_h);
        draws.push(Draw::rect(
            w - band.axis_width - band.axis_gap / 2.0,
            price_y - badge_h / 2.0,
            band.axis_width + band.axis_gap / 2.0,
            badge_h,
            trend.with_alpha(alpha),
        ));
        draws.push(Draw::autofit_text(
            w - band.axis_width,
            price_y - axis_box_h / 2.0,
            band.axis_width,
            axis_box_h,
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
                    calendar_label_time(b.t_secs, &tz_name)
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

    let header = super::header_row(series, symbol, period_label, &band, trend, closed, header_h);
    col(
        props!(background: BACKGROUND, width: w, height: h),
        [header, canvas(props!(width: w, height: canvas_h), draws)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prices::candle::CandleBar;

    fn series() -> Series {
        let bars = vec![
            CandleBar {
                t_secs: 0,
                open: 90.0,
                high: 105.0,
                low: 85.0,
                close: 100.0,
                volume: None,
            },
            CandleBar {
                t_secs: 3_600,
                open: 100.0,
                high: 125.0,
                low: 95.0,
                close: 120.0,
                volume: None,
            },
        ];
        Series {
            bars,
            current: 120.0,
            change_pct: 100.0 / 3.0,
            quote_currency: Some("USD".to_owned()),
            market_open: true,
        }
    }

    fn canvas_draws(node: Node) -> Vec<Draw> {
        let Node::Column(_, children) = node else {
            panic!("BUG: candlestick root must be a column");
        };
        let Some(Node::Canvas { draws, .. }) = children.into_iter().nth(1) else {
            panic!("BUG: candlestick chart must follow the header");
        };
        draws
    }

    fn dashed_paths(draws: &[Draw]) -> Vec<(&[(f32, f32)], Color)> {
        draws
            .iter()
            .filter_map(|draw| {
                let Draw::Path {
                    points,
                    paint:
                        PathPaint::Stroke {
                            color,
                            width,
                            dash: Some(dash),
                        },
                    closed,
                    interpolation,
                } = draw
                else {
                    return None;
                };
                assert_eq!(
                    dash.on + *width,
                    DASH_ON,
                    "round caps must restore the intended painted run"
                );
                assert_eq!(
                    dash.off - *width,
                    DASH_OFF,
                    "round caps must restore the intended painted gap"
                );
                assert!(!closed, "grid strokes must remain open paths");
                assert_eq!(*interpolation, Interpolation::Linear);
                Some((points.as_slice(), *color))
            })
            .collect()
    }

    #[test]
    fn horizontal_lines_are_one_bounded_draw_each() {
        let series = series();
        let narrow = canvas_draws(series_view(
            &series,
            "AAPL",
            Period::D7,
            "7d",
            WidgetSize::from_dimensions(480, 320),
        ));
        let wide = canvas_draws(series_view(
            &series,
            "AAPL",
            Period::D7,
            "7d",
            WidgetSize::from_dimensions(1_280, 720),
        ));

        let narrow_paths = dashed_paths(&narrow);
        let wide_paths = dashed_paths(&wide);
        let (min, max) = price_range(&series.bars, series.current)
            .expect("BUG: finite bars must have a price range");
        let expected_line_count = nice_ticks(min, max, Y_TICK_TARGET).len() + 1;

        assert_eq!(narrow_paths.len(), expected_line_count);
        assert_eq!(wide_paths.len(), expected_line_count);
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "test viewport dimensions are small integers, exact in f32"
    )]
    fn horizontal_paths_preserve_plot_geometry_and_style() {
        let series = series();
        let ws = WidgetSize::from_dimensions(1_280, 720);
        let draws = canvas_draws(series_view(&series, "AAPL", Period::D7, "7d", ws));
        let paths = dashed_paths(&draws);
        let band = band_for(ws.variant).scaled(ws.fit());
        let width = ws.width as f32;
        let half_stroke = DASH_STROKE_W / 2.0;
        let axis_gutter = width - band.axis_width - band.axis_gap;

        for (points, _) in &paths {
            assert_eq!(points.len(), 2);
            assert_eq!(points[0].1, points[1].1);
            assert_eq!(points[0].0, PLOT_LEFT_INSET + half_stroke);
            assert_eq!(
                points[1].0 + half_stroke,
                axis_gutter,
                "grid strokes must cover the plot without entering the price-axis gutter"
            );
        }
        assert_eq!(paths.last().map(|(_, color)| *color), Some(TREND_UP));
    }

    fn autofit_boxes(draws: &[Draw]) -> Vec<(f32, f32, u32)> {
        draws
            .iter()
            .filter_map(|draw| {
                let Draw::AutofitText {
                    box_width,
                    box_height,
                    style,
                    ..
                } = draw
                else {
                    return None;
                };
                Some((*box_width, *box_height, style.size))
            })
            .collect()
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "font sizes are small integers, exact in f32"
    )]
    fn axis_price_boxes_are_shorter_than_two_authored_lines() {
        let series = series();
        let ws = WidgetSize::from_dimensions(1_280, 720);
        let draws = canvas_draws(series_view(&series, "AAPL", Period::D7, "7d", ws));
        let band = band_for(ws.variant).scaled(ws.fit());
        let boxes = autofit_boxes(&draws);
        let (min, max) = price_range(&series.bars, series.current)
            .expect("BUG: finite bars must have a price range");

        assert_eq!(
            boxes.len(),
            nice_ticks(min, max, Y_TICK_TARGET).len() + 1,
            "every tick label and the current-price badge must autofit"
        );
        let one_line = band.axis_font as f32 * 1.4;
        for (box_width, box_height, size) in boxes {
            assert_eq!(size, band.axis_font);
            assert!(
                (box_width - band.axis_width).abs() < f32::EPSILON,
                "an axis price shrinks to the gutter instead of spilling into the plot"
            );
            assert!(
                box_height >= one_line && box_height < one_line * 2.0,
                "a second authored-size line must not fit in the autofit box"
            );
        }
    }
}

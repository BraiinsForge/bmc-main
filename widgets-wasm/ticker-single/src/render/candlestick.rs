// Copyright (C) 2026  Braiins Systems s.r.o.

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
    candle_shapes, dash_spans, label_indices, label_kind, label_text, max_candles, merge_bars,
    nice_ticks, price_range, price_to_y, volume_heights,
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
    let is_index = symbol.starts_with('^');
    let alpha = if !series.market_open && !is_index {
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
        let grid = SYMBOL_COLOR.with_alpha(GRID_ALPHA * alpha);
        for tick in nice_ticks(min, max, Y_TICK_TARGET) {
            let y = price_to_y(tick, min, max, 0.0, plot_h);
            for (x0, x1) in dash_spans(plot_x, plot_x + plot_w, DASH_ON, DASH_OFF) {
                draws.push(Draw::rect(x0, y, x1 - x0, 1.0, grid));
            }
            draws.push(Draw::text(
                w - band.axis_width,
                y,
                price_text(symbol, tick),
                style!(
                    size: band.axis_font,
                    color: SYMBOL_COLOR.with_alpha(alpha),
                    valign: VerticalAlign::Center,
                ),
            ));
        }

        for shape in candle_shapes(&bars, plot_x, plot_w, 0.0, plot_h, min, max) {
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

        let price_y = price_to_y(series.current, min, max, 0.0, plot_h);
        for (x0, x1) in dash_spans(plot_x, plot_x + plot_w, DASH_ON, DASH_OFF) {
            draws.push(Draw::rect(
                x0,
                price_y,
                x1 - x0,
                1.0,
                trend.with_alpha(alpha),
            ));
        }
        let badge_h = band.axis_font as f32 + 8.0;
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
    }

    if has_volume {
        let volume_top = plot_h;
        let slot = plot_w / bars.len().max(1) as f32;
        let bar_w = (slot * 0.7).max(1.0);
        for (i, height) in volumes.iter().enumerate() {
            let Some(height) = height else { continue };
            let up = bars[i].close >= bars[i].open;
            let color = if up { TREND_UP } else { TREND_DOWN };
            draws.push(Draw::rect(
                plot_x + slot * (i as f32 + 0.5) - bar_w / 2.0,
                volume_top + (band.volume_height - height),
                bar_w,
                *height,
                color.with_alpha(VOLUME_ALPHA * alpha),
            ));
        }
    }

    if band.show_x_labels {
        let tz_name = system::current().timezone().unwrap_or("UTC").to_owned();
        let time_format = system::current().time_format().unwrap_or_default();
        let kind = label_kind(period);
        let labels_y = plot_h + volume_h + labels_h / 2.0;
        let slot = plot_w / bars.len().max(1) as f32;
        let mut previous = String::new();
        for i in label_indices(bars.len(), plot_w, X_LABEL_MIN_PX) {
            let Some(ldt) = tz_convert(bars[i].t_secs, &tz_name) else {
                continue;
            };
            let text_str = label_text(kind, &ldt, time_format);
            if text_str == previous {
                continue;
            }
            previous.clone_from(&text_str);
            draws.push(Draw::text(
                plot_x + slot * (i as f32 + 0.5),
                labels_y,
                text_str,
                style!(
                    size: band.axis_font,
                    color: SYMBOL_COLOR.with_alpha(alpha),
                    align: TextAlign::Center,
                    valign: VerticalAlign::Center,
                ),
            ));
        }
    }

    let header = super::header_row(series, symbol, period_label, &band, trend, alpha, header_h);
    col(
        props!(background: BACKGROUND, width: w, height: h),
        [header, canvas(props!(width: w, height: canvas_h), draws)],
    )
}

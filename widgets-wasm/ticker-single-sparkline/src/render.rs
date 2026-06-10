// Copyright (C) 2026  Braiins Systems s.r.o.

//! The wasm render path: the header row, and a canvas that draws the
//! bottom-anchored sparkline with the tile-centered price drawn over it.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports and macros"
)]
use bmc_wasm_sdk::*;

use crate::display::{SizeBand, band_for};
use crate::model::{IconStyle, Series, change_text, fraction_digits, icon_for};
use prices::chart;

const BACKGROUND: Color = BLACK;
const SYMBOL_COLOR: Color = Color::from_rgb(0xc6, 0xc6, 0xc6);
const PRICE_COLOR: Color = Color::from_rgb(0xf4, 0xf4, 0xf4);
const TREND_UP: Color = Color::from_rgb(0x42, 0xbe, 0x65);
const TREND_DOWN: Color = Color::from_rgb(0xfa, 0x4d, 0x56);
const BADGE_BG_ALPHA: f32 = 0.15;
const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;
const CLOSED_ALPHA: f32 = 0.4;

#[expect(
    clippy::cast_precision_loss,
    reason = "viewport and font sizes are small integers, exact in f32"
)]
fn header_height(band: &SizeBand) -> f32 {
    let header_line = band.header_font as f32 * 1.3;
    band.top_padding * 2.0 + band.icon_diameter.max(header_line)
}

fn fixed_width(width: f32) -> Node {
    col(props!(width: width), Vec::<Node>::new())
}

fn icon_node(icon: &IconStyle, diameter: f32, glyph_font: u32, alpha: f32) -> Node {
    let disc = Color::from_rgb(icon.rgb.0, icon.rgb.1, icon.rgb.2).with_alpha(alpha);
    let glyph = WHITE.with_alpha(alpha);
    canvas(
        props!(width: diameter, height: diameter),
        [
            Draw::circle(diameter / 2.0, diameter / 2.0, diameter / 2.0, disc),
            Draw::text(
                diameter / 2.0,
                diameter / 2.0,
                icon.glyph.clone(),
                style!(
                    size: glyph_font,
                    weight: FontWeight::BOLD,
                    color: glyph,
                    align: TextAlign::Center,
                    valign: VerticalAlign::Center,
                ),
            ),
        ],
    )
}

fn badge_node(text_str: String, trend: Color, band: &SizeBand) -> Node {
    row(
        props!(background: trend.with_alpha(BADGE_BG_ALPHA), padding: band.badge_padding),
        [text(
            text_str,
            style!(size: band.header_font, weight: FontWeight::BOLD, color: trend),
        )],
    )
}

/// The loaded view: header on top, sparkline + price on a canvas below.
#[must_use]
pub fn series_view(series: &Series, symbol: &str, period_label: &str, ws: WidgetSize) -> Node {
    let band = band_for(ws.variant).scaled(ws.fit());
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
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

    // ── header ──────────────────────────────────────────────────────────
    let mut header_children = vec![fixed_width(band.edge_padding)];
    if let Some(icon) = icon_for(symbol) {
        header_children.push(icon_node(&icon, band.icon_diameter, band.glyph_font, alpha));
        header_children.push(fixed_width(band.header_left_gap));
    }
    header_children.push(text(
        symbol.to_uppercase(),
        style!(size: band.header_font, weight: FontWeight::BOLD, color: SYMBOL_COLOR),
    ));
    header_children.push(spacer(1.0));
    if band.show_period {
        header_children.push(text(
            period_label,
            style!(size: band.header_font, color: SYMBOL_COLOR),
        ));
        header_children.push(fixed_width(band.header_right_gap));
    }
    header_children.push(badge_node(change_text(series.change_pct), trend, &band));
    header_children.push(fixed_width(band.edge_padding));
    let header = row(
        props!(height: header_h, cross_align: CrossAlign::Center),
        header_children,
    );

    // ── chart + price canvas ────────────────────────────────────────────
    let mut draws = Vec::new();
    let raw_line = chart::series_points(&series.closes, w, band.chart_height, CHART_INSET);
    if raw_line.len() >= 2 {
        let y0 = canvas_h - band.chart_height;
        let line: Vec<(f32, f32)> = raw_line.iter().map(|&(x, y)| (x, y + y0)).collect();
        let mut area = line.clone();
        area.push((w, canvas_h));
        area.push((0.0, canvas_h));
        draws.push(fill!(area, linear: (trend.with_alpha(alpha), trend.with_alpha(0.0))));
        draws.push(path!(line, stroke: CHART_STROKE, color: trend.with_alpha(alpha)));
    }
    let price = format_number!(series.current, fraction_digits(series.current));
    draws.push(Draw::text(
        w / 2.0,
        h / 2.0 - header_h,
        price,
        style!(
            size: band.price_font,
            weight: FontWeight::BOLD,
            color: PRICE_COLOR,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));

    col(
        props!(background: BACKGROUND, width: w, height: h),
        [header, canvas(props!(width: w, height: canvas_h), draws)],
    )
}

/// A centered status message (loading / error / warming / input error).
#[must_use]
pub fn message_view(message: &str, ws: WidgetSize) -> Node {
    let band = band_for(ws.variant).scaled(ws.fit());
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
    let (w, h) = (ws.width as f32, ws.height as f32);
    col(
        props!(background: BACKGROUND, width: w, height: h, cross_align: CrossAlign::Center),
        [
            spacer(1.0),
            text(
                message,
                style!(size: band.header_font, color: SYMBOL_COLOR, align: TextAlign::Center),
            ),
            spacer(1.0),
        ],
    )
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! The wasm render path shared by both chart views: the palette, the
//! header row, the price formatting rule, and the status-message view.

pub mod candlestick;
pub mod sparkline;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports and macros"
)]
use bmc_wasm_sdk::*;

use crate::display::{SizeBand, band_for};
use crate::model::{
    IconStyle, MIN_PRICE, PricePrecision, Series, change_text, icon_for, price_precision,
};

const BACKGROUND: Color = BLACK;
const SYMBOL_COLOR: Color = Color::from_rgb(0xc6, 0xc6, 0xc6);
const PRICE_COLOR: Color = Color::from_rgb(0xf4, 0xf4, 0xf4);
const TREND_UP: Color = Color::from_rgb(0x42, 0xbe, 0x65);
const TREND_DOWN: Color = Color::from_rgb(0xfa, 0x4d, 0x56);
const BADGE_BG_ALPHA: f32 = 0.15;
/// The fill under the sparkline is a faint wash of the trend color, fading
/// from 15% at the line's peak to a 2% tint at the bottom edge — matching the
/// reference design previews.
const CHART_FILL_TOP_ALPHA: f32 = 0.15;
const CHART_FILL_BOTTOM_ALPHA: f32 = 0.02;
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

/// The shared header row: icon + symbol, then period label and change
/// badge right-aligned.
fn header_row(
    series: &Series,
    symbol: &str,
    period_label: &str,
    band: &SizeBand,
    trend: Color,
    alpha: f32,
    header_h: f32,
) -> Node {
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
    header_children.push(badge_node(change_text(series.change_pct), trend, band));
    header_children.push(fixed_width(band.edge_padding));
    row(
        props!(height: header_h, cross_align: CrossAlign::Center),
        header_children,
    )
}

/// The formatted price per the shared precision rule.
fn price_text(symbol: &str, value: f64) -> String {
    match price_precision(symbol, value) {
        PricePrecision::Fraction(digits) => format_number!(value, digits),
        PricePrecision::BelowMin => {
            let mut out = String::from("<");
            out.push_str(&format_number!(MIN_PRICE, 6));
            out
        }
    }
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

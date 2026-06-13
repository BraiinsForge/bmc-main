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

//! The wasm render path: the per-size ticker grid (two columns at Full, a
//! single column otherwise), each row's symbol/company, optional
//! sparkline, and price/change, with hairline dividers between cells.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports and macros"
)]
use bmc_wasm_sdk::*;

use crate::layout::{Band, band_for};
use crate::model::{RowState, TickerRow, truncate_name};
use prices::chart;
use prices::format::{MIN_PRICE, PricePrecision, change_text, price_precision};

const BACKGROUND: Color = BLACK;
const PRIMARY: Color = Color::from_rgb(0xf4, 0xf4, 0xf4);
const SECONDARY: Color = Color::from_rgb(0xc6, 0xc6, 0xc6);
const TREND_UP: Color = Color::from_rgb(0x42, 0xbe, 0x65);
const TREND_DOWN: Color = Color::from_rgb(0xfa, 0x4d, 0x56);
const ERROR: Color = Color::from_rgb(0xfa, 0x4d, 0x56);
const BORDER: Color = Color::from_rgb(0x52, 0x52, 0x52);
const BADGE_BG_ALPHA: f32 = 0.15;
const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;
const CHART_FILL_TOP_ALPHA: f32 = 0.15;
const CHART_FILL_BOTTOM_ALPHA: f32 = 0.02;
const CLOSED_ALPHA: f32 = 0.4;
const ERROR_ROW_ALPHA: f32 = 0.6;
const CLOSED_MARKER_SCALE: f32 = 0.4;

fn fixed_width(width: f32) -> Node {
    col(props!(width: width), Vec::<Node>::new())
}

fn h_divider() -> Node {
    col(props!(height: 1.0, background: BORDER), Vec::<Node>::new())
}

fn v_divider() -> Node {
    col(props!(width: 1.0, background: BORDER), Vec::<Node>::new())
}

fn empty_row() -> Node {
    col(
        props!(flex: 1.0, background: BACKGROUND),
        Vec::<Node>::new(),
    )
}

/// The gray stop-marker square shown when the market is closed. A filled rect
/// node, not a text glyph — the embedded fonts have no Geometric Shapes
/// coverage, so U+25A0 would render as a missing-glyph box.
fn closed_marker(band: &Band) -> Node {
    #[expect(
        clippy::cast_precision_loss,
        reason = "scaled font sizes are small, exact in f32"
    )]
    let side = scale_font(band.symbol_font, CLOSED_MARKER_SCALE) as f32;
    col(
        props!(width: side, height: side, background: SECONDARY),
        Vec::<Node>::new(),
    )
}

/// `symbol` line, with a gray stop-marker after it when the market is closed
/// and, when the row is stale, a badge aging from its last good load.
fn symbol_line(
    symbol: &str,
    color: Color,
    band: &Band,
    closed: bool,
    stale: Option<SystemTime>,
) -> Node {
    let mut children = vec![text(
        symbol,
        style!(size: band.symbol_font, weight: FontWeight::BOLD, color: color),
    )];
    if closed {
        children.push(fixed_width(band.row_gap));
        children.push(closed_marker(band));
    }
    if let Some(anchor) = stale {
        children.push(fixed_width(band.row_gap));
        children.push(stale_badge(band, anchor));
    }
    row(props!(cross_align: CrossAlign::Center), children)
}

fn sparkline_node(series: &[f64], trend: Color, closed: bool, band: &Band) -> Node {
    let margin = band.row_padding;
    let (w, h) = (band.chart_width, band.chart_height);
    let color = if closed { SECONDARY } else { trend };
    let alpha = if closed { CLOSED_ALPHA } else { 1.0 };
    let line = chart::series_points(series, w, h, CHART_INSET);
    let canvas_node = if line.len() < 2 {
        fixed_width(w)
    } else {
        let mut area = line.clone();
        area.push((w, h));
        area.push((0.0, h));
        canvas(
            props!(width: w, height: h),
            [
                fill!(
                    area,
                    linear: (
                        color.with_alpha(CHART_FILL_TOP_ALPHA * alpha),
                        color.with_alpha(CHART_FILL_BOTTOM_ALPHA * alpha)
                    )
                ),
                path!(line, stroke: CHART_STROKE, color: color.with_alpha(alpha)),
            ],
        )
    };
    row(
        props!(cross_align: CrossAlign::Center),
        [fixed_width(margin), canvas_node, fixed_width(margin)],
    )
}

fn badge_node(text_str: String, trend: Color, band: &Band) -> Node {
    row(
        props!(background: trend.with_alpha(BADGE_BG_ALPHA), padding: band.badge_padding),
        [text(
            text_str,
            style!(size: band.change_font, weight: FontWeight::BOLD, color: trend),
        )],
    )
}

fn right_col(price_str: String, change_str: String, trend: Color, band: &Band) -> Node {
    col(
        props!(cross_align: CrossAlign::End, gap: band.row_gap),
        [
            text(
                price_str,
                style!(size: band.price_font, weight: FontWeight::BOLD, color: PRIMARY, align: TextAlign::Right),
            ),
            badge_node(change_str, trend, band),
        ],
    )
}

fn resolved_row(
    row_data: &TickerRow,
    name: Option<&str>,
    stale: Option<SystemTime>,
    band: &Band,
) -> Node {
    let trend = if row_data.is_positive() {
        TREND_UP
    } else {
        TREND_DOWN
    };
    let closed = row_data.is_closed_marked();
    let company = name
        .map(|n| truncate_name(n, band.company_chars))
        .unwrap_or_default();
    let left = col(
        props!(flex: 1.0, cross_align: CrossAlign::Start, gap: band.row_gap),
        [
            symbol_line(&row_data.symbol, PRIMARY, band, closed, stale),
            text(company, style!(size: band.company_font, color: SECONDARY)),
        ],
    );
    let price = match price_precision(&row_data.symbol, row_data.price) {
        PricePrecision::Fraction(digits) => format_number!(row_data.price, digits),
        PricePrecision::BelowMin => {
            let mut out = String::from("<");
            out.push_str(&format_number!(MIN_PRICE, 6));
            out
        }
    };
    let right = right_col(price, change_text(row_data.change_pct), trend, band);

    let mut children = vec![left];
    if band.show_sparkline {
        children.push(sparkline_node(&row_data.series, trend, closed, band));
    }
    children.push(right);
    row(
        props!(flex: 1.0, cross_align: CrossAlign::Center, padding: band.row_padding),
        children,
    )
}

/// Per-row counterpart to the SDK's `with_stale_overlay` pill, rendered inline
/// after the symbol: a list polls each row separately, so a single pill floated
/// over the root could not say *which* rows are holding an old series.
///
/// It carries the SDK pill's Carbon Warning tokens rather than a palette of its
/// own, but builds its own chrome off the band — `Node::Tag`'s padding and icon
/// are fixed host-side constants, which at a 0.67 `fit` would leave a badge
/// wider than the sparkline sitting beside a 21px symbol. `band.stale_label`
/// adds the age; bands too narrow for it show the icon alone.
fn stale_badge(band: &Band, anchor: SystemTime) -> Node {
    let mut children = vec![canvas(
        props!(width: band.stale_icon, height: band.stale_icon),
        [Draw::svg_builtin(
            0.0,
            0.0,
            band.stale_icon,
            band.stale_icon,
            ICON_WARNING,
            ORANGE_40,
        )],
    )];
    if band.stale_label {
        children.push(relative_time_live(
            anchor,
            // Bare "5m", not the pill's "Last refresh 5m ago" — a row has room
            // for a magnitude, not a sentence.
            RelTimeFormat {
                length: RelTimeLength::Short,
                segments: RelTimeSegments::Single,
            },
            // An age only counts up; never "in …" on a clock step back.
            RelTimeClamp::ElapsedOnly,
            TextStyle {
                size: band.stale_font,
                weight: FontWeight::BOLD,
                color: ORANGE_40,
                ..TextStyle::default()
            },
        ));
    }
    row(
        props!(
            background: GRAY_100,
            padding: band.badge_padding,
            gap: band.badge_padding,
            cross_align: CrossAlign::Center
        ),
        children,
    )
}

/// A placeholder row: symbol (error-colored for not-found, gray otherwise) +
/// a short status, price `N/A`, and the whole row dimmed to 0.6.
fn placeholder_row(symbol: &str, status: &str, symbol_color: Color, band: &Band) -> Node {
    let sym = symbol_color.with_alpha(ERROR_ROW_ALPHA);
    let muted = SECONDARY.with_alpha(ERROR_ROW_ALPHA);
    let left = col(
        props!(flex: 1.0, cross_align: CrossAlign::Start, gap: band.row_gap),
        [
            symbol_line(symbol, sym, band, false, None),
            text(status, style!(size: band.company_font, color: muted)),
        ],
    );
    let right = col(
        props!(cross_align: CrossAlign::End),
        [text(
            "N/A",
            style!(size: band.price_font, weight: FontWeight::BOLD, color: muted, align: TextAlign::Right),
        )],
    );
    let mut children = vec![left];
    if band.show_sparkline {
        children.push(fixed_width(band.chart_width + band.row_padding * 2.0));
    }
    children.push(right);
    row(
        props!(flex: 1.0, cross_align: CrossAlign::Center, padding: band.row_padding),
        children,
    )
}

fn slot(
    index: usize,
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    band: &Band,
) -> Node {
    let Some(symbol) = symbols.get(index) else {
        return empty_row();
    };
    let Some(row_state) = states.get(index) else {
        return empty_row();
    };
    match row_state {
        RowState::Resolved { data } => resolved_row(
            data,
            names.get(index).and_then(Option::as_deref),
            stale.get(index).copied().flatten(),
            band,
        ),
        RowState::InputError { .. } => placeholder_row(symbol, "Not found", ERROR, band),
        RowState::NoData { market_closed } => {
            let text = if *market_closed { "Closed" } else { "No data" };
            placeholder_row(symbol, text, SECONDARY, band)
        }
        RowState::Failed => placeholder_row(symbol, "Unavailable", SECONDARY, band),
        RowState::Loading => placeholder_row(symbol, "Loading\u{2026}", SECONDARY, band),
    }
}

/// The full grid for the current size.
#[must_use]
pub fn view(
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    ws: WidgetSize,
) -> Node {
    let band = band_for(ws.variant).scaled(ws.fit());
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
    let (w, h) = (ws.width as f32, ws.height as f32);

    let mut children = Vec::new();
    if band.columns == 2 {
        let grid_rows = band.rows / 2;
        for r in 0..grid_rows {
            let left = slot(2 * r, symbols, states, names, stale, &band);
            let right = slot(2 * r + 1, symbols, states, names, stale, &band);
            children.push(row(
                props!(flex: 1.0, cross_align: CrossAlign::Center),
                [left, v_divider(), right],
            ));
            if r + 1 < grid_rows {
                children.push(h_divider());
            }
        }
    } else {
        for i in 0..band.rows {
            children.push(slot(i, symbols, states, names, stale, &band));
            if i + 1 < band.rows {
                children.push(h_divider());
            }
        }
    }
    col(
        props!(background: BACKGROUND, width: w, height: h),
        children,
    )
}

/// A centered full-widget message (invalid symbols / all failed).
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
                style!(size: band.company_font, color: SECONDARY, align: TextAlign::Center),
            ),
            spacer(1.0),
        ],
    )
}

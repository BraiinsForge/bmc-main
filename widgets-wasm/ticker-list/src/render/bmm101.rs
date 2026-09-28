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

//! BMM101's 480×320 frame: the widget's name over four ruled rows,
//! each a symbol and name, a sparkline, and a price over its change.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports and macros"
    )
)]
use bmc_wasm_sdk::*;

use super::{
    Cells, Grid, NO_SYMBOLS, Paint, Slot, fixed_width, list, price_text, slot, sparkline,
    stale_line, symbol_line,
};
use crate::layout::{BMM101_ROWS, Band};
use crate::model::{RowState, TickerRow};
use prices::closed_market::CLOSED_CHART_ALPHA;
use prices::format::change_text;

const EDGE: f32 = 16.0;
/// Figma's `normal` line box for Braiins Sans, which every slot below is measured in.
const LINE_HEIGHT: f32 = 1.3;
const TITLE: &str = "Financial Ticker List";
const TITLE_SIZE: u32 = 14;
const TITLE_SLOT: f32 = 18.0;
const BODY_SIZE: u32 = 20;
const SUB_SIZE: u32 = 16;
const SUB_SLOT: f32 = 21.0;
/// Between a symbol and its name, and between a price and its change.
const LINE_GAP: f32 = 2.0;
const RULE_GAP: f32 = 8.0;
// The design's grid: a 100 px chart 44 px off each column, the prices no narrower than 110 px.
const CHART_WIDTH: f32 = 100.0;
const CHART_GAP: f32 = 44.0;
const PRICE_MIN_WIDTH: f32 = 110.0;
const CHART_STROKE: f32 = 3.0;
/// The fill fades from these at the line to nothing;
/// the falling tint is stronger to read on black, as on the miner faces' charts.
const RISING_FILL_ALPHA: f32 = 0.16;
const FALLING_FILL_ALPHA: f32 = 0.30;
const TAG_PADDING_X: f32 = 4.0;
const TAG_PADDING_Y: f32 = 2.0;
const TAG_RADIUS: f32 = 4.0;
/// Between a symbol and its pause marker.
const MARKER_GAP: f32 = 4.0;
const MARKER_SIZE: f32 = 16.0;
/// A placeholder row dims as it does on the Deck.
const PLACEHOLDER_ALPHA: f32 = 0.6;

/// The sizes the shared row helpers read, for rows `row_height` tall.
fn band(row_height: f32) -> Band {
    Band {
        symbol_font: BODY_SIZE,
        company_font: SUB_SIZE,
        price_font: BODY_SIZE,
        change_font: BODY_SIZE,
        chart_width: CHART_WIDTH,
        chart_height: row_height,
        badge_padding: 0.0,
        row_padding: 0.0,
        row_gap: MARKER_GAP,
        rows: BMM101_ROWS,
        columns: 1,
        show_sparkline: true,
        marker_size: MARKER_SIZE,
    }
}

fn line_box(height: f32, content: Node) -> Node {
    col(props!(height: height), [content])
}

fn symbol(symbol: &str, color: Color) -> Node {
    text(
        symbol,
        style!(
            size: BODY_SIZE,
            weight: FontWeight::SEMIBOLD,
            color: color,
            line_height: LINE_HEIGHT,
            text_overflow: TextOverflow::Ellipsis
        ),
    )
}

fn sub_style(color: Color) -> StyleResult {
    style!(
        size: SUB_SIZE,
        color: color,
        line_height: LINE_HEIGHT,
        text_overflow: TextOverflow::Ellipsis
    )
}

fn sub_line(value: &str, color: Color) -> Node {
    line_box(SUB_SLOT, text(value, sub_style(color)))
}

fn figure(value: impl Into<String>, color: Color) -> Node {
    text(
        value,
        style!(
            size: BODY_SIZE,
            weight: FontWeight::SEMIBOLD,
            color: color,
            line_height: LINE_HEIGHT,
            align: TextAlign::Right
        ),
    )
}

fn name_cell(symbol_line: Node, sub: Node) -> Node {
    col(props!(flex: 1.0, gap: LINE_GAP), [symbol_line, sub])
}

fn paint(rising: bool, closed: bool) -> Paint {
    let (color, fill) = if rising {
        (GREEN_40, RISING_FILL_ALPHA)
    } else {
        (RED_50, FALLING_FILL_ALPHA)
    };
    let (color, alpha) = if closed {
        (GRAY_40, CLOSED_CHART_ALPHA)
    } else {
        (color, 1.0)
    };
    Paint {
        line: color.with_alpha(alpha),
        fill_top: color.with_alpha(fill * alpha),
        fill_bottom: color.with_alpha(0.0),
        stroke: CHART_STROKE,
    }
}

fn tag(change: String, rising: bool) -> Node {
    let (background, color) = if rising {
        (GREEN_90, GREEN_30)
    } else {
        (RED_90, RED_30)
    };
    let side = TAG_PADDING_X - TAG_PADDING_Y;
    row(
        props!(background: background, border_radius: TAG_RADIUS, padding: TAG_PADDING_Y),
        [
            fixed_width(side),
            text(
                change,
                style!(
                    size: BODY_SIZE,
                    weight: FontWeight::SEMIBOLD,
                    color: color,
                    line_height: LINE_HEIGHT
                ),
            ),
            fixed_width(side),
        ],
    )
}

fn resolved(data: &TickerRow, name: Option<&str>, stale: Option<SystemTime>, band: &Band) -> Cells {
    let rising = data.is_positive();
    let closed = data.is_closed_marked();
    Cells {
        name: name_cell(
            symbol_line(symbol(&data.symbol, WHITE), band, GRAY_40, closed),
            match stale {
                Some(anchor) => line_box(SUB_SLOT, stale_line(anchor, sub_style(GRAY_40).0, band)),
                None => sub_line(name.unwrap_or_default(), GRAY_40),
            },
        ),
        chart: sparkline(&data.series, &paint(rising, closed), band),
        price: col(
            props!(cross_align: CrossAlign::End, gap: LINE_GAP),
            [
                figure(price_text(data), WHITE),
                tag(change_text(data.change_pct), rising),
            ],
        ),
    }
}

fn placeholder(symbol_text: &str, status: &str, not_found: bool, band: &Band) -> Cells {
    let symbol_color = if not_found { RED_50 } else { GRAY_40 };
    let muted = GRAY_40.with_alpha(PLACEHOLDER_ALPHA);
    Cells {
        name: name_cell(
            symbol_line(
                symbol(symbol_text, symbol_color.with_alpha(PLACEHOLDER_ALPHA)),
                band,
                GRAY_40,
                false,
            ),
            sub_line(status, muted),
        ),
        chart: fixed_width(CHART_WIDTH),
        price: figure("N/A", muted),
    }
}

fn cells(slot: Slot, band: &Band) -> Cells {
    match slot {
        Slot::Empty => super::empty_cells(band),
        Slot::Resolved { data, name, stale } => resolved(data, name, stale, band),
        Slot::Placeholder {
            symbol,
            status,
            not_found,
        } => placeholder(symbol, status, not_found, band),
    }
}

/// The frame after the design: the name, then four rows sized from the viewport,
/// so a price column taller than its row bleeds into the rule gap
/// instead of stretching the list past the frame.
pub(super) fn view(
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    ws: WidgetSize,
) -> Node {
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions and row counts are small, exact in f32"
    )]
    let (w, h, row_count) = (ws.width as f32, ws.height as f32, BMM101_ROWS as f32);
    let body = if symbols.is_empty() {
        col(
            props!(flex: 1.0, justify_content: Justify::Center, cross_align: CrossAlign::Center),
            [text(
                NO_SYMBOLS,
                style!(size: BODY_SIZE, color: GRAY_40, line_height: LINE_HEIGHT, align: TextAlign::Center),
            )],
        )
    } else {
        let rules = (row_count - 1.0) * (2.0 * RULE_GAP + 1.0);
        let row_height = (h - 3.0 * EDGE - TITLE_SLOT - rules) / row_count;
        let band = band(row_height);
        let grid = Grid {
            edge: 0.0,
            chart_gap: CHART_GAP,
            show_charts: true,
            rule: GRAY_90,
            rule_gap: RULE_GAP,
            row_height: Some(row_height),
            price_min_width: Some(PRICE_MIN_WIDTH),
        };
        let rows = (0..BMM101_ROWS)
            .map(|index| cells(slot(index, symbols, states, names, stale), &band))
            .collect();
        list(rows, &grid)
    };
    col(
        props!(background: BLACK, width: w, height: h, padding: EDGE, gap: EDGE),
        [
            line_box(
                TITLE_SLOT,
                text(
                    TITLE,
                    style!(
                        size: TITLE_SIZE,
                        weight: FontWeight::SEMIBOLD,
                        color: GRAY_40,
                        line_height: LINE_HEIGHT
                    ),
                ),
            ),
            body,
        ],
    )
}

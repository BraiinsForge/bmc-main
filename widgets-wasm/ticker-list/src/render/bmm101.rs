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

use super::{Grid, NO_SYMBOLS, Paint, Pending, RowStyle, fixed_width, list, row_cells, slot};
use crate::layout::BMM101_ROWS;
use crate::model::RowState;
use prices::closed_market::CLOSED_CHART_ALPHA;

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

fn line_box(height: f32, content: Node) -> Node {
    col(props!(height: height), [content])
}

/// BMM101's rows, `row_height` tall.
struct Bmm101 {
    row_height: f32,
}

impl RowStyle for Bmm101 {
    fn symbol(&self, symbol: &str, color: Color) -> Node {
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

    fn sub_style(&self, color: Color) -> StyleResult {
        style!(
            size: SUB_SIZE,
            color: color,
            line_height: LINE_HEIGHT,
            text_overflow: TextOverflow::Ellipsis
        )
    }

    fn sub_line(&self, line: Node) -> Node {
        line_box(SUB_SLOT, line)
    }

    fn price(&self, price: String, color: Color) -> Node {
        text(
            price,
            style!(
                size: BODY_SIZE,
                weight: FontWeight::SEMIBOLD,
                color: color,
                line_height: LINE_HEIGHT,
                align: TextAlign::Right
            ),
        )
    }

    fn change(&self, change: String, rising: bool) -> Node {
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

    fn paint(&self, rising: bool, closed: bool) -> Paint {
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

    fn primary(&self) -> Color {
        WHITE
    }

    fn secondary(&self) -> Color {
        GRAY_40
    }

    fn placeholder_color(&self, reason: Pending) -> Color {
        match reason {
            Pending::NotFound => RED_50,
            Pending::Loading | Pending::Failed | Pending::NoData | Pending::Closed => GRAY_40,
        }
    }

    fn line_gap(&self) -> f32 {
        LINE_GAP
    }

    fn marker_gap(&self) -> f32 {
        MARKER_GAP
    }

    fn marker_size(&self) -> f32 {
        MARKER_SIZE
    }

    fn chart_size(&self) -> (f32, f32) {
        (CHART_WIDTH, self.row_height)
    }

    fn show_chart(&self) -> bool {
        true
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
        let row_style = Bmm101 { row_height };
        let grid = Grid {
            edge: 0.0,
            column_gap: CHART_GAP,
            show_charts: true,
            rule: GRAY_90,
            rule_gap: RULE_GAP,
            row_height: Some(row_height),
            price_min_width: Some(PRICE_MIN_WIDTH),
        };
        let rows = (0..BMM101_ROWS)
            .map(|index| row_cells(&row_style, slot(index, symbols, states, names, stale)))
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

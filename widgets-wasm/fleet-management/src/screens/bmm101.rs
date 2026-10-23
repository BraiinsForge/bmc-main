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

//! BMM101's own frame (480×320), measured off the Figma "Braiins DECK" BMM101 frames.
//! BMM100 has no design of its own and gets this frame shrunk to fit.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::history::{ChartPoint, ChartWindow};
use crate::screens::SEARCHING_HINT;
use crate::screens::parts::{
    BORDER, CARD_BG, LABEL, PagerSize, TableMetrics, area_chart, back_button, head_label, toggle,
};

pub mod dashboard;
pub mod device_detail;
pub mod model_detail;
pub mod table;

// Geometry and type at the design size, which `Bmm101` scales.
const EDGE: f32 = 16.0;
// The view switcher's height, which doesn't scale, so neither does the header.
const HEADER_H: f32 = 40.0;
const HEADER_GAP: f32 = 8.0;
const TILE_GAP: f32 = 4.0;
const CHIP: f32 = 32.0;

const TITLE_FONT: u32 = 14;
const LABEL_FONT: u32 = 16;
const ROW_FONT: u32 = 16;
const VALUE_FONT: u32 = 22;
const UNIT_FONT: u32 = 14;

/// BMM101's frame on a viewport: its type and fixed widths at a scale,
/// 1 on its own panel and below 1 on a smaller one.
#[derive(Clone, Copy, Debug)]
pub struct Bmm101 {
    scale: f32,
    width: f32,
    height: f32,
}

impl Bmm101 {
    pub const DESIGN_WIDTH: u32 = 480;
    pub const DESIGN_HEIGHT: u32 = 320;

    pub const NATIVE: Self = Self {
        scale: 1.0,
        width: dim(Self::DESIGN_WIDTH),
        height: dim(Self::DESIGN_HEIGHT),
    };

    /// Scaled by the viewport's tighter side, so everything fits;
    /// the frame still spans the whole viewport, the body taking up the slack.
    #[must_use]
    pub fn fit(width: u32, height: u32) -> Self {
        Self {
            scale: ratio(width, Self::DESIGN_WIDTH).min(ratio(height, Self::DESIGN_HEIGHT)),
            width: dim(width),
            height: dim(height),
        }
    }

    #[must_use]
    pub const fn scale(self) -> f32 {
        self.scale
    }

    fn px(self, design: f32) -> f32 {
        design * self.scale
    }

    fn font(self, design: u32) -> u32 {
        scale_font(design, self.scale)
    }

    // A text's `max_width`, which the style takes in whole pixels.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive width of a few hundred pixels"
    )]
    fn text_width(self, design: f32) -> u32 {
        self.px(design) as u32
    }

    fn width(self) -> f32 {
        self.width
    }

    fn height(self) -> f32 {
        self.height
    }

    // The body below the header, inside the edge.
    fn body_width(self) -> f32 {
        self.width() - 2.0 * self.px(EDGE)
    }

    fn body_height(self) -> f32 {
        self.height() - 2.0 * self.px(EDGE) - HEADER_H - self.px(HEADER_GAP)
    }

    fn pager(self) -> PagerSize {
        PagerSize {
            width: self.px(40.0),
            chip: self.px(CHIP),
            gap: self.px(6.0),
            font: self.font(12),
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "viewport sides are a few hundred pixels"
)]
const fn dim(px: u32) -> f32 {
    px as f32
}

fn ratio(actual: u32, design: u32) -> f32 {
    dim(actual) / dim(design)
}

fn frame(m: Bmm101, header: Node, body: Node) -> Node {
    col(
        props!(
            background: BLACK,
            width: m.width(),
            height: m.height(),
            padding: m.px(EDGE),
            gap: m.px(HEADER_GAP),
        ),
        [header, body],
    )
}

fn fleet_header(m: Bmm101, title: &str, count: usize, list_active: bool) -> Node {
    row(
        props!(height: HEADER_H, cross_align: CrossAlign::Center, gap: m.px(8.0)),
        [
            title_text(m, title, 230.0),
            count_text(m, count),
            col(props!(flex: 1.0), Vec::<Node>::new()),
            toggle(list_active),
        ],
    )
}

// The Deck's breadcrumb doesn't fit here; Back is the way out.
fn back_header(m: Bmm101, title: &str, count: Option<usize>) -> Node {
    let mut children = vec![back_button(m.px(CHIP)), title_text(m, title, 330.0)];
    children.extend(count.map(|c| count_text(m, c)));
    row(
        props!(height: HEADER_H, cross_align: CrossAlign::Center, gap: m.px(8.0)),
        children,
    )
}

fn title_text(m: Bmm101, title: &str, max_width: f32) -> Node {
    text(
        title,
        style!(
            size: m.font(TITLE_FONT),
            weight: FontWeight::BOLD,
            color: WHITE,
            max_width: m.text_width(max_width),
            text_overflow: TextOverflow::Ellipsis,
        ),
    )
}

fn count_text(m: Bmm101, count: usize) -> Node {
    text(
        typography::unbroken(fmt!("{count} Devices")),
        style!(size: m.font(TITLE_FONT), color: LABEL),
    )
}

fn tile(m: Bmm101, width: f32, height: f32, children: Vec<Node>) -> Node {
    col(
        props!(background: BORDER, width: width, height: height, padding: 1.0),
        [center(
            props!(background: CARD_BG, flex: 1.0),
            [col(
                props!(gap: m.px(8.0), cross_align: CrossAlign::Center),
                children,
            )],
        )],
    )
}

fn tile_label(m: Bmm101, label: &str) -> Node {
    text(label, style!(size: m.font(LABEL_FONT), color: LABEL))
}

fn value_row(m: Bmm101, value: &str, unit: &str) -> Node {
    row(
        props!(gap: m.px(4.0), cross_align: CrossAlign::Center),
        [
            text(
                value,
                style!(size: m.font(VALUE_FONT), weight: FontWeight::SEMIBOLD, color: WHITE),
            ),
            text(unit, style!(size: m.font(UNIT_FONT), color: LABEL)),
        ],
    )
}

// ── Tables ───────────────────────────────────────────────────────────

const HEAD_H: f32 = 38.0;
// Four rows a page fit under the head, matching the Deck's page size.
const DATA_H: f32 = 48.0;
const ROW_PAD: f32 = 12.0;
const COL_GAP: f32 = 12.0;
const PAGER_GAP: f32 = 8.0;
const SPARK_W: f32 = 44.0;

impl Bmm101 {
    fn table(self) -> TableMetrics {
        TableMetrics {
            head_h: self.px(HEAD_H),
            row_h: self.px(DATA_H),
            edge: self.px(ROW_PAD),
            gutter: self.px(COL_GAP),
        }
    }

    // The card's width, leaving the pager its strip.
    fn table_width(self) -> f32 {
        self.body_width() - self.px(PAGER_GAP) - self.pager().width
    }
}

fn head(m: Bmm101, label: &str) -> Node {
    head_label(label, m.font(UNIT_FONT))
}

fn paged(m: Bmm101, table: Node, pager: Node) -> Node {
    row(props!(gap: m.px(PAGER_GAP), flex: 1.0), [table, pager])
}

// A row's leading name, cut short with an ellipsis by the column it grows in.
fn name_text(m: Bmm101, name: &str, weight: FontWeight) -> Node {
    text(
        name,
        style!(size: m.font(ROW_FONT), weight: weight, color: WHITE, text_overflow: TextOverflow::Ellipsis),
    )
}

// A figure with its unit, never broken over two lines.
fn value_text(m: Bmm101, number: &str, unit: &str) -> Node {
    text(
        typography::unbroken(fmt!("{number} {unit}")),
        style!(size: m.font(ROW_FONT), color: WHITE),
    )
}

fn sparkline(m: Bmm101, series: &[Vec<ChartPoint>], window: ChartWindow) -> Node {
    let (w, h) = (m.px(SPARK_W), m.px(24.0));
    canvas(
        props!(width: w, height: h),
        area_chart(series, window, w, h, 0.15),
    )
}

/// Fleet-empty state on BMM101: the Deck's, in the smaller type,
/// with the hint wrapped to the narrower panel.
#[must_use]
pub fn searching(m: Bmm101) -> Node {
    col(
        props!(background: BLACK, width: m.width(), height: m.height()),
        [center(
            props!(flex: 1.0),
            [col(
                props!(gap: m.px(12.0), cross_align: CrossAlign::Center),
                [
                    text(
                        "Searching for miners\u{2026}",
                        style!(size: m.font(20), color: WHITE),
                    ),
                    col(
                        props!(width: m.px(360.0)),
                        [text(
                            SEARCHING_HINT,
                            style!(size: m.font(14), color: GRAY_60, align: TextAlign::Center),
                        )],
                    ),
                    col(
                        props!(width: m.px(200.0)),
                        [progress_bar!(
                            ProgressMode::Indeterminate,
                            track_h: m.px(4.0), active: true, fill_color: WHITE,
                            track_color: TRANSPARENT, bg_color: TRANSPARENT,
                        )],
                    ),
                ],
            )],
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmm100_fills_its_whole_panel_though_its_type_shrinks() {
        let m = Bmm101::fit(320, 240);
        assert!(m.scale() < 1.0, "BMM100 is narrower than the design");
        assert_eq!(
            (m.width(), m.height()),
            (320.0, 240.0),
            "a 4:3 panel must not be letterboxed to BMM101's 3:2"
        );
    }

    #[test]
    fn the_native_frame_is_bmm101s_panel() {
        assert_eq!(
            (Bmm101::NATIVE.width(), Bmm101::NATIVE.height()),
            (480.0, 320.0)
        );
    }
}

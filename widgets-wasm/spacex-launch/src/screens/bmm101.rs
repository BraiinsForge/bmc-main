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

//! BMM101's own frame (480×320): the header, the mission as the hero,
//! and the launch's details in two ruled columns.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::model::{LaunchData, SizeBucket};
use crate::screens::parts::{BRAND, RowStyle, detail_rows, divider, launch_info_rows};

pub(super) const EDGE: f32 = 16.0;
/// Figma's line boxes: 14 in 18, 16 in 21, 20 in 26.
const LINE_HEIGHT: f32 = 1.3;
const HEADER_SIZE: u32 = 14;
const HEADER_H: f32 = (HEADER_SIZE as f32 * LINE_HEIGHT).floor();
pub(super) const BODY_SIZE: u32 = 20;
const SUB_SIZE: u32 = 16;
/// Under the header, and between the rule and the grid.
const GAP: f32 = 8.0;
const CAPTION_GAP: f32 = 4.0;

const HERO_W: f32 = SizeBucket::Bmm101.design_size().0 as f32 - 2.0 * EDGE;
const HERO_MAX: u32 = 40;
const HERO_MIN: u16 = 24;
/// Two lines at the floor, so a name too wide for one line at the top size
/// shrinks until it wraps onto two.
const HERO_H: f32 = 2.0 * HERO_MIN as f32 * LINE_HEIGHT;

const GRID_H: f32 = 152.0;
const LEFT_W: f32 = 228.0;
const COLUMN_GAP: f32 = 24.0;
const RIGHT_W: f32 = HERO_W - LEFT_W - COLUMN_GAP;

const ROW: RowStyle = RowStyle {
    size: SUB_SIZE,
    line_height: LINE_HEIGHT,
    label_color: GRAY_40,
    value_color: WHITE,
    value_weight: FontWeight::SEMIBOLD,
    value_align: TextAlign::Right,
};

/// The header and hero at the top, the ruled grid on the bottom edge.
pub(super) fn launch(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(
            padding: EDGE,
            background: BLACK,
            justify_content: Justify::SpaceBetween,
        ),
        [
            col(props!(gap: GAP), [header(), hero(&data.mission_name)]),
            col(props!(gap: GAP), [divider(), grid(data, countdown, status)]),
        ],
    )
}

pub(super) fn header() -> Node {
    row(
        props!(height: HEADER_H, gap: 8.0, cross_align: CrossAlign::Center),
        [
            text(
                BRAND,
                style!(
                    size: HEADER_SIZE,
                    weight: FontWeight::BOLD,
                    color: WHITE,
                    line_height: LINE_HEIGHT,
                ),
            ),
            text(
                "Next Launch",
                style!(
                    size: HEADER_SIZE,
                    weight: FontWeight::SEMIBOLD,
                    color: GRAY_40,
                    line_height: LINE_HEIGHT,
                ),
            ),
        ],
    )
}

/// The renderer measures the name, so none is budgeted by its length.
fn hero(mission: &str) -> Node {
    col(
        props!(gap: CAPTION_GAP),
        [
            canvas(
                props!(width: HERO_W, height: HERO_H),
                [Draw::autofit_text_ranged(
                    0.0,
                    0.0,
                    HERO_W,
                    HERO_H,
                    mission,
                    style!(
                        size: HERO_MAX,
                        weight: FontWeight::BOLD,
                        color: WHITE,
                        line_height: LINE_HEIGHT,
                    ),
                    AutoFit::Shrink,
                    HERO_MIN,
                    0,
                )],
            ),
            text(
                "Mission name",
                style!(size: BODY_SIZE, color: GRAY_40, line_height: LINE_HEIGHT),
            ),
        ],
    )
}

fn grid(data: &LaunchData, countdown: &str, status: &str) -> Node {
    row(
        props!(gap: COLUMN_GAP),
        [
            column(LEFT_W, launch_info_rows(data, countdown, status, ROW)),
            column(RIGHT_W, detail_rows(data, ROW)),
        ],
    )
}

/// Fixed height with its rows spread over it, so a value that wraps
/// takes its line from the spacing instead of pushing the grid off the frame.
fn column(width: f32, rows: [Node; 7]) -> Node {
    col(
        props!(
            width: width,
            height: GRID_H,
            justify_content: Justify::SpaceBetween,
        ),
        rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::{fixtures, launch_view};

    fn healthy() -> Node {
        launch_view(&fixtures::healthy(fixtures::at_bucket(SizeBucket::Bmm101)))
    }

    /// Width and height of every column the frame sets a width on.
    fn sized_columns(node: &Node) -> Vec<(f32, f32)> {
        match node {
            Node::Column(props, children) => {
                let own = (props.width > 0.0).then_some((props.width, props.height));
                own.into_iter()
                    .chain(children.iter().flat_map(sized_columns))
                    .collect()
            }
            Node::Row(_, children) | Node::Center(_, children) => {
                children.iter().flat_map(sized_columns).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Every autofit text's box, how it fits, and the sizes it may take.
    fn autofits(node: &Node) -> Vec<(f32, f32, AutoFit, u16, u32)> {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                children.iter().flat_map(autofits).collect()
            }
            Node::Canvas { draws, .. } => draws
                .iter()
                .filter_map(|draw| match draw {
                    Draw::AutofitText {
                        box_width,
                        box_height,
                        mode,
                        min_size,
                        style,
                        ..
                    } => Some((*box_width, *box_height, *mode, *min_size, style.size)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn the_hero_spans_the_frame_and_shrinks_from_40_to_two_lines_at_24() {
        assert_eq!(
            autofits(&healthy()),
            [(448.0, 2.0 * 24.0 * LINE_HEIGHT, AutoFit::Shrink, 24, 40)]
        );
    }

    #[test]
    fn the_grid_splits_the_frame_width_into_the_designed_columns() {
        assert_eq!(sized_columns(&healthy()), [(228.0, 152.0), (196.0, 152.0)]);
    }

    #[test]
    fn the_fixed_parts_stack_within_the_frame_height_with_a_gap_over_the_rule() {
        let caption_h = BODY_SIZE as f32 * LINE_HEIGHT;
        let top = HEADER_H + GAP + HERO_H + CAPTION_GAP + caption_h;
        let bottom = crate::screens::parts::DIVIDER_THICKNESS + GAP + GRID_H;
        let height = SizeBucket::Bmm101.design_size().1 as f32;
        assert!(
            2.0 * EDGE + top + GAP + bottom <= height,
            "{top} + {bottom} between the edges leaves under {GAP} over the rule in {height}"
        );
    }
}

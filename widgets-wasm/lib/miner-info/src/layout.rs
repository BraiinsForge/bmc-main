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

use bmc_wasm_sdk::{ViewportShape, WidgetViewport};

/// The panel a viewport is drawn for, which picks the face and what it fetches.
///
/// `Small` is the BMC100 small slot and the BMM100; `Bmm101` the 480×320 board;
/// `Round` the BFM100. The manifests admit no rectangle wider than BMM101.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Small,
    Bmm101,
    Round,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent per-field visibility toggles, not a state enum"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoOverloadFields {
    pub show_price_graph: bool,
    pub show_hashvalue: bool,
    pub show_fee_percent: bool,
    pub show_difficulty_row: bool,
    /// Blocks across the grid. At two, `Block Height` moves out of the primary
    /// row into the bottom one, which has the slot to spare.
    pub grid_columns: usize,
}

/// The BMM101 frame, which its faces lay out at fixed widths.
const BMM101_WIDTH: u32 = 480;
const BMM101_HEIGHT: u32 = 320;

/// A rectangle is BMM101 only when the frame fits whole: the manifests admit
/// every size between the small slot and the board, and the small faces
/// are the ones that flex.
#[must_use]
pub fn classify(viewport: WidgetViewport) -> Panel {
    if viewport.shape == ViewportShape::Round {
        Panel::Round
    } else if viewport.width >= BMM101_WIDTH && viewport.height >= BMM101_HEIGHT {
        Panel::Bmm101
    } else {
        Panel::Small
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSizes {
    pub title: u32,
    pub value: u32,
    /// The unit trailing a value; set apart from it only where the frame does.
    pub unit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiningLayout {
    pub padding_horizontal: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    pub text: TextSizes,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockLayout {
    pub padding_horizontal: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    pub horizontal_gap: f32,
    pub vertical_gap: f32,
    pub block_width: f32,
    pub block_height: f32,
    pub text: TextSizes,
}

/// The Geek list: a title row, then the lines and their rules spread over the
/// rest of the height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeekLayout {
    /// The margin on every side, and the rules' inset.
    pub edge: f32,
    pub title_to_rows: f32,
    pub text: TextSizes,
}

/// BMM101 follows its frame; the small panel takes the same list
/// at the type size its other faces use, with margins to match.
#[must_use]
pub(crate) fn geek_layout(panel: Panel) -> GeekLayout {
    match panel {
        Panel::Small => GeekLayout {
            edge: 12.0,
            title_to_rows: 8.0,
            text: TextSizes {
                title: 16,
                value: 16,
                unit: 16,
            },
        },
        Panel::Bmm101 => GeekLayout {
            edge: 16.0,
            title_to_rows: 16.0,
            text: TextSizes {
                title: 20,
                value: 20,
                unit: 20,
            },
        },
        Panel::Round => unreachable!("BUG: the round Geek face is the gauge, not the list"),
    }
}

/// The line list the rectangular Mining face draws on the small panel.
#[must_use]
pub(crate) fn mining_layout() -> MiningLayout {
    MiningLayout {
        padding_horizontal: 16.0,
        padding_top: 16.0,
        padding_bottom: 22.0,
        text: TextSizes {
            title: 16,
            value: 16,
            unit: 16,
        },
    }
}

/// Blocks are laid at a fixed width, so a row wider than its screen clips
/// rather than reflowing. Three blocks need 479 px of a 480 px screen,
/// and the 317 px BMC100 slot has room for two —
/// the narrower block is what keeps that pair inside it.
///
/// BMM101's grid follows its Figma frame; its zero vertical gap turns into
/// flex spacers, so the rows spread over the height below the price band.
#[must_use]
pub(crate) fn info_overload_layout(panel: Panel) -> BlockLayout {
    match panel {
        Panel::Bmm101 => BlockLayout {
            padding_horizontal: 16.0,
            padding_top: 13.0,
            padding_bottom: 16.0,
            horizontal_gap: 8.0,
            vertical_gap: 0.0,
            block_width: 144.0,
            block_height: 46.0,
            text: TextSizes {
                title: 14,
                value: 20,
                unit: 14,
            },
        },
        Panel::Small | Panel::Round => BlockLayout {
            padding_horizontal: 16.0,
            padding_top: 24.0,
            padding_bottom: 24.0,
            horizontal_gap: 24.0,
            vertical_gap: 15.0,
            block_width: if panel == Panel::Small { 130.0 } else { 133.0 },
            block_height: 41.0,
            text: TextSizes {
                title: 16,
                value: 16,
                unit: 16,
            },
        },
    }
}

#[must_use]
pub(crate) fn info_overload_fields(panel: Panel) -> InfoOverloadFields {
    match panel {
        Panel::Small => InfoOverloadFields {
            show_price_graph: false,
            show_hashvalue: false,
            show_fee_percent: false,
            show_difficulty_row: false,
            grid_columns: 2,
        },
        Panel::Bmm101 | Panel::Round => InfoOverloadFields {
            show_price_graph: true,
            show_hashvalue: true,
            show_fee_percent: true,
            show_difficulty_row: true,
            grid_columns: 3,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangular(width: u32, height: u32) -> WidgetViewport {
        WidgetViewport {
            width,
            height,
            shape: ViewportShape::Rectangular,
        }
    }

    #[test]
    fn classifies_each_shipped_geometry() {
        for (viewport, panel, screen) in [
            (rectangular(317, 238), Panel::Small, "BMC100 small slot"),
            (rectangular(320, 240), Panel::Small, "BMM100"),
            (rectangular(480, 320), Panel::Bmm101, "BMM101"),
            (
                WidgetViewport {
                    width: 480,
                    height: 480,
                    shape: ViewportShape::Round,
                },
                Panel::Round,
                "BFM100",
            ),
        ] {
            assert_eq!(classify(viewport), panel, "{screen}");
        }
    }

    /// An admitted size short of the frame on either side takes the small
    /// faces, which flex, rather than a grid that would clip.
    #[test]
    fn a_rectangle_is_small_until_the_bmm101_frame_fits() {
        assert_eq!(classify(rectangular(479, 320)), Panel::Small);
        assert_eq!(classify(rectangular(480, 319)), Panel::Small);
        assert_eq!(classify(rectangular(400, 300)), Panel::Small);
        assert_eq!(classify(rectangular(480, 320)), Panel::Bmm101);
    }

    #[test]
    fn hides_info_overload_secondary_fields_on_small_viewport() {
        let fields = info_overload_fields(Panel::Small);
        assert!(!fields.show_price_graph);
        assert!(!fields.show_hashvalue);
        assert!(!fields.show_fee_percent);
        assert!(!fields.show_difficulty_row);
    }

    #[test]
    fn mining_layout_matches_boser_theme_for_bmm100() {
        assert_eq!(
            mining_layout(),
            MiningLayout {
                padding_horizontal: 16.0,
                padding_top: 16.0,
                padding_bottom: 22.0,
                text: TextSizes {
                    title: 16,
                    value: 16,
                    unit: 16,
                }
            }
        );
    }

    #[test]
    fn info_overload_layout_keeps_boser_grid_on_the_round_panel() {
        assert_eq!(
            info_overload_layout(Panel::Round),
            BlockLayout {
                padding_horizontal: 16.0,
                padding_top: 24.0,
                padding_bottom: 24.0,
                horizontal_gap: 24.0,
                vertical_gap: 15.0,
                block_width: 133.0,
                block_height: 41.0,
                text: TextSizes {
                    title: 16,
                    value: 16,
                    unit: 16,
                }
            }
        );
    }

    #[test]
    fn each_grid_fits_the_narrowest_screen_it_serves() {
        for (panel, width, screen) in [
            (Panel::Bmm101, 480.0, "BMM101"),
            (Panel::Round, 480.0, "BFM100"),
            (Panel::Small, 317.0, "BMC100 small"),
        ] {
            let metrics = info_overload_layout(panel);
            let columns = info_overload_fields(panel).grid_columns;
            #[expect(
                clippy::cast_precision_loss,
                reason = "a column count of two or three is exact in f32"
            )]
            let blocks = columns as f32;
            let used = 2.0f32.mul_add(metrics.padding_horizontal, blocks * metrics.block_width)
                + (blocks - 1.0) * metrics.horizontal_gap;
            assert!(
                used <= width,
                "{screen}: {columns} blocks need {used} px of {width}"
            );
        }
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportClass {
    Small,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
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
}

#[must_use]
pub fn classify(viewport: Viewport) -> ViewportClass {
    if viewport.width <= 320 || viewport.height <= 240 {
        ViewportClass::Small
    } else {
        ViewportClass::Large
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSizes {
    pub title: u32,
    pub value: u32,
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

#[must_use]
pub fn mining_layout(class: ViewportClass) -> MiningLayout {
    match class {
        ViewportClass::Small => MiningLayout {
            padding_horizontal: 16.0,
            padding_top: 16.0,
            padding_bottom: 22.0,
            text: TextSizes {
                title: 16,
                value: 16,
            },
        },
        ViewportClass::Large => MiningLayout {
            padding_horizontal: 16.0,
            padding_top: 28.0,
            padding_bottom: 25.0,
            text: TextSizes {
                title: 20,
                value: 20,
            },
        },
    }
}

#[must_use]
pub fn info_overload_layout() -> BlockLayout {
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
        },
    }
}

#[must_use]
pub fn info_overload_fields(class: ViewportClass) -> InfoOverloadFields {
    match class {
        ViewportClass::Small => InfoOverloadFields {
            show_price_graph: false,
            show_hashvalue: false,
            show_fee_percent: false,
            show_difficulty_row: false,
        },
        ViewportClass::Large => InfoOverloadFields {
            show_price_graph: true,
            show_hashvalue: true,
            show_fee_percent: true,
            show_difficulty_row: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treats_bmm100_as_small() {
        assert_eq!(
            classify(Viewport {
                width: 320,
                height: 240
            }),
            ViewportClass::Small
        );
    }

    #[test]
    fn treats_bmm101_as_large() {
        assert_eq!(
            classify(Viewport {
                width: 480,
                height: 320
            }),
            ViewportClass::Large
        );
    }

    #[test]
    fn hides_info_overload_secondary_fields_on_small_viewport() {
        let fields = info_overload_fields(ViewportClass::Small);
        assert!(!fields.show_price_graph);
        assert!(!fields.show_hashvalue);
        assert!(!fields.show_fee_percent);
        assert!(!fields.show_difficulty_row);
    }

    #[test]
    fn mining_layout_matches_boser_theme_for_bmm100() {
        assert_eq!(
            mining_layout(ViewportClass::Small),
            MiningLayout {
                padding_horizontal: 16.0,
                padding_top: 16.0,
                padding_bottom: 22.0,
                text: TextSizes {
                    title: 16,
                    value: 16
                }
            }
        );
    }

    #[test]
    fn info_overload_layout_keeps_boser_grid_without_graph() {
        assert_eq!(
            info_overload_layout(),
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
                    value: 16
                }
            }
        );
    }
}

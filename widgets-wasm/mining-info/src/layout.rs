// Copyright (C) 2026  Braiins Systems s.r.o.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewportClass {
    Small,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Viewport {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NetworkFields {
    pub(crate) show_extra_difficulty: bool,
    pub(crate) show_fee_percent: bool,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent per-field visibility toggles, not a state enum"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InfoOverloadFields {
    pub(crate) show_price_graph: bool,
    pub(crate) show_hashvalue: bool,
    pub(crate) show_fee_percent: bool,
    pub(crate) show_difficulty_row: bool,
}

pub(crate) fn classify(viewport: Viewport) -> ViewportClass {
    if viewport.width <= 320 || viewport.height <= 240 {
        ViewportClass::Small
    } else {
        ViewportClass::Large
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextSizes {
    pub(crate) title: u32,
    pub(crate) value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MiningLayout {
    pub(crate) padding_horizontal: f32,
    pub(crate) padding_top: f32,
    pub(crate) padding_bottom: f32,
    pub(crate) text: TextSizes,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockLayout {
    pub(crate) padding_horizontal: f32,
    pub(crate) padding_top: f32,
    pub(crate) padding_bottom: f32,
    pub(crate) horizontal_gap: f32,
    pub(crate) vertical_gap: f32,
    pub(crate) block_width: f32,
    pub(crate) block_height: f32,
    pub(crate) text: TextSizes,
}

pub(crate) fn mining_layout(class: ViewportClass) -> MiningLayout {
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

pub(crate) fn network_layout(class: ViewportClass) -> BlockLayout {
    match class {
        ViewportClass::Small => BlockLayout {
            padding_horizontal: 16.0,
            padding_top: 16.0,
            padding_bottom: 16.0,
            horizontal_gap: 24.0,
            vertical_gap: 0.0,
            block_width: 132.0,
            block_height: 41.0,
            text: TextSizes {
                title: 16,
                value: 16,
            },
        },
        ViewportClass::Large => BlockLayout {
            padding_horizontal: 16.0,
            padding_top: 24.0,
            padding_bottom: 24.0,
            horizontal_gap: 24.0,
            vertical_gap: 0.0,
            block_width: 212.0,
            block_height: 46.0,
            text: TextSizes {
                title: 20,
                value: 20,
            },
        },
    }
}

pub(crate) fn info_overload_layout() -> BlockLayout {
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

pub(crate) fn network_fields(class: ViewportClass) -> NetworkFields {
    match class {
        ViewportClass::Small => NetworkFields {
            show_extra_difficulty: false,
            show_fee_percent: false,
        },
        ViewportClass::Large => NetworkFields {
            show_extra_difficulty: true,
            show_fee_percent: true,
        },
    }
}

pub(crate) fn info_overload_fields(class: ViewportClass) -> InfoOverloadFields {
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
    fn hides_network_extras_on_small_viewport() {
        let fields = network_fields(ViewportClass::Small);
        assert!(!fields.show_extra_difficulty);
        assert!(!fields.show_fee_percent);
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
    fn network_layout_matches_boser_theme_for_bmm101() {
        assert_eq!(
            network_layout(ViewportClass::Large),
            BlockLayout {
                padding_horizontal: 16.0,
                padding_top: 24.0,
                padding_bottom: 24.0,
                horizontal_gap: 24.0,
                vertical_gap: 0.0,
                block_width: 212.0,
                block_height: 46.0,
                text: TextSizes {
                    title: 20,
                    value: 20
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

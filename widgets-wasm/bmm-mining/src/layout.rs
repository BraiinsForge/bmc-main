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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spacing {
    pub(crate) padding: f32,
    pub(crate) gap: f32,
    pub(crate) columns: usize,
}

// Per-spec (`Rendering Behavior`): responsive bands choose layout, not font
// sizes. Spacing and column count vary by class; typography stays stable.
pub(crate) fn spacing(class: ViewportClass) -> Spacing {
    match class {
        ViewportClass::Small => Spacing {
            padding: 16.0,
            gap: 8.0,
            columns: 2,
        },
        ViewportClass::Large => Spacing {
            padding: 32.0,
            gap: 14.0,
            columns: 3,
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
}

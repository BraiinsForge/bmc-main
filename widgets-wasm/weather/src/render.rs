// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod bar;
pub mod common;
pub mod full;
pub mod icons;
pub mod large;
pub mod medium;
pub mod small;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::{manifest_params::Params, model::Weather};

#[must_use]
pub fn current_view(weather: &Weather, params: &Params, size: WidgetSize) -> Node {
    match size.variant {
        SizeVariant::Full => full::full(weather, params, size),
        SizeVariant::Large => large::large(weather, params, size),
        SizeVariant::Medium => medium::medium(weather, params, size),
        SizeVariant::Small => small::small(weather, params, size),
    }
}

#[must_use]
pub fn message_view(message: &str, _size: WidgetSize) -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [common::txt(
                message.to_string(),
                32,
                FontWeight::REGULAR,
                GRAY_30,
            )],
        )],
    )
}

pub const STALE_DATA_TEXT: &str = "Stale data";

const STALE_TEXT_SIZE: u32 = 14;
const STALE_ICON_PX: f32 = 16.0;
const STALE_INSET: f32 = 8.0;

// A warning icon and the message on a panel, pinned to the bottom-left corner.
// The insets make the node absolutely positioned, so it floats over the view
// without taking part in its layout.
fn stale_banner() -> Node {
    row(
        props!(inset_bottom: STALE_INSET, inset_left: STALE_INSET),
        [row(
            props!(
                background: GRAY_100,
                padding: 6.0,
                gap: 6.0,
                cross_align: CrossAlign::Center
            ),
            [
                canvas(
                    props!(width: STALE_ICON_PX, height: STALE_ICON_PX),
                    [Draw::svg_builtin(
                        0.0,
                        0.0,
                        STALE_ICON_PX,
                        STALE_ICON_PX,
                        ICON_WARN_FILLED,
                        RED_50,
                    )],
                ),
                common::txt(STALE_DATA_TEXT, STALE_TEXT_SIZE, FontWeight::BOLD, RED_50),
            ],
        )],
    )
}

/// Float a "stale data" banner over a view's root container, so a forecast kept
/// after a failed refresh shows that it is outdated. The banner is absolutely
/// positioned and does not disturb the underlying layout.
#[must_use]
pub fn with_stale_banner(mut root: Node) -> Node {
    if let Node::Column(_, children) | Node::Row(_, children) = &mut root {
        children.push(stale_banner());
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_banner_floats_over_a_view_without_replacing_it() {
        let view = col(
            props!(),
            [common::txt("forecast", 16, FontWeight::REGULAR, GRAY_30)],
        );
        let Node::Column(_, before) = &view else {
            panic!("BUG: test view is a column");
        };
        let count = before.len();
        let wrapped = with_stale_banner(view);
        let Node::Column(_, after) = &wrapped else {
            panic!("BUG: with_stale_banner keeps the column root");
        };
        assert_eq!(after.len(), count + 1);
    }
}

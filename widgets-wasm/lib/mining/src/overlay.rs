// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared error/stale overlay banner for the mining widgets: a warning icon and
//! a message floated over a view's root column, positioned per viewport shape.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

pub const AUTH_ERROR_TEXT: &str = "Cannot authenticate";
pub const STALE_DATA_TEXT: &str = "Stale data";
const OVERLAY_TEXT_SIZE: u32 = 14;
const OVERLAY_ICON_PX: f32 = 16.0;
const OVERLAY_INSET: f32 = 8.0;
// On a round face a bottom-left banner clips against the circle, so the banner
// is centered horizontally and lifted this far above the bottom edge to stay
// inside the lower clear area.
const OVERLAY_ROUND_BOTTOM: f32 = 56.0;

// The banner itself: a warning icon and the message on a panel. It carries no
// insets so the positioning wrappers below can place it per viewport shape.
fn overlay_banner(message: &'static str) -> Node {
    row(
        props!(
            background: GRAY_100,
            padding: 6.0,
            gap: 6.0,
            cross_align: CrossAlign::Center
        ),
        [
            canvas(
                props!(width: OVERLAY_ICON_PX, height: OVERLAY_ICON_PX),
                [Draw::svg_builtin(
                    0.0,
                    0.0,
                    OVERLAY_ICON_PX,
                    OVERLAY_ICON_PX,
                    ICON_WARN_FILLED,
                    RED_50,
                )],
            ),
            text(
                message,
                style!(size: OVERLAY_TEXT_SIZE, weight: FontWeight::BOLD, color: RED_50),
            ),
        ],
    )
}

// Rectangular faces pin the banner to the bottom-left corner. Only the bottom and
// left insets are set, so it sizes to its content and anchors there, overlapping
// whatever the view draws underneath.
fn overlay_rect(message: &'static str) -> Node {
    row(
        props!(inset_bottom: OVERLAY_INSET, inset_left: OVERLAY_INSET),
        [overlay_banner(message)],
    )
}

// The round variant stretches full width (left and right insets) and centers the
// banner with flanking flex spacers, lifted above the bottom edge.
fn overlay_round(message: &'static str) -> Node {
    row(
        props!(
            inset_bottom: OVERLAY_ROUND_BOTTOM,
            inset_left: 0.0,
            inset_right: 0.0
        ),
        [spacer(1.0), overlay_banner(message), spacer(1.0)],
    )
}

// Overlay an error banner onto a view's root column as an absolute child so it
// floats over the existing layout without disturbing it. The banner is positioned
// per viewport shape: bottom-left on rectangular faces, centered and lifted on
// round faces where a corner banner would clip against the circle. The root must
// be a `Column` for the banner to attach.
#[must_use]
pub fn with_overlay(mut root: Node, message: &'static str, shape: ViewportShape) -> Node {
    if let Node::Column(_, children) = &mut root {
        let overlay = match shape {
            ViewportShape::Round => overlay_round(message),
            ViewportShape::Rectangular => overlay_rect(message),
        };
        children.push(overlay);
    }
    root
}

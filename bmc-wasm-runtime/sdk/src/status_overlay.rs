// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared status overlay — a Carbon `Tag` floated over a widget's face. Ships
//! `stale` (data age) and `error` (reason) variants; `with_overlay` places any
//! `Tag` for a bespoke one.

use bmc_wasm_protocol::{
    Color, CrossAlign, ORANGE_40, RED_50, RelTimeClamp, RelTimeFormat, RelTimeLength,
    RelTimeSegments, TagKind,
};

use crate::host::{SystemTime, ViewportShape};
use crate::props;
use crate::relative_time::relative_time_live;
use crate::tag::{TagIcon, tag};
use crate::tree::{Node, PropsData, StyleResult, TextStyle, row, spacer, text};

const LABEL_PX: u32 = 12; // Carbon label-01
const INSET: f32 = 8.0; // Carbon $spacing-03
const ROUND_BOTTOM: f32 = 56.0; // lifted clear of the round face's bottom curve

/// Tag content text — the host paints only the chrome, so the caller colors it.
fn label_style(color: Color) -> TextStyle {
    TextStyle {
        size: LABEL_PX,
        color,
        ..TextStyle::default()
    }
}

/// Warning tag reading "Last refresh N ago", self-ticking against the host clock.
#[must_use]
pub fn stale_tag(anchor: SystemTime) -> Node {
    tag(
        TagKind::Warning,
        TagIcon::Default,
        row(
            props!(cross_align: CrossAlign::Center),
            [
                text(
                    "Last refresh ",
                    StyleResult(label_style(ORANGE_40), PropsData::default()),
                ),
                relative_time_live(
                    anchor,
                    RelTimeFormat {
                        length: RelTimeLength::Short,
                        segments: RelTimeSegments::Single,
                    },
                    // A last-refresh pill only counts up; never "in …" on a clock step back.
                    RelTimeClamp::ElapsedOnly,
                    label_style(ORANGE_40),
                ),
            ],
        ),
    )
}

/// Error tag reading `reason` (e.g. "Image too large") in the Error theme.
#[must_use]
pub fn error_tag(reason: &str) -> Node {
    tag(
        TagKind::Error,
        TagIcon::Default,
        text(
            reason,
            StyleResult(label_style(RED_50), PropsData::default()),
        ),
    )
}

/// Float `tag` over `root` — bottom-left on rectangular faces, centered and
/// lifted on round ones. An absolute child, so it rides any flex root.
#[must_use]
pub fn with_overlay(mut root: Node, tag: Node, shape: ViewportShape) -> Node {
    // Only a flex container can hold the absolute overlay child; a non-container
    // root drops it silently, so trip a debug build to surface the misuse.
    debug_assert!(
        matches!(root, Node::Column(..) | Node::Row(..)),
        "with_overlay needs a Column or Row root"
    );
    if let Node::Column(_, children) | Node::Row(_, children) = &mut root {
        children.push(match shape {
            ViewportShape::Round => row(
                props!(inset_bottom: ROUND_BOTTOM, inset_left: 0.0, inset_right: 0.0),
                [spacer(1.0), tag, spacer(1.0)],
            ),
            ViewportShape::Rectangular => {
                row(props!(inset_bottom: INSET, inset_left: INSET), [tag])
            }
        });
    }
    root
}

/// Float a "Last refresh N ago" pill over `root`.
#[must_use]
pub fn with_stale_overlay(root: Node, anchor: SystemTime, shape: ViewportShape) -> Node {
    with_overlay(root, stale_tag(anchor), shape)
}

/// Float a reason-specific error tag (e.g. "Image too large") over `root`.
#[must_use]
pub fn with_error_overlay(root: Node, reason: &str, shape: ViewportShape) -> Node {
    with_overlay(root, error_tag(reason), shape)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{col, row};

    #[test]
    fn stale_overlay_attaches_one_absolute_child_to_a_column() {
        let out = with_stale_overlay(
            col(PropsData::default(), Vec::new()),
            SystemTime { unix_secs: 0 },
            ViewportShape::Rectangular,
        );
        let Node::Column(_, children) = out else {
            panic!("BUG: a Column root stays a Column");
        };
        assert_eq!(children.len(), 1, "the overlay is appended as a child");
    }

    #[test]
    fn error_overlay_attaches_to_a_row_root() {
        let out = with_error_overlay(
            row(PropsData::default(), Vec::new()),
            "Image too large",
            ViewportShape::Round,
        );
        let Node::Row(_, children) = out else {
            panic!("BUG: a Row root stays a Row");
        };
        assert_eq!(children.len(), 1, "a Row root gets the overlay too");
    }

    #[test]
    #[should_panic(expected = "Column or Row root")]
    fn non_container_root_trips_the_debug_assert() {
        // A debug build surfaces a misplaced overlay instead of dropping it.
        let _ = with_overlay(
            Node::Spacer { flex: 1.0 },
            error_tag("Unsupported image"),
            ViewportShape::Round,
        );
    }
}

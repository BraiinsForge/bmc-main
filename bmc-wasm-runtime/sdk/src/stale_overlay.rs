// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared stale-data overlay — a Warning tag showing when data last refreshed.

use bmc_wasm_protocol::{CrossAlign, ORANGE_40, RelTimeFormat, TagKind};

use crate::host::{SystemTime, ViewportShape};
use crate::props;
use crate::relative_time::relative_time_live;
use crate::tag::{TagIcon, tag};
use crate::tree::{Node, PropsData, StyleResult, TextStyle, row, spacer, text};

const LABEL_PX: u32 = 12; // Carbon label-01
const INSET: f32 = 8.0;
const ROUND_BOTTOM: f32 = 56.0; // lifted clear of the round face's bottom curve

fn label_style() -> TextStyle {
    TextStyle {
        size: LABEL_PX,
        color: ORANGE_40,
        ..TextStyle::default()
    }
}

fn stale_pill(anchor: SystemTime) -> Node {
    tag(
        TagKind::Warning,
        TagIcon::Default,
        row(
            props!(cross_align: CrossAlign::Center),
            [
                text(
                    "Last refresh ",
                    StyleResult(label_style(), PropsData::default()),
                ),
                relative_time_live(anchor, RelTimeFormat::Short, label_style()),
            ],
        ),
    )
}

fn stale_rect(anchor: SystemTime) -> Node {
    row(
        props!(inset_bottom: INSET, inset_left: INSET),
        [stale_pill(anchor)],
    )
}

fn stale_round(anchor: SystemTime) -> Node {
    row(
        props!(inset_bottom: ROUND_BOTTOM, inset_left: 0.0, inset_right: 0.0),
        [spacer(1.0), stale_pill(anchor), spacer(1.0)],
    )
}

/// Float a "Last refresh N ago" pill over `root`'s column — bottom-left on
/// rectangular faces, centered and lifted on round ones. Root must be a Column.
#[must_use]
pub fn with_stale_overlay(mut root: Node, anchor: SystemTime, shape: ViewportShape) -> Node {
    if let Node::Column(_, children) = &mut root {
        children.push(match shape {
            ViewportShape::Round => stale_round(anchor),
            ViewportShape::Rectangular => stale_rect(anchor),
        });
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::col;

    #[test]
    fn attaches_one_absolute_child_to_the_column() {
        let root = col(PropsData::default(), Vec::new());
        let out = with_stale_overlay(
            root,
            SystemTime { unix_secs: 0 },
            ViewportShape::Rectangular,
        );
        let Node::Column(_, children) = out else {
            panic!("BUG: a Column root stays a Column");
        };
        assert_eq!(children.len(), 1, "the overlay is appended as a child");
    }

    #[test]
    fn non_column_root_is_returned_untouched() {
        let out = with_stale_overlay(
            Node::Spacer { flex: 1.0 },
            SystemTime { unix_secs: 0 },
            ViewportShape::Round,
        );
        assert!(matches!(out, Node::Spacer { .. }), "no column, no overlay");
    }
}

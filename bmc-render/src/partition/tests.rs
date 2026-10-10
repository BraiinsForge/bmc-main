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

use super::{
    HashMap, HostPaintState, Range, ScrollState, has_static_content, host_layout_key,
    node_is_dynamic, node_self_is_dynamic, static_bytes_key, static_hash,
};
use crate::tree::{AnimationDef, Draw, Node, TransitionDef};
use bmc_ui::{
    AnimProperty, ArcCap, ArcFill, ArcSegments, Color, ColorSpace, Easing, LoopMode, PropsData,
    RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments, TagKind, TextStyle,
};
use bmc_ui::{ProgressMode, TagIcon};

fn leaf() -> Draw {
    Draw::Arc {
        cx: 20.0,
        cy: 30.0,
        radius: 40.0,
        start_angle: 0.0,
        end_angle: 1.0,
        width: 6.0,
        fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
        segments: ArcSegments::Continuous,
        cap: ArcCap::Round,
    }
}

fn animated(inner: Draw) -> Draw {
    Draw::Modified {
        animations: vec![AnimationDef {
            property: AnimProperty::Rotate,
            from: 0.0,
            to: 1.0,
            duration_ms: 1000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    }
}

fn transitioned(inner: Draw) -> Draw {
    Draw::Modified {
        animations: Vec::new(),
        transition: Some(TransitionDef {
            id_hash: 42,
            duration_ms: 500,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    }
}

fn canvas(draws: Vec<Draw>) -> Node {
    Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws,
    }
}

fn spacer() -> Node {
    Node::Spacer { flex: 1.0 }
}

fn progress_bar(track_h: f32, fraction: f32) -> Node {
    Node::ProgressBar {
        touch_key: None,
        track_h,
        mode: ProgressMode::Meter(fraction),
        active: true,
        fill_color: Color::from_rgb(1, 2, 3),
        track_color: Color::from_rgb(1, 2, 3),
        bg_color: Color::from_rgb(1, 2, 3),
        skin: None,
    }
}

fn rel_time_at(anchor: i64) -> Node {
    Node::RelTime {
        anchor,
        format: RelTimeFormat {
            length: RelTimeLength::Short,
            segments: RelTimeSegments::Single,
        },
        clamp: RelTimeClamp::default(),
        style: TextStyle::default(),
    }
}

/// A tag's pill paints into the layer while its `RelTime` label paints
/// dynamic, so the label's width decides how wide a cached pill has to be.
/// Miss the change and the text runs past its own background.
#[test]
fn a_rel_time_label_change_moves_the_layout_key() {
    let tag = Node::Tag {
        kind: TagKind::Info,
        icon: TagIcon::Hidden,
        content: Box::new(rel_time_at(0)),
    };

    let at_9 = host_layout_key(&tag, 9);
    let at_10 = host_layout_key(&tag, 10);
    assert_ne!(
        at_9, at_10,
        "9 s and 10 s render different labels, so the layer laid out for one cannot serve the other"
    );
}

/// The clock advances every frame. The layer only has to be dropped
/// once the label its layout was measured from actually changes.
#[test]
fn a_rel_time_holding_its_label_holds_the_layout_key() {
    let tag = Node::Tag {
        kind: TagKind::Info,
        icon: TagIcon::Hidden,
        content: Box::new(rel_time_at(0)),
    };

    let same = (0..4)
        .map(|_| host_layout_key(&tag, 90))
        .collect::<Vec<_>>();
    assert!(
        same.windows(2).all(|w| w[0] == w[1]),
        "the same instant must key the same, got {same:?}"
    );
    assert_eq!(
        host_layout_key(&tag, 90),
        host_layout_key(&tag, 91),
        "a coarser label spans both seconds, so nothing reflowed"
    );
}

/// A `RelTime` beside static content shifts it, not just its own tag.
#[test]
fn a_rel_time_sibling_moves_the_layout_key_of_its_row() {
    let row = Node::Row(
        PropsData::default(),
        vec![canvas(vec![leaf()]), rel_time_at(0), spacer()],
    );
    assert_ne!(host_layout_key(&row, 9), host_layout_key(&row, 10));
}

/// A progress bar's value moves its fill, never its box,
/// so folding it in would drop the layer for nothing.
#[test]
fn a_progress_value_leaves_the_layout_key_alone() {
    assert_eq!(
        host_layout_key(&progress_bar(4.0, 0.1), 0),
        host_layout_key(&progress_bar(4.0, 0.9), 0)
    );
}

/// The decoder keys none of a progress bar's bytes,
/// so a taller track would move its static siblings under an unchanged layer.
#[test]
fn a_progress_track_moves_the_layout_key() {
    assert_ne!(
        host_layout_key(&progress_bar(4.0, 0.5), 0),
        host_layout_key(&progress_bar(8.0, 0.5), 0)
    );
}

/// Taffy measures the label in its style,
/// so a larger font reflows it even when the text holds.
#[test]
fn a_rel_time_style_moves_the_layout_key() {
    let styled = |size: u32| Node::RelTime {
        anchor: 0,
        format: RelTimeFormat {
            length: RelTimeLength::Short,
            segments: RelTimeSegments::Single,
        },
        clamp: RelTimeClamp::default(),
        style: TextStyle {
            size,
            ..TextStyle::default()
        },
    };
    assert_ne!(
        host_layout_key(&styled(14), 0),
        host_layout_key(&styled(28), 0)
    );
}

/// A tree the clock cannot touch must key identically forever, or every
/// widget without a `RelTime` pays a recapture per frame.
#[test]
fn a_tree_without_host_time_keys_the_same_at_any_instant() {
    let tree = Node::Column(
        PropsData::default(),
        vec![canvas(vec![leaf(), animated(leaf())]), spacer()],
    );
    assert_eq!(host_layout_key(&tree, 0), host_layout_key(&tree, 10_000));
}

fn absolute_paragraph() -> Node {
    Node::Paragraph {
        props: PropsData {
            inset_top: 0.0,
            ..PropsData::default()
        },
        base_style: TextStyle::default(),
        spans: Vec::new(),
    }
}

fn bordered_row(children: Vec<Node>) -> Node {
    Node::Row(
        PropsData {
            border_width: 2.0,
            border_color: Color::from_rgb(9, 9, 9),
            ..PropsData::default()
        },
        children,
    )
}

/// A bordered container is static paint in its own right, so a tree whose
/// only static content is a border still earns a layer.
#[test]
fn a_border_alone_is_static_content() {
    assert!(has_static_content(&bordered_row(Vec::new())));
}

/// Where a static node lands is the walk's question, not this one's.
#[test]
fn an_absolute_static_node_over_a_dynamic_canvas_is_static_content() {
    let over = Node::Row(
        PropsData::default(),
        vec![canvas(vec![animated(leaf())]), absolute_paragraph()],
    );
    assert!(has_static_content(&over));
}

#[test]
fn a_canvas_whose_statics_all_sit_above_contributes_nothing_to_the_layer() {
    let node = canvas(vec![transitioned(leaf()), leaf()]);
    assert!(
        !has_static_content(&node),
        "capturing a layer for content the dynamic pass repaints buys nothing"
    );
}

// ── static_bytes_key / static_hash ──────────────────────────────

const DYNAMIC_MIDDLE: Range<usize> = 7..11;

#[test]
fn bytes_inside_a_dynamic_range_do_not_move_the_key() {
    assert_eq!(
        static_bytes_key(b"static-AAAA-static", &[DYNAMIC_MIDDLE]),
        static_bytes_key(b"static-BBBB-static", &[DYNAMIC_MIDDLE]),
    );
}

#[test]
fn bytes_outside_every_dynamic_range_move_the_key() {
    assert_ne!(
        static_bytes_key(b"static-AAAA-static", &[DYNAMIC_MIDDLE]),
        static_bytes_key(b"statiX-AAAA-static", &[DYNAMIC_MIDDLE]),
    );
}

const UNDERSCORES_AFTER_B: Range<usize> = 2..4;
const UNDERSCORES_BEFORE_B: Range<usize> = 1..3;

#[test]
fn static_bytes_moved_across_a_dynamic_range_move_the_key() {
    // The static bytes concatenate the same, but `b` moved across the dynamic range.
    assert_ne!(
        static_bytes_key(b"ab__c", &[UNDERSCORES_AFTER_B]),
        static_bytes_key(b"a__bc", &[UNDERSCORES_BEFORE_B]),
    );
}

const STATIC_BYTES: u64 = 0x5eed;

fn hash_idle() -> u64 {
    static_hash(
        STATIC_BYTES,
        &HostPaintState {
            pressed_key: None,
            scroll_offsets: &HashMap::new(),
        },
    )
}

#[test]
fn a_changed_static_half_invalidates_the_layer() {
    assert_ne!(
        hash_idle(),
        static_hash(
            STATIC_BYTES + 1,
            &HostPaintState {
                pressed_key: None,
                scroll_offsets: &HashMap::new(),
            },
        )
    );
}

#[test]
fn a_dim_classifies_as_its_child() {
    let animated = || canvas(vec![transitioned(leaf())]);
    let dimmed = crate::tree::dimmed(0.5, animated());
    assert_eq!(node_is_dynamic(&dimmed), node_is_dynamic(&animated()));
    assert_eq!(
        node_self_is_dynamic(&dimmed),
        node_self_is_dynamic(&animated())
    );
    assert_eq!(has_static_content(&dimmed), has_static_content(&animated()));
}

#[test]
fn pressing_an_element_invalidates_the_layer() {
    assert_ne!(
        hash_idle(),
        static_hash(
            STATIC_BYTES,
            &HostPaintState {
                pressed_key: Some("open_modal"),
                scroll_offsets: &HashMap::new(),
            },
        ),
        "an unpressed layer must not survive the press"
    );
}

#[test]
fn scrolling_invalidates_the_layer() {
    let scrolled = HashMap::from([(
        "list".to_owned(),
        ScrollState {
            scroll_offset: 40.0,
        },
    )]);
    assert_ne!(
        hash_idle(),
        static_hash(
            STATIC_BYTES,
            &HostPaintState {
                pressed_key: None,
                scroll_offsets: &scrolled,
            },
        ),
        "the offset moves static content without changing it"
    );
}

#[test]
fn scroll_offsets_hash_independently_of_map_order() {
    let one = HashMap::from([
        ("a".to_owned(), ScrollState { scroll_offset: 1.0 }),
        ("b".to_owned(), ScrollState { scroll_offset: 2.0 }),
    ]);
    let other = HashMap::from([
        ("b".to_owned(), ScrollState { scroll_offset: 2.0 }),
        ("a".to_owned(), ScrollState { scroll_offset: 1.0 }),
    ]);
    let hash_with = |scroll_offsets| {
        static_hash(
            STATIC_BYTES,
            &HostPaintState {
                pressed_key: None,
                scroll_offsets,
            },
        )
    };
    assert_eq!(hash_with(&one), hash_with(&other));
}

// ── has_static_content ──────────────────────────────────────────

#[test]
fn all_animated_canvas_has_no_static_content() {
    let node = Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![animated(leaf()), animated(leaf())],
    };
    assert!(!has_static_content(&node));
}

/// Behind the animation it would be [`Band::Above`] and not in the layer.
#[test]
fn one_static_draw_keeps_the_layer() {
    let node = Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![leaf(), animated(leaf())],
    };
    assert!(has_static_content(&node));
}

#[test]
fn container_background_counts_as_static_content() {
    let dynamic_child = Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![animated(leaf())],
    };
    let bare = Node::Column(PropsData::default(), vec![dynamic_child.clone()]);
    assert!(!has_static_content(&bare));

    let painted = Node::Column(
        PropsData {
            background: Color::from_rgb(10, 20, 30),
            ..PropsData::default()
        },
        vec![dynamic_child],
    );
    assert!(has_static_content(&painted));
}

#[test]
fn static_content_surfaces_through_nesting() {
    let leafy = Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![leaf()],
    };
    let nested = Node::Column(
        PropsData::default(),
        vec![Node::Row(PropsData::default(), vec![leafy])],
    );
    assert!(has_static_content(&nested));
}

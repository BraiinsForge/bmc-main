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

//! Static/dynamic classification of a widget tree.
//!
//! An animation-only frame replays the cached tree without running the guest,
//! so the static subtrees can be rasterised once and reused. A node is
//! **dynamic** when it — or any descendant — can change while the guest is
//! idle: an animated or transitioned draw, a host-driven time label, a
//! self-animating widget. Everything else is static, because every animatable
//! property is a draw-time transform applied after layout and so never
//! reflows.
//!
//! Classification is conservative — a node wrongly called dynamic costs a
//! redraw, one wrongly called static renders a stale frame.
//!
//! The decoder records the dynamic byte ranges while it reads,
//! so which kinds are dynamic is defined here, beside the types.

use crate::tree::{Draw, Node};

/// Whether `draw`, or anything it wraps, can change while the guest is idle.
#[must_use]
pub fn draw_is_dynamic(draw: &Draw) -> bool {
    match draw {
        // `Modified` wraps both `.animate*()` and `.transition()`, and one
        // carrying neither is inert, so test the payload, not the variant.
        Draw::Modified {
            animations,
            transition,
            inner,
            ..
        } => !animations.is_empty() || transition.is_some() || draw_is_dynamic(inner),
        Draw::Centered { inner }
        | Draw::Orbit { inner, .. }
        | Draw::Rotated { inner, .. }
        | Draw::Shadow { inner, .. } => draw_is_dynamic(inner),
        Draw::Rect { .. }
        | Draw::Circle { .. }
        | Draw::Arc { .. }
        | Draw::Svg { .. }
        | Draw::Bitmap { .. }
        | Draw::Path { .. }
        | Draw::Sphere { .. }
        | Draw::Mesh { .. }
        | Draw::Text { .. }
        | Draw::CurvedText { .. }
        | Draw::AutofitText { .. }
        | Draw::NinePatch { .. }
        | Draw::Qr { .. } => false,
    }
}

/// Where a draw sits in its canvas's paint order relative to the dynamic half.
///
/// The cached layer is composited at one point in the frame, so it can only
/// hold content that paints *before* everything dynamic. A static draw pushed
/// after a dynamic one has to keep painting after it — putting it in the layer
/// inverts the two and it vanishes under the dynamic content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Static and painted before anything dynamic: the cached layer.
    Below,
    /// Repainted every frame.
    Dynamic,
    /// Static, but ordered after dynamic content, so it repaints with the
    /// dynamic half to keep that order. A layer of its own would restore the
    /// caching, at a full-surface texture and a blit per widget.
    Above,
}

impl Band {
    /// Whether the cached layer holds this draw.
    #[must_use]
    pub fn is_in_layer(self) -> bool {
        matches!(self, Self::Below)
    }
}

/// The [`Band`] of each draw, in paint order.
///
/// One pass answering for the whole canvas, because [`Band::Above`] is a
/// property of a draw's position among its siblings rather than of the draw
/// itself. Every reader has to agree: a draw emitted with the dynamic half but
/// missing from the damage set is scissored away and disappears.
pub fn canvas_bands(draws: &[Draw]) -> impl Iterator<Item = Band> + '_ {
    let mut seen_dynamic = false;
    draws.iter().map(move |draw| {
        if draw_is_dynamic(draw) {
            seen_dynamic = true;
            Band::Dynamic
        } else if seen_dynamic {
            Band::Above
        } else {
            Band::Below
        }
    })
}

/// Whether `node`, or any descendant, can change while the guest is idle.
#[must_use]
pub fn node_is_dynamic(node: &Node) -> bool {
    match node {
        // `Scroll` is no more dynamic than what it holds: its offset only
        // moves under touch, and a touch already forces a full guest frame.
        Node::Column(_, children)
        | Node::Row(_, children)
        | Node::Center(_, children)
        | Node::Scroll { children, .. } => children.iter().any(node_is_dynamic),
        Node::Tag { content, .. } => node_is_dynamic(content),
        Node::Dimmed { child, .. } => node_is_dynamic(child),
        Node::Canvas { draws, .. } => draws.iter().any(draw_is_dynamic),
        // Host-driven: these advance without the guest running. Calling a
        // modal always-dynamic is cheap — a closed one draws nothing.
        Node::RelTime { .. } | Node::ProgressBar { .. } | Node::Modal { .. } => true,
        // Leaves the guest owns; a skeleton's bar has no progress of its own.
        Node::Paragraph { .. }
        | Node::Button { .. }
        | Node::Spacer { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_)
        | Node::Notification { .. } => false,
    }
}

/// Whether this node's **own paint** can change while the guest is idle.
///
/// Distinct from [`node_is_dynamic`], which asks about the whole subtree: a
/// container holding an animated child must be descended into, but its own
/// background belongs in the cached layer — emitting it in the dynamic pass
/// paints over everything the layer supplied.
///
/// Canvas draws are excluded here because they are gated individually.
#[must_use]
pub fn node_self_is_dynamic(node: &Node) -> bool {
    match node {
        // Its taffy node is the child's own.
        Node::Dimmed { child, .. } => node_self_is_dynamic(child),
        Node::RelTime { .. } | Node::ProgressBar { .. } | Node::Modal { .. } => true,
        Node::Column(..)
        | Node::Row(..)
        | Node::Center(..)
        | Node::Scroll { .. }
        | Node::Tag { .. }
        | Node::Canvas { .. }
        | Node::Paragraph { .. }
        | Node::Button { .. }
        | Node::Spacer { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_)
        | Node::Notification { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Band, canvas_bands, draw_is_dynamic, node_is_dynamic, node_self_is_dynamic};
    use crate::animation::{AnimProperty, ColorSpace, Easing, LoopMode};
    use crate::arc::{ArcCap, ArcFill, ArcSegments};
    use crate::colors::Color;
    use crate::nodes::TagKind;
    use crate::relative_time::{RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments};
    use crate::text::{PropsData, TextStyle};
    use crate::tree::{
        AnimationDef, Draw, Node, NotificationKind, ProgressMode, TagIcon, TransitionDef, dimmed,
    };

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

    fn inert_modified(inner: Draw) -> Draw {
        Draw::Modified {
            animations: Vec::new(),
            transition: None,
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

    fn rel_time() -> Node {
        Node::RelTime {
            anchor: 0,
            format: RelTimeFormat {
                length: RelTimeLength::Short,
                segments: RelTimeSegments::Single,
            },
            clamp: RelTimeClamp::default(),
            style: TextStyle::default(),
        }
    }

    fn progress(mode: ProgressMode) -> Node {
        Node::ProgressBar {
            touch_key: None,
            track_h: 4.0,
            mode,
            active: true,
            fill_color: Color::from_rgb(1, 2, 3),
            track_color: Color::from_rgb(1, 2, 3),
            bg_color: Color::from_rgb(1, 2, 3),
            skin: None,
        }
    }

    #[test]
    fn plain_draw_is_static() {
        assert!(!draw_is_dynamic(&leaf()));
    }

    #[test]
    fn animation_and_transition_each_make_a_draw_dynamic() {
        assert!(draw_is_dynamic(&animated(leaf())));
        assert!(draw_is_dynamic(&transitioned(leaf())));
    }

    #[test]
    fn modified_without_animation_or_transition_stays_static() {
        assert!(!draw_is_dynamic(&inert_modified(leaf())));
    }

    #[test]
    fn dynamic_draw_nested_under_wrappers_is_found() {
        let wrapped = Draw::Centered {
            inner: Box::new(Draw::Rotated {
                angle: 0.5,
                inner: Box::new(animated(leaf())),
            }),
        };
        assert!(draw_is_dynamic(&wrapped));
    }

    #[test]
    fn static_wrappers_stay_static() {
        let wrapped = Draw::Centered {
            inner: Box::new(Draw::Rotated {
                angle: 0.5,
                inner: Box::new(leaf()),
            }),
        };
        assert!(!draw_is_dynamic(&wrapped));
    }

    #[test]
    fn canvas_is_dynamic_only_when_one_of_its_draws_is() {
        assert!(!node_is_dynamic(&canvas(vec![leaf(), leaf()])));
        assert!(node_is_dynamic(&canvas(vec![leaf(), animated(leaf())])));
    }

    /// A clock: static dial, transitioned hands, static centre dot drawn last.
    #[test]
    fn clock_shaped_canvas_is_dynamic() {
        let clock = canvas(vec![
            leaf(),
            leaf(),
            transitioned(leaf()),
            transitioned(leaf()),
            transitioned(leaf()),
            leaf(),
        ]);
        assert!(node_is_dynamic(&clock));
    }

    #[test]
    fn container_is_dynamic_when_any_child_is() {
        let all_static = Node::Row(PropsData::default(), vec![spacer(), canvas(vec![leaf()])]);
        assert!(!node_is_dynamic(&all_static));

        let one_dynamic = Node::Row(
            PropsData::default(),
            vec![spacer(), canvas(vec![animated(leaf())])],
        );
        assert!(node_is_dynamic(&one_dynamic));
    }

    #[test]
    fn dynamic_child_propagates_through_nested_containers() {
        let deep = Node::Column(
            PropsData::default(),
            vec![Node::Center(
                PropsData::default(),
                vec![Node::Tag {
                    kind: TagKind::Info,
                    icon: TagIcon::Hidden,
                    content: Box::new(canvas(vec![animated(leaf())])),
                }],
            )],
        );
        assert!(node_is_dynamic(&deep));
    }

    #[test]
    fn container_with_animated_child_is_subtree_dynamic_but_paints_static() {
        let node = Node::Row(PropsData::default(), vec![canvas(vec![animated(leaf())])]);
        assert!(node_is_dynamic(&node), "subtree must be walked");
        assert!(
            !node_self_is_dynamic(&node),
            "its own background must come from the cached layer"
        );
    }

    #[test]
    fn host_driven_nodes_paint_themselves_dynamically() {
        assert!(node_self_is_dynamic(&progress(ProgressMode::Indeterminate)));
    }

    #[test]
    fn host_driven_nodes_are_always_dynamic() {
        assert!(node_is_dynamic(&rel_time()));
    }

    /// Both predicates answer for the host-driven nodes from their own arms,
    /// which only holds while they agree on that set.
    #[test]
    fn a_node_that_repaints_itself_is_dynamic_as_a_subtree() {
        let modal = Node::Modal {
            modal_id: "m".to_owned(),
            is_open: false,
            padding: 0,
            backdrop_alpha: 0,
            title: String::new(),
            content_height: 0.0,
            bg_color: Color::default(),
            header_color: Color::default(),
            title_color: Color::default(),
            max_width: 0,
            body: Vec::new(),
            footer_primary_key: String::new(),
            footer_primary_label: String::new(),
            footer_secondary_key: String::new(),
            footer_secondary_label: String::new(),
            footer_danger: false,
        };
        for node in [
            rel_time(),
            progress(ProgressMode::Indeterminate),
            modal,
            spacer(),
            canvas(vec![leaf()]),
            canvas(vec![animated(leaf())]),
        ] {
            assert!(
                !node_self_is_dynamic(&node) || node_is_dynamic(&node),
                "{node:?} repaints itself but is not dynamic as a subtree",
            );
        }
    }

    #[test]
    fn leaf_content_nodes_are_static() {
        assert!(!node_is_dynamic(&spacer()));
        assert!(!node_is_dynamic(&Node::Notification {
            kind: NotificationKind::Error,
            title: "t".to_owned(),
            subtitle: "s".to_owned(),
        }));
    }

    #[test]
    fn a_dim_classifies_as_its_child() {
        let animated = || canvas(vec![transitioned(leaf())]);
        let dim = dimmed(0.5, animated());
        assert_eq!(node_is_dynamic(&dim), node_is_dynamic(&animated()));
        assert_eq!(
            node_self_is_dynamic(&dim),
            node_self_is_dynamic(&animated())
        );
        let dim_time = dimmed(0.5, rel_time());
        assert!(node_self_is_dynamic(&dim_time));
    }

    // ── canvas_bands ────────────────────────────────────────────────

    fn bands(draws: &[Draw]) -> Vec<Band> {
        canvas_bands(draws).collect()
    }

    #[test]
    fn statics_before_the_first_dynamic_draw_go_in_the_layer() {
        assert_eq!(
            bands(&[leaf(), leaf(), animated(leaf())]),
            [Band::Below, Band::Below, Band::Dynamic]
        );
    }

    #[test]
    fn a_track_drawn_over_an_animated_globe_paints_with_the_dynamic_half() {
        // A transitioned sphere, then the ground track and marker on top of it.
        assert_eq!(
            bands(&[transitioned(leaf()), leaf(), leaf()]),
            [Band::Dynamic, Band::Above, Band::Above]
        );
    }

    #[test]
    fn a_centre_cap_between_two_hands_stays_above_both() {
        // Hour and minute hands, the cap over their pivot, then the second hand.
        assert_eq!(
            bands(&[transitioned(leaf()), leaf(), transitioned(leaf()), leaf()]),
            [Band::Dynamic, Band::Above, Band::Dynamic, Band::Above]
        );
    }

    #[test]
    fn a_canvas_with_nothing_dynamic_is_entirely_in_the_layer() {
        assert_eq!(bands(&[leaf(), leaf()]), [Band::Below, Band::Below]);
    }
}

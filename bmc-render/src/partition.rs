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

//! Static/dynamic partition of a widget tree.
//!
//! An animation-only frame replays the cached tree without running the guest
//! (the runtime's `render_cached_tree`), but still re-emits every draw even
//! though only a handful can have changed. Classifying subtrees is the
//! prerequisite for skipping the static ones and reusing their rasterised
//! output.
//!
//! A node is **dynamic** when it — or any descendant — can change while the
//! guest is not running: an animated or transitioned draw, a host-driven time
//! label, or a self-animating widget. Everything else is **static**: the same
//! tree and viewport produce the same pixels, because every animatable
//! property (`AnimProperty`: rotate, scale, alpha, translate, orbit, colour) is
//! a draw-time transform applied after layout and so never reflows.
//!
//! Classification is deliberately conservative — a node wrongly called dynamic
//! costs a redraw, one wrongly called static renders a stale frame.

use std::collections::hash_map::DefaultHasher;
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};
use std::mem;

use crate::tree::{DrawCommand, TreeNode};
use bmc_wasm_protocol::colors::Color;

/// Whether `draw`, or anything it wraps, can change while the guest is idle.
#[must_use]
pub fn draw_is_dynamic(draw: &DrawCommand) -> bool {
    match draw {
        // `Modified` is the wrapper the SDK emits for both `.animate*()` and
        // `.transition()`, and it carries the host-side state that advances on
        // cached frames. A `Modified` with neither payload is inert, so test
        // the payload rather than the variant.
        DrawCommand::Modified {
            animations,
            transition,
            inner,
            ..
        } => !animations.is_empty() || transition.is_some() || draw_is_dynamic(inner),
        DrawCommand::Centered { inner }
        | DrawCommand::Orbit { inner, .. }
        | DrawCommand::Rotated { inner, .. }
        | DrawCommand::Shadow { inner, .. } => draw_is_dynamic(inner),
        DrawCommand::Rect { .. }
        | DrawCommand::Circle { .. }
        | DrawCommand::Arc { .. }
        | DrawCommand::Svg { .. }
        | DrawCommand::Bitmap { .. }
        | DrawCommand::Path { .. }
        | DrawCommand::Sphere { .. }
        | DrawCommand::Mesh { .. }
        | DrawCommand::Text { .. }
        | DrawCommand::CurvedText { .. }
        | DrawCommand::AutofitText { .. }
        | DrawCommand::NinePatch { .. }
        | DrawCommand::Qr { .. } => false,
    }
}

/// Where a draw sits in its canvas's paint order relative to the dynamic half.
///
/// The cached layer is composited at one point in the frame, so it can only
/// hold content that paints *before* everything dynamic. Paint order inside a
/// canvas is the order the draws were pushed, and a static draw pushed after a
/// dynamic one has to keep painting after it — putting it in the layer inverts
/// the two. An analog clock's centre cap did exactly that, and the ISS widget's
/// ground track and marker vanished under the globe they are drawn on top of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Static and painted before anything dynamic: the cached layer.
    Below,
    /// Repainted every frame.
    Dynamic,
    /// Static, but ordered after dynamic content, so it repaints with the
    /// dynamic half to keep that order. A layer of its own would restore the
    /// caching, and costs a full-surface texture and a blit per widget to do it.
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
/// itself. Emission, damage collection, [`static_hash`] and
/// [`has_static_content`] all read it: they have to agree, or a draw emitted
/// with the dynamic half but missing from the damage set is scissored away and
/// disappears exactly as if it were still buried in the layer.
pub fn canvas_bands(draws: &[DrawCommand]) -> impl Iterator<Item = Band> + '_ {
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
pub fn node_is_dynamic(node: &TreeNode) -> bool {
    match node {
        // `Scroll` belongs here rather than with the always-dynamic nodes: its
        // offset only moves under touch, and a delivered touch already forces a
        // full WASM frame (`interaction_pending`), so it is no more dynamic
        // than what it holds.
        TreeNode::Column(_, children)
        | TreeNode::Row(_, children)
        | TreeNode::Center(_, children)
        | TreeNode::Scroll { children, .. } => children.iter().any(node_is_dynamic),
        TreeNode::Tag { content, .. } => node_is_dynamic(content),
        TreeNode::Canvas { draws, .. } => draws.iter().any(draw_is_dynamic),
        // Host-driven: these advance without the guest running. A relative-time
        // label re-formats on its own cadence, an indeterminate progress bar
        // animates continuously, and a modal animates open/close progress with
        // a backdrop over the whole surface. Calling a modal always-dynamic is
        // cheap — a closed one draws nothing.
        TreeNode::RelTime { .. } | TreeNode::ProgressBar { .. } | TreeNode::Modal { .. } => true,
        // `Switcher` and `Skeleton` are leaves the guest owns: a switcher's
        // active tab only moves when the guest sets it, and a skeleton paints a
        // plain bar with no host-driven progress of its own.
        TreeNode::Paragraph { .. }
        | TreeNode::Button { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Switcher { .. }
        | TreeNode::Skeleton(_)
        | TreeNode::Notification { .. } => false,
    }
}

/// Whether this node's **own paint** can change while the guest is idle.
///
/// Distinct from [`node_is_dynamic`], which asks about the whole subtree. A
/// container holding an animated child is dynamic as a *subtree* — it must be
/// descended into — but its own background is static and belongs in the cached
/// layer. Emitting it in the dynamic pass paints over everything the layer
/// supplied, which is exactly the bug this split exists to prevent.
///
/// Only host-driven nodes repaint themselves without the guest running. Canvas
/// draws are excluded here because they are gated individually, per draw.
#[must_use]
pub fn node_self_is_dynamic(node: &TreeNode) -> bool {
    match node {
        TreeNode::RelTime { .. } | TreeNode::ProgressBar { .. } | TreeNode::Modal { .. } => true,
        TreeNode::Column(..)
        | TreeNode::Row(..)
        | TreeNode::Center(..)
        | TreeNode::Scroll { .. }
        | TreeNode::Tag { .. }
        | TreeNode::Canvas { .. }
        | TreeNode::Paragraph { .. }
        | TreeNode::Button { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Switcher { .. }
        | TreeNode::Skeleton(_)
        | TreeNode::Notification { .. } => false,
    }
}

/// Hash the static half of a tree.
///
/// Lets a frame that refreshes the cached layer notice that nothing static
/// actually changed and blit the existing layer instead — the expensive part of
/// a guest frame is re-rasterising static content that is usually identical
/// from one guest run to the next.
///
/// Dynamic nodes and draws are skipped: their values change every frame by
/// definition and they are not in the layer. A container hashes only its own
/// props, then recurses, so a dynamic descendant cannot perturb it.
///
/// Static leaves go through their `Debug` output rather than field by field,
/// which costs about **5.6x** what hand-written field hashing would: 932 us
/// against 168 us for 2000 `Arc` draws, x86_64 release. Whole-tree figures on
/// the same machine are 56 us for 30 nodes and 50 draws, 508 us for 120 nodes
/// and 800 draws — it scales with draw count.
///
/// Kept anyway, on two counts. It runs on guest frames only, so a cached frame
/// pays none of it. And `DrawCommand` carries `f32`, which has no `Hash`, so
/// the alternative is a hand-written arm per variant spelling out `to_bits()`
/// per float — where a field added later escapes the hash silently and strands
/// a stale layer on screen, which shows as a visual artefact rather than a
/// failing test. If this becomes the target, derive the hash with a float
/// newtype rather than hand-rolling the arms: same saving, same safety.
#[must_use]
pub fn static_hash(node: &TreeNode) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_node(node, &mut hasher);
    hasher.finish()
}

/// Whether the static half of `node` would paint anything at all.
///
/// A fully dynamic widget has an empty static half, and capturing it produces a
/// layer of plain black that is then composited over every frame — on the Deck
/// that redundant full-screen blit measured 27 ms, a third of such a widget's
/// frame. Answering `false` lets the caller skip both the capture and the blit
/// and simply emit the whole tree, which for an empty static half is the same
/// picture for less work.
///
/// Errs toward `true`: a wrong `true` only keeps today's behaviour, and a wrong
/// `false` costs a re-rasterised static half but still draws it. Neither
/// changes what ends up on screen.
#[must_use]
pub fn has_static_content(node: &TreeNode) -> bool {
    // The arms below answer for every host-driven node directly, which is only
    // right while the two predicates agree on which nodes those are.
    debug_assert!(
        !node_self_is_dynamic(node) || node_is_dynamic(node),
        "a node that repaints itself while the guest is idle must be dynamic as a subtree",
    );
    match node {
        // Painted by the walk whenever the background is set; every other
        // container field draws nothing on its own.
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children) => {
            props.background != Color::default() || children.iter().any(has_static_content)
        }
        TreeNode::Scroll {
            props, children, ..
        } => props.background != Color::default() || children.iter().any(has_static_content),
        TreeNode::Canvas { props, draws, .. } => {
            props.background != Color::default() || canvas_bands(draws).any(Band::is_in_layer)
        }
        TreeNode::Tag { content, .. } => has_static_content(content),
        // Never in the layer, so they contribute nothing to it; a spacer
        // paints nothing at all.
        TreeNode::RelTime { .. }
        | TreeNode::ProgressBar { .. }
        | TreeNode::Modal { .. }
        | TreeNode::Spacer { .. } => false,
        // Paints unconditionally. A switcher always draws its pill and tabs,
        // and a skeleton always draws its placeholder bar.
        TreeNode::Paragraph { .. }
        | TreeNode::Button { .. }
        | TreeNode::Switcher { .. }
        | TreeNode::Skeleton(_)
        | TreeNode::Notification { .. } => true,
    }
}

/// Feeds `Debug` output into a hasher without allocating a `String` per node.
struct HashWriter<'a, H: Hasher>(&'a mut H);

impl<H: Hasher> fmt::Write for HashWriter<'_, H> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

fn hash_debug<H: Hasher>(value: &dyn fmt::Debug, hasher: &mut H) {
    // Writing into a hasher cannot fail, and `Debug` impls here do not error.
    let _ = write!(HashWriter(hasher), "{value:?}");
}

fn hash_node<H: Hasher>(node: &TreeNode, hasher: &mut H) {
    mem::discriminant(node).hash(hasher);
    match node {
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children) => {
            hash_debug(props, hasher);
            for child in children {
                hash_node(child, hasher);
            }
        }
        TreeNode::Scroll {
            scroll_key,
            props,
            children,
        } => {
            hash_debug(scroll_key, hasher);
            hash_debug(props, hasher);
            for child in children {
                hash_node(child, hasher);
            }
        }
        TreeNode::Tag {
            kind,
            icon,
            content,
        } => {
            hash_debug(kind, hasher);
            hash_debug(icon, hasher);
            hash_node(content, hasher);
        }
        TreeNode::Canvas {
            props,
            touch_key,
            draws,
        } => {
            hash_debug(props, hasher);
            hash_debug(touch_key, hasher);
            for (draw, _) in draws
                .iter()
                .zip(canvas_bands(draws))
                .filter(|(_, band)| band.is_in_layer())
            {
                hash_debug(draw, hasher);
            }
        }
        // Always dynamic: never part of the layer, so changes here cannot
        // invalidate it.
        TreeNode::RelTime { .. } | TreeNode::ProgressBar { .. } | TreeNode::Modal { .. } => {}
        TreeNode::Paragraph { .. }
        | TreeNode::Button { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Switcher { .. }
        | TreeNode::Skeleton(_)
        | TreeNode::Notification { .. } => hash_debug(node, hasher),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Band, canvas_bands, draw_is_dynamic, has_static_content, node_is_dynamic,
        node_self_is_dynamic, static_hash,
    };
    use crate::tree::{DrawCommand, HostAnimationDef, HostTransitionDef, TreeNode};
    use bmc_wasm_protocol::{
        AnimProperty, ArcCap, ArcFill, ArcSegments, Color, ColorSpace, Easing, LoopMode,
        ProgressKind, PropsData, RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments,
        TagKind, TextStyle,
    };

    /// Plain leaf draw — the shape used for every static fixture below.
    fn leaf() -> DrawCommand {
        DrawCommand::Arc {
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

    fn animated(inner: DrawCommand) -> DrawCommand {
        DrawCommand::Modified {
            animations: vec![HostAnimationDef {
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

    fn transitioned(inner: DrawCommand) -> DrawCommand {
        DrawCommand::Modified {
            animations: Vec::new(),
            transition: Some(HostTransitionDef {
                id_hash: 42,
                duration_ms: 500,
                easing: Easing::Linear,
            }),
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        }
    }

    /// A `Modified` carrying neither payload — the wrapper exists but nothing
    /// advances, so it must not force a redraw.
    fn inert_modified(inner: DrawCommand) -> DrawCommand {
        DrawCommand::Modified {
            animations: Vec::new(),
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(inner),
        }
    }

    fn canvas(draws: Vec<DrawCommand>) -> TreeNode {
        TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws,
        }
    }

    fn spacer() -> TreeNode {
        TreeNode::Spacer { flex: 1.0 }
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
        let wrapped = DrawCommand::Centered {
            inner: Box::new(DrawCommand::Rotated {
                angle: 0.5,
                inner: Box::new(animated(leaf())),
            }),
        };
        assert!(draw_is_dynamic(&wrapped));
    }

    #[test]
    fn static_wrappers_stay_static() {
        let wrapped = DrawCommand::Centered {
            inner: Box::new(DrawCommand::Rotated {
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

    /// The hello-widget clock shape: static dial, transitioned hands, then a
    /// static centre dot drawn last. The canvas as a whole is dynamic.
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
        let all_static = TreeNode::Row(PropsData::default(), vec![spacer(), canvas(vec![leaf()])]);
        assert!(!node_is_dynamic(&all_static));

        let one_dynamic = TreeNode::Row(
            PropsData::default(),
            vec![spacer(), canvas(vec![animated(leaf())])],
        );
        assert!(node_is_dynamic(&one_dynamic));
    }

    #[test]
    fn dynamic_child_propagates_through_nested_containers() {
        let deep = TreeNode::Column(
            PropsData::default(),
            vec![TreeNode::Center(
                PropsData::default(),
                vec![TreeNode::Tag {
                    kind: TagKind::Info,
                    icon: None,
                    content: Box::new(canvas(vec![animated(leaf())])),
                }],
            )],
        );
        assert!(node_is_dynamic(&deep));
    }

    /// The distinction that makes a cached static layer usable: a container
    /// holding an animated child must be descended into, but its own
    /// background belongs in the layer, not repainted over it.
    #[test]
    fn container_with_animated_child_is_subtree_dynamic_but_paints_static() {
        let node = TreeNode::Row(PropsData::default(), vec![canvas(vec![animated(leaf())])]);
        assert!(node_is_dynamic(&node), "subtree must be walked");
        assert!(
            !node_self_is_dynamic(&node),
            "its own background must come from the cached layer"
        );
    }

    #[test]
    fn host_driven_nodes_paint_themselves_dynamically() {
        let pb = TreeNode::ProgressBar {
            touch_key: None,
            track_h: 4.0,
            mode: ProgressKind::Indeterminate,
            fraction: 0.0,
            active: true,
            fill_color: Color::from_rgb(1, 2, 3),
            track_color: Color::from_rgb(1, 2, 3),
            bg_color: Color::from_rgb(1, 2, 3),
            skin: None,
        };
        assert!(node_self_is_dynamic(&pb));
    }

    #[test]
    fn host_driven_nodes_are_always_dynamic() {
        let rel_time = TreeNode::RelTime {
            anchor: 0,
            format: RelTimeFormat {
                length: RelTimeLength::Short,
                segments: RelTimeSegments::Single,
            },
            clamp: RelTimeClamp::default(),
            style: TextStyle::default(),
        };
        assert!(node_is_dynamic(&rel_time));
    }

    /// `node_self_is_dynamic` is the leaf half of `node_is_dynamic`, and
    /// `has_static_content` leans on that: it answers for the host-driven nodes
    /// from its own arms. A variant added to one predicate and not the other
    /// would quietly put a self-repainting node in the cached layer.
    #[test]
    fn a_node_that_repaints_itself_is_dynamic_as_a_subtree() {
        let rel_time = TreeNode::RelTime {
            anchor: 0,
            format: RelTimeFormat {
                length: RelTimeLength::Short,
                segments: RelTimeSegments::Single,
            },
            clamp: RelTimeClamp::default(),
            style: TextStyle::default(),
        };
        let progress = TreeNode::ProgressBar {
            touch_key: None,
            track_h: 4.0,
            mode: ProgressKind::Indeterminate,
            fraction: 0.0,
            active: true,
            fill_color: Color::from_rgb(1, 2, 3),
            track_color: Color::from_rgb(1, 2, 3),
            bg_color: Color::from_rgb(1, 2, 3),
            skin: None,
        };
        let modal = TreeNode::Modal {
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
            rel_time,
            progress,
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
        assert!(!node_is_dynamic(&TreeNode::Notification {
            kind: 0,
            title: "t".to_owned(),
            subtitle: "s".to_owned(),
        }));
    }
    // ── canvas_bands ────────────────────────────────────────────────

    fn bands(draws: &[DrawCommand]) -> Vec<Band> {
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
        // The ISS widget's canvas: a transitioned sphere, then the ground track
        // and marker that belong on top of it.
        assert_eq!(
            bands(&[transitioned(leaf()), leaf(), leaf()]),
            [Band::Dynamic, Band::Above, Band::Above]
        );
    }

    #[test]
    fn a_centre_cap_between_two_hands_stays_above_both() {
        // The analog clock's order: hour and minute hands, the cap that covers
        // their pivot, then the second hand and its own cap.
        assert_eq!(
            bands(&[transitioned(leaf()), leaf(), transitioned(leaf()), leaf()]),
            [Band::Dynamic, Band::Above, Band::Dynamic, Band::Above]
        );
    }

    #[test]
    fn a_canvas_with_nothing_dynamic_is_entirely_in_the_layer() {
        assert_eq!(bands(&[leaf(), leaf()]), [Band::Below, Band::Below]);
    }

    #[test]
    fn a_canvas_whose_statics_all_sit_above_contributes_nothing_to_the_layer() {
        let node = canvas(vec![transitioned(leaf()), leaf()]);
        assert!(
            !has_static_content(&node),
            "capturing a layer for content the dynamic pass repaints buys nothing"
        );
    }

    // ── static_hash ─────────────────────────────────────────────────

    /// `leaf()` with a distinguishable radius, for hashing two canvases that
    /// differ in exactly one static draw.
    fn leaf_with_radius(radius: f32) -> DrawCommand {
        DrawCommand::Arc {
            cx: 20.0,
            cy: 30.0,
            radius,
            start_angle: 0.0,
            end_angle: 1.0,
            width: 6.0,
            fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Round,
        }
    }

    #[test]
    fn changing_a_draw_in_the_layer_invalidates_it() {
        assert_ne!(
            static_hash(&canvas(vec![leaf_with_radius(1.0)])),
            static_hash(&canvas(vec![leaf_with_radius(2.0)]))
        );
    }

    #[test]
    fn changing_a_draw_above_the_dynamic_half_does_not() {
        // It is not in the layer, so re-capturing on its account would rewrite
        // an identical texture.
        assert_eq!(
            static_hash(&canvas(vec![transitioned(leaf()), leaf_with_radius(1.0)])),
            static_hash(&canvas(vec![transitioned(leaf()), leaf_with_radius(2.0)]))
        );
    }

    // ── has_static_content ──────────────────────────────────────────

    /// The case that motivated it: every draw animates, so the layer would hold
    /// nothing and blitting it is a wasted full-screen pass.
    #[test]
    fn all_animated_canvas_has_no_static_content() {
        let node = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![animated(leaf()), animated(leaf())],
        };
        assert!(!has_static_content(&node));
    }

    /// One unanimated draw ahead of the animation is enough to make the layer
    /// worth keeping. Behind it the draw is [`Band::Above`] and repaints with
    /// the dynamic half instead.
    #[test]
    fn one_static_draw_keeps_the_layer() {
        let node = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![leaf(), animated(leaf())],
        };
        assert!(has_static_content(&node));
    }

    /// A container's own background is painted by the walk, so it counts even
    /// when every child animates.
    #[test]
    fn container_background_counts_as_static_content() {
        let dynamic_child = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![animated(leaf())],
        };
        let bare = TreeNode::Column(PropsData::default(), vec![dynamic_child.clone()]);
        assert!(!has_static_content(&bare));

        let painted = TreeNode::Column(
            PropsData {
                background: Color::from_rgb(10, 20, 30),
                ..PropsData::default()
            },
            vec![dynamic_child],
        );
        assert!(has_static_content(&painted));
    }

    /// Nested static content has to surface through the containers above it.
    #[test]
    fn static_content_surfaces_through_nesting() {
        let leafy = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![leaf()],
        };
        let nested = TreeNode::Column(
            PropsData::default(),
            vec![TreeNode::Row(PropsData::default(), vec![leafy])],
        );
        assert!(has_static_content(&nested));
    }
}

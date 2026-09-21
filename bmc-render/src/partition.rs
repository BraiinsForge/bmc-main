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

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::{self, Write as _};
use std::hash::{BuildHasher, Hash, Hasher};
use std::ops::Range;

use crate::ScrollState;
use crate::tree::{DrawCommand, TreeNode};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{BitmapId, PropsData};

/// Whether `draw`, or anything it wraps, can change while the guest is idle.
#[must_use]
pub fn draw_is_dynamic(draw: &DrawCommand) -> bool {
    match draw {
        // `Modified` wraps both `.animate*()` and `.transition()`, and one
        // carrying neither is inert, so test the payload, not the variant.
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
pub fn canvas_bands(draws: &[DrawCommand]) -> impl Iterator<Item = Band> + '_ {
    canvas_bands_after(draws, false)
}

/// The [`Band`] of each draw, seeded with whether the canvas itself
/// already paints over the dynamic half.
///
/// Paint order runs through the whole tree, not one canvas at a time.
/// A canvas the walk banded [`Band::Above`] covers content it must keep
/// painting after, so every draw in it bands `Above` too,
/// including the ones before its own first dynamic draw.
pub fn canvas_bands_after(
    draws: &[DrawCommand],
    mut seen_dynamic: bool,
) -> impl Iterator<Item = Band> + '_ {
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
        // `Scroll` is no more dynamic than what it holds: its offset only
        // moves under touch, and a touch already forces a full guest frame.
        TreeNode::Column(_, children)
        | TreeNode::Row(_, children)
        | TreeNode::Center(_, children)
        | TreeNode::Scroll { children, .. } => children.iter().any(node_is_dynamic),
        TreeNode::Tag { content, .. } => node_is_dynamic(content),
        TreeNode::Dimmed { child, .. } => node_is_dynamic(child),
        TreeNode::Canvas { draws, .. } => draws.iter().any(draw_is_dynamic),
        // Host-driven: these advance without the guest running. Calling a
        // modal always-dynamic is cheap — a closed one draws nothing.
        TreeNode::RelTime { .. } | TreeNode::ProgressBar { .. } | TreeNode::Modal { .. } => true,
        // Leaves the guest owns; a skeleton's bar has no progress of its own.
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
/// Distinct from [`node_is_dynamic`], which asks about the whole subtree: a
/// container holding an animated child must be descended into, but its own
/// background belongs in the cached layer — emitting it in the dynamic pass
/// paints over everything the layer supplied.
///
/// Canvas draws are excluded here because they are gated individually.
#[must_use]
pub fn node_self_is_dynamic(node: &TreeNode) -> bool {
    match node {
        // Its taffy node is the child's own.
        TreeNode::Dimmed { child, .. } => node_self_is_dynamic(child),
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

/// Key the static half of a tree from the wire bytes it was decoded from.
///
/// `dynamic_ranges` are the bytes outside the layer, sorted and disjoint.
/// The decoder records them against the decoded tree:
/// every [`node_self_is_dynamic`] node past its type byte, and in each canvas its draw count
/// plus every draw from the first [`draw_is_dynamic`] one on, as [`canvas_bands`] splits it.
/// Keying the rest as raw bytes covers a wire field added later without anyone listing it.
///
/// Segments are length-prefixed,
/// so static bytes moved across a dynamic range move the key.
#[must_use]
pub fn static_bytes_key(data: &[u8], dynamic_ranges: &[Range<usize>]) -> u64 {
    let mut hasher = foldhash::fast::FixedState::with_seed(0).build_hasher();
    let mut start = 0;
    for range in dynamic_ranges {
        hash_segment(&data[start..range.start], &mut hasher);
        start = range.end;
    }
    hash_segment(&data[start..], &mut hasher);
    hasher.finish()
}

fn hash_segment<H: Hasher>(segment: &[u8], hasher: &mut H) {
    hasher.write_usize(segment.len());
    hasher.write(segment);
}

/// Key the static half of a tree for the host state it paints under.
///
/// Lets a frame that refreshes the cached layer notice that nothing static changed
/// and blit the existing layer instead.
/// `static_bytes` is the tree's [`static_bytes_key`], which covers paint, not layout:
/// a dynamic node still sizes its static siblings through the Taffy pass,
/// and [`host_layout_key`] is the half that sees that.
#[must_use]
pub fn static_hash(static_bytes: u64, host: &HostPaintState<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    static_bytes.hash(&mut hasher);
    hash_debug(&host.pressed_key, &mut hasher);
    // `HashMap` iteration order varies between runs, so fold with XOR rather
    // than hashing in sequence.
    let scroll = host.scroll_offsets.iter().fold(0, |acc, (key, state)| {
        let mut entry = DefaultHasher::new();
        hash_debug(key, &mut entry);
        hash_debug(state, &mut entry);
        acc ^ entry.finish()
    });
    hasher.finish() ^ scroll
}

/// Hash what a dynamic node contributes to *layout*, for the time `now_unix_secs`.
///
/// [`static_bytes_key`] deliberately skips dynamic nodes, which is right for paint
/// and wrong for layout: the Taffy pass still measures a dynamic node, so it
/// sizes the static siblings and ancestors that do reach the layer. A
/// [`TreeNode::RelTime`] label growing from "9 seconds" to "10 seconds" widens
/// the tag around it, and a layer holding the narrow pill shows the text
/// running past its own background.
///
/// The decoder keys none of a dynamic node's bytes, its sizing props included,
/// so those come in here too: a `RelTime`'s style and a `ProgressBar`'s track,
/// but not its value. A `Modal` lays out as `Display::None`.
///
/// Cheap on purpose — an animation-only replay recomputes this every frame,
/// where [`static_hash`] runs on guest frames only.
#[must_use]
pub fn host_layout_key(node: &TreeNode, now_unix_secs: i64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_layout_contribution(node, now_unix_secs, &mut hasher);
    hasher.finish()
}

fn hash_layout_contribution<H: Hasher>(node: &TreeNode, now_unix_secs: i64, hasher: &mut H) {
    match node {
        TreeNode::RelTime {
            anchor,
            format,
            clamp,
            style,
        } => {
            // Taffy measures the rendered text, so hash that rather than the
            // anchor: an unchanged label has moved nothing, whatever the clock
            // did.
            let label = crate::components::format_rel(now_unix_secs - *anchor, *format, *clamp);
            hash_debug(&label, hasher);
            hash_debug(style, hasher);
        }
        TreeNode::ProgressBar {
            track_h,
            mode,
            skin,
            ..
        } => {
            hash_debug(track_h, hasher);
            hash_debug(mode, hasher);
            hash_debug(skin, hasher);
        }
        TreeNode::Column(_, children)
        | TreeNode::Row(_, children)
        | TreeNode::Center(_, children)
        | TreeNode::Scroll { children, .. } => {
            for child in children {
                hash_layout_contribution(child, now_unix_secs, hasher);
            }
        }
        TreeNode::Tag { content, .. } => hash_layout_contribution(content, now_unix_secs, hasher),
        TreeNode::Dimmed { child, .. } => hash_layout_contribution(child, now_unix_secs, hasher),
        TreeNode::Modal { .. }
        | TreeNode::Canvas { .. }
        | TreeNode::Paragraph { .. }
        | TreeNode::Button { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Switcher { .. }
        | TreeNode::Skeleton(_)
        | TreeNode::Notification { .. } => {}
    }
}

/// The host-side state the static pass paints from, which the tree does not
/// carry.
///
/// Anything that alters how static content rasterises has to be part of
/// [`static_hash`]. A pressed button paints a darker variant and a scroll
/// offset shifts the content it wraps; left out, the layer keeps serving the
/// unpressed button until some unrelated tree change invalidates it.
#[derive(Debug)]
pub struct HostPaintState<'a> {
    /// The element currently held down, if any — `InteractionState::pressed_key`.
    pub pressed_key: Option<&'a str>,
    /// Scroll offsets by scroll key.
    pub scroll_offsets: &'a HashMap<String, ScrollState>,
}

/// Whether a box's own decoration paints anything.
///
/// The three sources a container draws its box from, in order of precedence:
/// a nine-patch background, a solid fill, a border.
/// A bordered box with no fill paints just as surely as a filled one,
/// so reading the fill alone would miss it.
#[must_use]
pub(crate) fn decoration_paints(
    nine_patch: Option<BitmapId>,
    background: Color,
    border_width: f32,
) -> bool {
    nine_patch.is_some() || background != Color::default() || border_width > 0.0
}

fn props_paint(props: &PropsData) -> bool {
    decoration_paints(props.bg_np_id, props.background, props.border_width)
}

/// Whether the static half of `node` would paint anything at all.
///
/// A fully dynamic widget has an empty static half, and capturing it produces a
/// layer of plain black composited over every frame. Answering `false` lets the
/// caller skip both the capture and the blit and emit the whole tree instead —
/// the same picture for less work.
///
/// Banding needs the laid-out boxes, so this answers only whether anything static paints.
/// A static node laid over the animation counts, though it never reaches the layer;
/// a capture left empty that way reports [`crate::tree::TreeResult::static_layer_empty`].
///
/// Errs toward `true`: a wrong `true` costs one empty capture, a wrong
/// `false` re-rasterises the static half. Neither changes what reaches screen.
#[must_use]
pub fn has_static_content(node: &TreeNode) -> bool {
    debug_assert!(
        !node_self_is_dynamic(node) || node_is_dynamic(node),
        "a node that repaints itself while the guest is idle must be dynamic as a subtree",
    );
    match node {
        // Only the box decoration paints; no other container field draws.
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children)
        | TreeNode::Scroll {
            props, children, ..
        } => props_paint(props) || children.iter().any(has_static_content),
        TreeNode::Canvas { props, draws, .. } => {
            props_paint(props) || canvas_bands(draws).any(Band::is_in_layer)
        }
        TreeNode::Tag { content, .. } => has_static_content(content),
        TreeNode::Dimmed { child, .. } => has_static_content(child),
        // Never in the layer; a spacer paints nothing at all.
        TreeNode::RelTime { .. }
        | TreeNode::ProgressBar { .. }
        | TreeNode::Modal { .. }
        | TreeNode::Spacer { .. } => false,
        // Always paints something.
        TreeNode::Paragraph { .. }
        | TreeNode::Button { .. }
        | TreeNode::Switcher { .. }
        | TreeNode::Skeleton(_)
        | TreeNode::Notification { .. } => true,
    }
}

/// Feeds `Debug` output into a hasher without allocating a `String` per value.
struct HashWriter<'a, H: Hasher>(&'a mut H);

impl<H: Hasher> fmt::Write for HashWriter<'_, H> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

fn hash_debug<H: Hasher>(value: &dyn fmt::Debug, hasher: &mut H) {
    // Writing into a hasher cannot fail.
    let _ = write!(HashWriter(hasher), "{value:?}");
}

#[cfg(test)]
mod tests {
    use super::{
        Band, HashMap, HostPaintState, Range, ScrollState, canvas_bands, canvas_bands_after,
        draw_is_dynamic, has_static_content, host_layout_key, node_is_dynamic,
        node_self_is_dynamic, static_bytes_key, static_hash,
    };
    use crate::tree::{DrawCommand, HostAnimationDef, HostTransitionDef, TreeNode};
    use bmc_wasm_protocol::{
        AnimProperty, ArcCap, ArcFill, ArcSegments, Color, ColorSpace, Easing, LoopMode,
        ProgressKind, PropsData, RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments,
        TagKind, TextStyle,
    };

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

    fn progress_bar(track_h: f32, fraction: f32) -> TreeNode {
        TreeNode::ProgressBar {
            touch_key: None,
            track_h,
            mode: ProgressKind::Meter,
            fraction,
            active: true,
            fill_color: Color::from_rgb(1, 2, 3),
            track_color: Color::from_rgb(1, 2, 3),
            bg_color: Color::from_rgb(1, 2, 3),
            skin: None,
        }
    }

    fn rel_time_at(anchor: i64) -> TreeNode {
        TreeNode::RelTime {
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
        let tag = TreeNode::Tag {
            kind: TagKind::Info,
            icon: None,
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
        let tag = TreeNode::Tag {
            kind: TagKind::Info,
            icon: None,
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
        let row = TreeNode::Row(
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
        let styled = |size: u32| TreeNode::RelTime {
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
        let tree = TreeNode::Column(
            PropsData::default(),
            vec![canvas(vec![leaf(), animated(leaf())]), spacer()],
        );
        assert_eq!(host_layout_key(&tree, 0), host_layout_key(&tree, 10_000));
    }

    fn absolute_paragraph() -> TreeNode {
        TreeNode::Paragraph {
            props: PropsData {
                inset_top: 0.0,
                ..PropsData::default()
            },
            base_style: TextStyle::default(),
            spans: Vec::new(),
        }
    }

    fn bordered_row(children: Vec<TreeNode>) -> TreeNode {
        TreeNode::Row(
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

    /// A canvas following dynamic content paints after it as a whole,
    /// so the draws before its own first dynamic one band `Above` too.
    #[test]
    fn a_canvas_after_dynamic_content_bands_every_draw_above() {
        let draws = vec![leaf(), animated(leaf()), leaf()];
        let bands: Vec<_> = canvas_bands_after(&draws, true).collect();
        assert_eq!(bands, vec![Band::Above, Band::Dynamic, Band::Above]);
    }

    /// Where a static node lands is the walk's question, not this one's.
    #[test]
    fn an_absolute_static_node_over_a_dynamic_canvas_is_static_content() {
        let over = TreeNode::Row(
            PropsData::default(),
            vec![canvas(vec![animated(leaf())]), absolute_paragraph()],
        );
        assert!(has_static_content(&over));
    }

    /// `has_static_content` answers for the host-driven nodes from its own
    /// arms, which holds only while the two predicates agree on that set.
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
        let node = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![animated(leaf()), animated(leaf())],
        };
        assert!(!has_static_content(&node));
    }

    /// Behind the animation it would be [`Band::Above`] and not in the layer.
    #[test]
    fn one_static_draw_keeps_the_layer() {
        let node = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![leaf(), animated(leaf())],
        };
        assert!(has_static_content(&node));
    }

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

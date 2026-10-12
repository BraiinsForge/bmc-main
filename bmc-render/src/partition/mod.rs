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
mod tests;

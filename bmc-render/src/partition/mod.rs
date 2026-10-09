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

//! Static/dynamic partition of a widget tree: the keys of its static half.
//!
//! Which nodes and draws are dynamic is [`bmc_ui::partition`]'s call,
//! re-exported here; this module hashes the static half
//! and decides whether a cached layer of it is worth having.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::{self, Write as _};
use std::hash::{BuildHasher, Hash, Hasher};
use std::ops::Range;

pub use bmc_ui::partition::{
    Band, canvas_bands, canvas_bands_after, draw_is_dynamic, node_is_dynamic, node_self_is_dynamic,
};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{BitmapId, PropsData};

use crate::ScrollState;
use crate::tree::TreeNode;

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
            hash_debug(&mode.kind(), hasher);
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
/// a capture left empty that way reports [`crate::tree::StaticLayer::Captured`] as `empty`.
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

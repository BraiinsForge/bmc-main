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

//! What the layout and paint walks share: the emission mode, the animation
//! context threaded through them, and what a walk reports back.

use crate::interaction::Rect;
use crate::partition::Band;
use crate::{AnimationState, TransitionState, TransitionStateKey};
use std::collections::HashMap;

/// Which half of the static/dynamic partition the walk should emit.
///
/// The walk always *traverses* the whole tree whatever the mode: animation and
/// transition state key on positional counters, so skipping traversal would
/// re-key every animation. Only emission is gated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EmitMode {
    /// Emit everything — full frames, and the fallback whenever no cached
    /// static layer is available.
    #[default]
    All,
    /// Emit only nodes tagged static, to build the cached layer.
    StaticOnly,
    /// Emit only nodes tagged dynamic, to composite over a cached layer.
    DynamicOnly,
}

impl EmitMode {
    /// Whether a node with this classification should draw in this mode.
    #[must_use]
    pub fn emits(self, node_is_dynamic: bool) -> bool {
        match self {
            Self::All => true,
            Self::StaticOnly => !node_is_dynamic,
            Self::DynamicOnly => node_is_dynamic,
        }
    }

    /// Whether a canvas draw in `band` should paint in this mode.
    #[must_use]
    pub fn emits_band(self, band: Band) -> bool {
        self.emits(!band.is_in_layer())
    }
}

/// How far two boxes must overlap before a pixel can tell, in logical pixels.
///
/// Layout lands on whole pixels, but a fractional scroll offset does not,
/// and boxes that merely touch can then overlap by a float step.
const OVERLAP_SLACK: f32 = 0.01;

/// The area the dynamic half has painted over so far, in paint order.
///
/// The cached layer composites in one go before the dynamic half.
/// A static node the walk reaches after dynamic content can stay in the layer
/// only while the two do not overlap; where they do,
/// the layer paints it *underneath* the content it should cover.
///
/// Hence a geometric test rather than position alone, which would band out
/// every node after the first animation — the ordinary column
/// whose label merely follows one included.
///
/// One rect per node painting outside the layer, kept apart:
/// [`Self::covers`] asks whether any of them overlaps,
/// and merging two into their bounding box would cover pixels neither paints.
///
/// Rects are clipped to the enclosing scroll viewport:
/// a node scrolled out of it paints nothing,
/// so it neither covers what follows nor is covered.
#[derive(Debug, Default)]
pub(crate) struct DynamicHalf {
    rects: Vec<Rect>,
    clip: Option<Rect>,
}

impl DynamicHalf {
    /// Whether anything painted so far shares pixels with `rect`.
    pub(super) fn covers(&self, rect: Rect) -> bool {
        let inset = Rect::new(
            rect.x + OVERLAP_SLACK,
            rect.y + OVERLAP_SLACK,
            rect.w - 2.0 * OVERLAP_SLACK,
            rect.h - 2.0 * OVERLAP_SLACK,
        );
        self.visible(inset)
            .is_some_and(|rect| self.rects.iter().any(|painted| painted.overlaps(rect)))
    }

    pub(super) fn add(&mut self, rect: Rect) {
        if let Some(rect) = self.visible(rect) {
            self.rects.push(rect);
        }
    }

    fn visible(&self, rect: Rect) -> Option<Rect> {
        self.clip.map_or(Some(rect), |clip| clip.intersection(rect))
    }

    /// Narrow the clip to `viewport`, returning the clip to restore after.
    pub(super) fn enter_viewport(&mut self, viewport: Rect) -> Option<Rect> {
        let outer = self.clip;
        // A viewport clipped away entirely leaves an empty clip, which nothing overlaps.
        self.clip = Some(
            self.visible(viewport)
                .unwrap_or(Rect::new(viewport.x, viewport.y, 0.0, 0.0)),
        );
        outer
    }

    pub(super) fn leave_viewport(&mut self, outer: Option<Rect>) {
        self.clip = outer;
    }

    pub(super) fn clear(&mut self) {
        self.rects.clear();
    }
}

/// Mutable animation context threaded through the render pipeline.
pub(crate) struct AnimationContext<'a> {
    /// Gates emission for static-layer caching; see [`EmitMode`].
    pub(crate) emit: EmitMode,
    pub(crate) animation_states: &'a mut HashMap<u64, AnimationState>,
    pub(crate) transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
    pub(crate) delta_ms: u32,
    pub(crate) frame_counter: u64,
    pub(crate) draw_counter: u32,
    pub(crate) canvas_index: u16,
    pub(crate) draw_in_canvas: u16,
    /// Monotonic counter for mesh atlas slot allocation (one per `draw_mesh` call).
    pub(crate) mesh_slot_counter: u8,
    /// What the walk has already painted into the dynamic half, in paint order.
    ///
    /// The running half of a node's [`Band`]; see [`DynamicHalf`].
    /// Both passes of a capture frame have to band identically,
    /// so this resets with the positional counters in `frame::render_tree`.
    pub(crate) dynamic_half: DynamicHalf,
    /// Whether the static pass has painted anything into the layer.
    pub(crate) layer_painted: bool,
    /// Set to true when any animation or transition is in progress.
    pub(crate) has_active: bool,
    /// Injected clock (unix seconds) for lazily-built subtrees.
    pub(crate) now_unix_secs: i64,
}

/// Touch interaction info (position relative to element, plus element dimensions).
#[derive(Debug, Clone, Copy)]
pub struct TouchHit {
    /// Local x position (relative to element left edge)
    pub x: f32,
    /// Local y position (relative to element top edge)
    pub y: f32,
    /// Element layout width
    pub width: f32,
    /// Element layout height
    pub height: f32,
}

/// Result from processing a tree
#[derive(Debug, Default)]
pub struct TreeResult {
    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub clicks: HashMap<String, TouchHit>,
    /// Active drag positions on interactive canvases (while finger is down)
    pub drags: HashMap<String, TouchHit>,
    /// Content bounding box (width, height) after layout. May be smaller than
    /// the viewport if the story content doesn't fill the available space.
    pub content_size: (f32, f32),
    /// Soonest ms until a time node's label changes; drives the boundary re-render.
    pub next_frame_delay_ms: Option<u32>,
    /// Union of the bounds of every node that paints dynamically, in logical
    /// pixels. This is the region a damage-tracked frame would have to repaint;
    /// everything outside it is unchanged from the previous frame.
    pub dynamic_bounds: Option<Rect>,
    /// The same regions un-merged, so a repaint can scissor to each rather than
    /// to a bounding box that may cover far more than the parts do.
    pub dynamic_rects: Vec<Rect>,
    /// How many nodes contributed to [`Self::dynamic_bounds`]. A union is only
    /// a good proxy for the repaint cost when the parts are close together.
    pub dynamic_node_count: usize,
    /// Whether the frame asked for the static layer and did not get it, so the
    /// caller must not treat the layer as holding this frame's static half.
    pub static_layer_missed: bool,
    /// Whether this frame captured a layer and painted nothing into it,
    /// every static node having banded above the dynamic half.
    /// Such a layer only costs a blit; dropping it never changes the picture,
    /// since a frame without one paints the whole tree.
    pub static_layer_empty: bool,
}

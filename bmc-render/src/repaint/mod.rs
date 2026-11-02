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

//! Repainting only what moved into a rotating pair of export buffers.
//!
//! A frame painted into a buffer that still holds an earlier frame can blit the
//! static layer back under the regions that moved and leave the rest alone.
//! [`DamageHistory`] keeps what that takes: the regions the last two walks moved,
//! and how many buffers no longer hold what the frame before them showed.

use std::cell::RefCell;

use crate::interaction::{CoalesceIterExt, Rect};
use crate::partition::static_layer::LayerCommit;
use crate::renderer::{FrameBackground, FrameClear, Renderer};
use crate::tree::StaticLayer;

/// Frames that must reach every buffer, rather than only the one they paint,
/// repeat this many times. The hosts own the real count
/// (`bmc_widget::egl::EXPORT_BUFFER_SLOTS`) and assert the two agree at
/// compile time; this crate cannot see it to check for itself.
pub const EXPORT_BUFFERS: usize = 2;

/// What the caller left in the target a frame is about to paint.
///
/// A damage-tracked frame repaints only the regions that moved,
/// which is correct only if the rest of the target is still there.
/// The walk cannot see the target — the caller owns it, clears it,
/// rotates it — so the caller has to say. Answering [`Self::Cleared`]
/// is always safe: the frame repaints everything and costs a full pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetContents {
    /// The caller left the previous frame in place (`FrameClear::Keep`).
    Preserved,
    /// The caller cleared the target, or handed over one whose contents are
    /// undefined — nothing outside this frame's own drawing survives.
    Cleared,
}

impl TargetContents {
    /// How the renderer starts a frame over a target holding this:
    /// clearing would wipe the regions a damage-tracked frame deliberately keeps.
    #[must_use]
    pub fn frame_clear(self) -> FrameClear {
        match self {
            Self::Preserved => FrameClear::Keep,
            Self::Cleared => FrameClear::Clear,
        }
    }

    /// Start `renderer`'s `width`×`height` frame over a target holding this,
    /// cleared to `background` unless the frame keeps it.
    pub fn begin_frame(
        self,
        renderer: &mut dyn Renderer,
        (width, height): (u32, u32),
        background: FrameBackground,
    ) {
        renderer.begin_frame_with_clear(width, height, 1.0, background, self.frame_clear());
    }
}

/// How a frame repainted its target.
#[derive(Debug, Clone)]
pub enum FrameRepaint {
    /// No tree was painted this frame;
    /// the target holds whatever was laid down before it.
    NoTree,
    /// The tree was painted over the whole target.
    Full,
    /// The static layer was blitted back only under `blitted`,
    /// and the dynamic draws painted over the rest an earlier frame left.
    Damage { blitted: Vec<Rect> },
}

impl FrameRepaint {
    /// How a walk handed `damage` repainted, from what it did with its static layer:
    /// only a blit under the damage leaves the rest of the target alone.
    /// Relies on `render_tree` blitting under the damage whenever there is any.
    #[must_use]
    pub fn of_walk(static_layer: StaticLayer, damage: Vec<Rect>) -> Self {
        match static_layer {
            StaticLayer::Blitted if !damage.is_empty() => Self::Damage { blitted: damage },
            StaticLayer::Unused
            | StaticLayer::Captured { .. }
            | StaticLayer::Blitted
            | StaticLayer::Missed => Self::Full,
        }
    }

    /// Pixels the frame repainted on a `width`×`height` target.
    #[must_use]
    pub fn area(&self, width: f32, height: f32) -> f32 {
        match self {
            Self::NoTree => 0.0,
            Self::Full => width * height,
            Self::Damage { blitted } => blitted.iter().map(Rect::area).sum(),
        }
    }
}

/// What a tree painted into rotating export buffers has to remember
/// to repaint only what moved.
#[derive(Debug, Default)]
pub struct DamageHistory {
    /// Dynamic regions from the last two walks, in logical pixels.
    ///
    /// Two, because the export buffers rotate: the buffer being drawn into was
    /// last painted two frames ago, so anything that moved in *either* of the
    /// intervening frames still has to be repainted or a stale pixel survives.
    recent: [Vec<Rect>; EXPORT_BUFFERS],

    /// How many upcoming frames must repaint in full because the export buffer
    /// they will paint does not hold what the frame before it showed.
    ///
    /// A frame paints one buffer, so changed static content reaches only that
    /// one while the next frame scissors to damage — and the two alternate on
    /// screen, flickering the change in and out at the display rate.
    stale_buffers: usize,

    /// [`Self::frame_damage`]'s last answer and the inputs it came from:
    /// a host asks twice a frame, and merging the rects costs more than comparing them.
    memo: RefCell<Option<DamageMemo>>,
}

/// A [`DamageHistory::frame_damage`] answer with the surface size, as bits,
/// and the two walks' rects it was worked out from.
#[derive(Debug)]
struct DamageMemo {
    size: (u32, u32),
    recent: [Vec<Rect>; EXPORT_BUFFERS],
    damage: Vec<Rect>,
}

impl DamageHistory {
    /// A history whose last two walks both moved `rects` and that owes no buffer a repaint,
    /// as a settled animation leaves it.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn settled(rects: &[Rect]) -> Self {
        Self {
            recent: [rects.to_vec(), rects.to_vec()],
            ..Self::default()
        }
    }

    /// Damage for the frame about to render, or empty to repaint everything.
    ///
    /// Empty while a buffer is owed a full repaint: a frame catching a buffer
    /// up on content it never received is painting exactly what the dynamic
    /// regions leave out.
    #[must_use]
    pub fn frame_damage(&self, width: f32, height: f32) -> Vec<Rect> {
        if self.stale_buffers > 0 {
            return Vec::new();
        }
        let size = (width.to_bits(), height.to_bits());
        let mut memo = self.memo.borrow_mut();
        if let Some(memo) = memo.as_ref()
            && memo.size == size
            && memo.recent == self.recent
        {
            return memo.damage.clone();
        }
        let damage = damage_rects(&self.recent, width, height);
        *memo = Some(DamageMemo {
            size,
            recent: self.recent.clone(),
            damage: damage.clone(),
        });
        damage
    }

    /// What the next frame may find in its target:
    /// kept only for a frame that `replays_over_layer`, the cached tree over its static layer,
    /// with damage to scissor to.
    #[must_use]
    pub fn target_for(&self, replays_over_layer: bool, width: f32, height: f32) -> TargetContents {
        if replays_over_layer && !self.frame_damage(width, height).is_empty() {
            TargetContents::Preserved
        } else {
            TargetContents::Cleared
        }
    }

    /// The damage a frame over `target` scissors to:
    /// none over a cleared one, which the frame repaints in full.
    #[must_use]
    pub fn damage_for(&self, target: TargetContents, width: f32, height: f32) -> Vec<Rect> {
        match target {
            TargetContents::Preserved => self.frame_damage(width, height),
            TargetContents::Cleared => Vec::new(),
        }
    }

    /// Account for a walk that painted a frame into the next buffer, handed `damage`,
    /// and say how it repainted it.
    ///
    /// A frame whose static half may differ from what the other buffers hold
    /// owes them a full repaint: one that drew the layer anew,
    /// or a `fresh` tree painted without one.
    pub fn walked(
        &mut self,
        committed: LayerCommit,
        fresh: bool,
        static_layer: StaticLayer,
        dynamic_rects: &[Rect],
        damage: Vec<Rect>,
    ) -> FrameRepaint {
        self.painted_in_full();
        self.recent.swap(0, 1);
        let newest = &mut self.recent[0];
        newest.clear();
        newest.extend_from_slice(dynamic_rects);
        let static_changed = match committed {
            LayerCommit::Reused | LayerCommit::Missed => false,
            LayerCommit::Rewritten => true,
            LayerCommit::Unlayered => fresh,
        };
        if static_changed {
            self.stale_buffers = EXPORT_BUFFERS - 1;
        }
        FrameRepaint::of_walk(static_layer, damage)
    }

    /// The regions the last walk moved.
    #[must_use]
    pub fn newest(&self) -> &[Rect] {
        &self.recent[0]
    }

    fn painted_in_full(&mut self) {
        self.stale_buffers = self.stale_buffers.saturating_sub(1);
    }

    /// Owe every buffer a full repaint, as a target whose buffers
    /// hold nothing this history describes leaves them.
    pub fn owe_every_buffer(&mut self) {
        self.stale_buffers = EXPORT_BUFFERS;
    }

    /// How many upcoming frames must repaint in full.
    #[must_use]
    pub fn owed_full_repaints(&self) -> usize {
        self.stale_buffers
    }
}

/// How much larger than the two rects apart a merged damage rect may be.
///
/// The blit paints each rect separately, so merging only pays while the union
/// covers no more than the pair does.
const DAMAGE_MERGE_TOLERANCE: f32 = 1.0;

/// Damage covering the last two frames, or empty to repaint everything.
///
/// Empty when the regions cover more of the surface than `BMC_DAMAGE_MAX_PCT`
/// (default 60): past that the bookkeeping and extra draw calls cost more than
/// the pixels they save.
fn damage_rects(recent: &[Vec<Rect>; EXPORT_BUFFERS], width: f32, height: f32) -> Vec<Rect> {
    let surface = width * height;
    if surface <= 0.0 || recent.iter().all(Vec::is_empty) {
        return Vec::new();
    }
    let rects = recent
        .iter()
        .flatten()
        .copied()
        .coalesce_within(DAMAGE_MERGE_TOLERANCE);
    // Rectangles that cross survive coalescing,
    // so their shared area counts twice, just as the per-rect blit paints it.
    let covered: f32 = rects.iter().map(|Rect { w, h, .. }| w * h).sum();
    if covered / surface > damage_max_fraction() {
        return Vec::new();
    }
    rects
}

/// Share of the surface above which damage tracking is abandoned.
fn damage_max_fraction() -> f32 {
    static MAX: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("BMC_DAMAGE_MAX_PCT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|pct| (0.0..=100.0).contains(pct))
            .unwrap_or(60.0)
            / 100.0
    })
}

#[cfg(test)]
mod tests;

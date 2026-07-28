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

use std::collections::HashMap;

use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::{Buffer as BufferCoord, Logical, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::{Anchor, Layer, LayerSurface, Margins};

use super::widget_tracker::LifecycleState;
use bmc::compositor::InstanceId;

/// Resolved client layer-surface state needed to place it.
pub struct LayerPlacement {
    pub size: Size<i32, Logical>,
    pub anchor: Anchor,
    pub margin: Margins,
}

/// Compute a layer surface's logical destination rectangle on an output of
/// `output` logical size. A zero size on an axis anchored to both opposite
/// edges stretches to fill that axis; otherwise the client's size is used.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "w/h/x/y are idiomatic for geometry"
)]
pub fn layer_geometry(p: &LayerPlacement, output: Size<i32, Logical>) -> Rectangle<i32, Logical> {
    let stretch_x = p.anchor.contains(Anchor::LEFT) && p.anchor.contains(Anchor::RIGHT);
    let stretch_y = p.anchor.contains(Anchor::TOP) && p.anchor.contains(Anchor::BOTTOM);

    let w = if p.size.w == 0 && stretch_x {
        output.w - p.margin.left - p.margin.right
    } else {
        p.size.w
    };
    let h = if p.size.h == 0 && stretch_y {
        output.h - p.margin.top - p.margin.bottom
    } else {
        p.size.h
    };

    #[expect(
        clippy::integer_division,
        reason = "intentional truncation for pixel centering"
    )]
    let x = if stretch_x {
        p.margin.left
    } else if p.anchor.contains(Anchor::RIGHT) {
        output.w - w - p.margin.right
    } else if p.anchor.contains(Anchor::LEFT) {
        p.margin.left
    } else {
        (output.w - w) / 2
    };
    #[expect(
        clippy::integer_division,
        reason = "intentional truncation for pixel centering"
    )]
    let y = if stretch_y {
        p.margin.top
    } else if p.anchor.contains(Anchor::BOTTOM) {
        output.h - h - p.margin.bottom
    } else if p.anchor.contains(Anchor::TOP) {
        p.margin.top
    } else {
        (output.h - h) / 2
    };

    Rectangle::from_loc_and_size((x, y), (w, h))
}

/// Stacking rank: higher draws on top.
#[must_use]
pub fn layer_rank(layer: Layer) -> u8 {
    match layer {
        Layer::Background => 0,
        Layer::Bottom => 1,
        Layer::Top => 2,
        Layer::Overlay => 3,
    }
}

/// Indices of `ranks` in paint order (bottom first). Stable within a rank, so
/// equal-rank surfaces keep registration order: later-registered paints last
/// (on top). Touch hit-testing iterates the reverse so the topmost wins.
#[must_use]
pub fn paint_order(ranks: &[u8]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..ranks.len()).collect();
    idx.sort_by_key(|&i| ranks[i]); // stable: preserves registration order within a rank
    idx
}

/// True if a mapped layer surface at `geo` covers the whole output and should
/// therefore suppress scene interaction (drag/cycling and tray preemption).
/// `Background` is excluded on purpose: those surfaces render above the scene
/// like every other layer, but are passive by contract (e.g. the offline
/// indicator) and must never block scene gestures — so even a fullscreen one is
/// not treated as a blocker.
#[must_use]
pub fn is_fullscreen_blocker(
    layer: Layer,
    geo: Rectangle<i32, Logical>,
    output: Size<i32, Logical>,
) -> bool {
    layer != Layer::Background
        && geo.loc.x <= 0
        && geo.loc.y <= 0
        && geo.size.w >= output.w
        && geo.size.h >= output.h
}

/// Demote every `Prepared` entry to `Dormant`.
pub fn suppress_prepared(states: &mut HashMap<InstanceId, LifecycleState>) {
    for state in states.values_mut() {
        if *state == LifecycleState::Prepared {
            *state = LifecycleState::Dormant;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerCommitEffects {
    pub geometry_changed: bool,
    pub layer_changed: bool,
    pub needs_damage: bool,
}

#[must_use]
pub fn layer_commit_effects(
    mapped: bool,
    old_layer: Layer,
    old_geometry: Option<Rectangle<i32, Logical>>,
    new_layer: Layer,
    new_geometry: Rectangle<i32, Logical>,
) -> LayerCommitEffects {
    let geometry_changed = mapped && old_geometry != Some(new_geometry);
    let layer_changed = mapped && old_layer != new_layer;
    LayerCommitEffects {
        geometry_changed,
        layer_changed,
        needs_damage: geometry_changed || layer_changed,
    }
}

/// True when a committed buffer's size disagrees with the geometry the
/// compositor configured for the surface. See
/// [`LayerEntry::warn_on_buffer_mismatch`] for what that disagreement means and
/// why it is worth a warn. An indeterminable buffer size (`None`) is not a
/// mismatch.
///
/// `buffer_size` is in buffer pixels and `geometry` is logical, so the
/// comparison happens in buffer pixels: a `wl_surface.set_buffer_scale(2)`
/// client attaching a double-size buffer has a surface size that matches
/// exactly, and must not warn. Scaling geometry up rather than the buffer down
/// keeps the check exact — a buffer that is not a whole multiple of the scale
/// stays a mismatch instead of truncating into agreement.
///
/// `wl_surface.set_buffer_transform` is deliberately not folded in: the
/// renderer ignores it, so a rotated buffer really does render wrong and should
/// keep warning.
#[must_use]
pub fn layer_buffer_mismatch(
    buffer_size: Option<Size<i32, BufferCoord>>,
    geometry: Rectangle<i32, Logical>,
    buffer_scale: i32,
) -> bool {
    buffer_size.is_some_and(|s| {
        (s.w, s.h)
            != (
                geometry.size.w.saturating_mul(buffer_scale),
                geometry.size.h.saturating_mul(buffer_scale),
            )
    })
}

/// Gate the mismatch warning to once per episode: fire only on the
/// transition into mismatch, stay silent while it persists, and re-arm when
/// a matching buffer arrives. Returns `(warn_now, warned_state_after)`.
#[must_use]
pub fn mismatch_warn_transition(already_warned: bool, mismatch: bool) -> (bool, bool) {
    (mismatch && !already_warned, mismatch)
}

/// Swap a tracked buffer for a new one (or `None` to clear), returning the
/// previous buffer and its id so the caller can release the buffer and
/// invalidate the texture. Pure: the real types are filled in by the caller.
#[must_use]
pub fn replace_buffer<B, I>(
    cur_buf: &mut Option<B>,
    cur_id: &mut Option<I>,
    new: Option<(B, I)>,
) -> (Option<B>, Option<I>) {
    let old_buf = cur_buf.take();
    let old_id = cur_id.take();
    if let Some((b, i)) = new {
        *cur_buf = Some(b);
        *cur_id = Some(i);
    }
    (old_buf, old_id)
}

/// One tracked layer-shell surface and its current buffer state.
pub struct LayerEntry {
    pub surface: LayerSurface,
    pub layer: Layer,
    /// Currently-committed buffer, or `None` when unmapped (NULL buffer).
    pub buffer: Option<WlBuffer>,
    /// ObjectId of the committed buffer, retained so an unmap (which carries
    /// no buffer object) can still evict the matching texture-cache entry.
    pub buffer_id: Option<ObjectId>,
    /// Last computed logical geometry, used to damage the vacated region on hide.
    /// Invariant: set whenever `buffer` is set; both are updated together in the
    /// NewBuffer commit path.
    pub last_geometry: Option<Rectangle<i32, Logical>>,
    /// True after warning about a mismatched buffer commit; cleared by a
    /// matching commit, so the warn fires once per mismatch episode instead
    /// of once per commit (see [`mismatch_warn_transition`]).
    pub buffer_mismatch_warned: bool,
}

impl LayerEntry {
    pub fn new(surface: LayerSurface, layer: Layer) -> Self {
        Self {
            surface,
            layer,
            buffer: None,
            buffer_id: None,
            last_geometry: None,
            buffer_mismatch_warned: false,
        }
    }

    pub fn is_mapped(&self) -> bool {
        self.buffer.is_some()
    }

    /// Warn when a layer client commits a buffer that disagrees with its
    /// configured geometry. This asserts a local invariant, not protocol
    /// conformance: the layer-shell configure size is a hint the client may
    /// legally ignore, and a standard compositor would draw the surface at its
    /// own size and centre it in the configured box. We draw at the configure
    /// size instead, so `geometry` stays the one source of truth for the blit,
    /// the touch hit-box, the hide-damage rect and [`is_fullscreen_blocker`] —
    /// at the cost that a disagreement shows as blur rather than as a misplaced
    /// surface, leaving this warn as its only symptom. Every in-tree layer
    /// client commits at its configure size. Fires once per mismatch episode
    /// (see [`mismatch_warn_transition`]); a matching commit re-arms it.
    pub fn warn_on_buffer_mismatch(
        &mut self,
        buffer: &WlBuffer,
        geometry: Rectangle<i32, Logical>,
        buffer_scale: i32,
    ) {
        let buffer_size = smithay::backend::renderer::buffer_dimensions(buffer);
        let mismatch = layer_buffer_mismatch(buffer_size, geometry, buffer_scale);
        let (warn_now, warned) = mismatch_warn_transition(self.buffer_mismatch_warned, mismatch);
        self.buffer_mismatch_warned = warned;
        if warn_now {
            tracing::warn!(
                ?buffer_size,
                ?geometry,
                buffer_scale,
                "layer surface committed a buffer whose surface size does not match \
                 its configured geometry; it is stretched into the geometry and will \
                 show blurred - size the buffer from the configure event"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(size: (i32, i32), anchor: Anchor) -> LayerPlacement {
        LayerPlacement {
            size: size.into(),
            anchor,
            margin: Margins::default(),
        }
    }

    #[test]
    fn matching_buffer_is_not_a_mismatch() {
        let geometry = Rectangle::from_loc_and_size((10, 20), (420, 180));
        assert!(!layer_buffer_mismatch(
            Some(Size::from((420, 180))),
            geometry,
            1
        ));
    }

    #[test]
    fn wrong_size_buffer_is_a_mismatch() {
        let geometry = Rectangle::from_loc_and_size((10, 20), (420, 180));
        assert!(layer_buffer_mismatch(
            Some(Size::from((420, 179))),
            geometry,
            1
        ));
    }

    #[test]
    fn indeterminable_buffer_size_is_not_a_mismatch() {
        let geometry = Rectangle::from_loc_and_size((10, 20), (420, 180));
        assert!(!layer_buffer_mismatch(None, geometry, 1));
    }

    #[test]
    fn scaled_buffer_matching_its_scale_is_not_a_mismatch() {
        // A scale-2 client attaching a double-size buffer is protocol-correct.
        let geometry = Rectangle::from_loc_and_size((10, 20), (420, 180));
        assert!(!layer_buffer_mismatch(
            Some(Size::from((840, 360))),
            geometry,
            2
        ));
    }

    #[test]
    fn unscaled_buffer_at_scale_two_is_a_mismatch() {
        let geometry = Rectangle::from_loc_and_size((10, 20), (420, 180));
        assert!(layer_buffer_mismatch(
            Some(Size::from((420, 180))),
            geometry,
            2
        ));
    }

    #[test]
    fn buffer_off_by_one_from_its_scale_is_a_mismatch() {
        // Scaling geometry up instead of the buffer down keeps this exact:
        // 841 / 2 would truncate to 420 and hide the disagreement.
        let geometry = Rectangle::from_loc_and_size((10, 20), (420, 180));
        assert!(layer_buffer_mismatch(
            Some(Size::from((841, 360))),
            geometry,
            2
        ));
    }

    #[test]
    fn mismatch_warns_once_per_episode_and_rearms_on_match() {
        // First mismatched commit warns; the episode then stays silent,
        // a matching commit re-arms, and a new episode warns again.
        assert_eq!(mismatch_warn_transition(false, true), (true, true));
        assert_eq!(mismatch_warn_transition(true, true), (false, true));
        assert_eq!(mismatch_warn_transition(true, false), (false, false));
        assert_eq!(mismatch_warn_transition(false, true), (true, true));
    }

    #[test]
    fn matching_commits_never_warn() {
        assert_eq!(mismatch_warn_transition(false, false), (false, false));
    }

    #[test]
    fn fullscreen_stretches_all_edges() {
        let p = placement(
            (0, 0),
            Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        );
        assert_eq!(
            layer_geometry(&p, (1280, 480).into()),
            Rectangle::from_loc_and_size((0, 0), (1280, 480))
        );
    }

    #[test]
    fn bottom_right_corner_uses_client_size() {
        let p = placement((120, 40), Anchor::BOTTOM | Anchor::RIGHT);
        assert_eq!(
            layer_geometry(&p, (1280, 480).into()),
            Rectangle::from_loc_and_size((1160, 440), (120, 40))
        );
    }

    #[test]
    fn top_full_width_panel() {
        let p = placement((0, 200), Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        assert_eq!(
            layer_geometry(&p, (1280, 480).into()),
            Rectangle::from_loc_and_size((0, 0), (1280, 200))
        );
    }

    #[test]
    fn unanchored_centers() {
        let p = placement((400, 100), Anchor::empty());
        assert_eq!(
            layer_geometry(&p, (1280, 480).into()),
            Rectangle::from_loc_and_size((440, 190), (400, 100))
        );
    }

    #[test]
    fn paint_order_is_stable_within_rank() {
        // ranks: Overlay(3), Top(2), Overlay(3) registered at indices 0,1,2.
        // Paint order: Top first, then the two Overlays in registration order.
        assert_eq!(paint_order(&[3, 2, 3]), vec![1, 0, 2]);
    }

    #[test]
    fn replace_buffer_new_returns_previous() {
        let mut buf = Some(10_u32);
        let mut id = Some(100_u32);
        let (old_buf, old_id) = replace_buffer(&mut buf, &mut id, Some((11, 101)));
        assert_eq!((old_buf, old_id), (Some(10), Some(100)));
        assert_eq!((buf, id), (Some(11), Some(101)));
    }

    #[test]
    fn replace_buffer_remove_clears_and_returns_previous() {
        let mut buf = Some(11_u32);
        let mut id = Some(101_u32);
        let (old_buf, old_id) = replace_buffer(&mut buf, &mut id, None);
        assert_eq!((old_buf, old_id), (Some(11), Some(101)));
        assert_eq!((buf, id), (None, None));
    }

    #[test]
    fn replace_buffer_remove_when_empty_is_noop() {
        let mut buf: Option<u32> = None;
        let mut id: Option<u32> = None;
        let (old_buf, old_id) = replace_buffer(&mut buf, &mut id, None);
        assert_eq!((old_buf, old_id), (None, None));
    }

    #[test]
    fn fullscreen_blocker_true_for_full_cover_above_background() {
        let output = Size::from((1280, 480));
        let geo = Rectangle::from_loc_and_size((0, 0), (1280, 480));
        assert!(is_fullscreen_blocker(Layer::Overlay, geo, output));
        assert!(is_fullscreen_blocker(Layer::Top, geo, output));
        assert!(is_fullscreen_blocker(Layer::Bottom, geo, output));
    }

    #[test]
    fn fullscreen_blocker_false_for_background_layer() {
        let output = Size::from((1280, 480));
        let geo = Rectangle::from_loc_and_size((0, 0), (1280, 480));
        assert!(!is_fullscreen_blocker(Layer::Background, geo, output));
    }

    #[test]
    fn fullscreen_blocker_false_for_corner_surface() {
        let output = Size::from((1280, 480));
        let geo = Rectangle::from_loc_and_size((1160, 440), (120, 40));
        assert!(!is_fullscreen_blocker(Layer::Overlay, geo, output));
    }

    #[test]
    fn suppress_prepared_demotes_only_prepared() {
        let mut states: HashMap<InstanceId, LifecycleState> = [
            ("a".to_owned(), LifecycleState::Prepared),
            ("b".to_owned(), LifecycleState::Visible),
            ("c".to_owned(), LifecycleState::Entering),
            ("d".to_owned(), LifecycleState::Leaving),
            ("e".to_owned(), LifecycleState::Dormant),
        ]
        .into_iter()
        .collect();

        suppress_prepared(&mut states);

        assert_eq!(states["a"], LifecycleState::Dormant);
        assert_eq!(states["b"], LifecycleState::Visible);
        assert_eq!(states["c"], LifecycleState::Entering);
        assert_eq!(states["d"], LifecycleState::Leaving);
        assert_eq!(states["e"], LifecycleState::Dormant);
    }

    #[test]
    fn mapped_layer_commit_effects_damage_geometry_change_without_buffer_change() {
        let old = Rectangle::from_loc_and_size((0, 0), (100, 100));
        let new = Rectangle::from_loc_and_size((10, 20), (100, 100));

        let effects = layer_commit_effects(true, Layer::Top, Some(old), Layer::Top, new);

        assert_eq!(
            effects,
            LayerCommitEffects {
                geometry_changed: true,
                layer_changed: false,
                needs_damage: true,
            },
        );
    }

    #[test]
    fn mapped_layer_commit_effects_damage_layer_change_without_buffer_change() {
        let geometry = Rectangle::from_loc_and_size((0, 0), (100, 100));

        let effects =
            layer_commit_effects(true, Layer::Top, Some(geometry), Layer::Overlay, geometry);

        assert_eq!(
            effects,
            LayerCommitEffects {
                geometry_changed: false,
                layer_changed: true,
                needs_damage: true,
            },
        );
    }

    #[test]
    fn unmapped_layer_commit_effects_do_not_damage_metadata_changes() {
        let old = Rectangle::from_loc_and_size((0, 0), (100, 100));
        let new = Rectangle::from_loc_and_size((10, 20), (100, 100));

        let effects = layer_commit_effects(false, Layer::Top, Some(old), Layer::Overlay, new);

        assert_eq!(
            effects,
            LayerCommitEffects {
                geometry_changed: false,
                layer_changed: false,
                needs_damage: false,
            },
        );
    }
}

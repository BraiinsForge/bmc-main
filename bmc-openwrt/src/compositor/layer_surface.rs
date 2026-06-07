// Copyright (C) 2026  Braiins Systems s.r.o.

// items wired up in subsequent tasks (layer-shell global, commit handling, compositor)
#![expect(dead_code)]

use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::{Logical, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::{Anchor, Layer, LayerSurface, Margins};

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
    pub last_geometry: Option<Rectangle<i32, Logical>>,
}

impl LayerEntry {
    pub fn new(surface: LayerSurface, layer: Layer) -> Self {
        Self {
            surface,
            layer,
            buffer: None,
            buffer_id: None,
            last_geometry: None,
        }
    }

    pub fn is_mapped(&self) -> bool {
        self.buffer.is_some()
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

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

//! The damage a widget frame commits, from what its walk repainted.

use bmc_render::interaction::Rect;
use bmc_widget::surface::DamageRegion;

/// `changed`, the runtime's committed damage, as regions of a `width`×`height` export buffer,
/// or `None` when the whole buffer has to count as changed.
///
/// Its rects run from the top in widget pixels,
/// while the buffer's rows run bottom-up, as GL writes them, so y flips.
#[must_use]
pub fn repainted_regions(
    changed: Option<&[Rect]>,
    width: u32,
    height: u32,
) -> Option<Vec<DamageRegion>> {
    let regions: Vec<_> = changed?
        .iter()
        .filter_map(|rect| buffer_region(rect, width, height))
        .collect();
    (!regions.is_empty()).then_some(regions)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "export buffer heights are far below where f32 loses whole pixels"
)]
fn buffer_region(rect: &Rect, width: u32, height: u32) -> Option<DamageRegion> {
    let dim = |v: u32| i32::try_from(v).expect("BUG: export buffer dimensions fit i32");
    let (x, y, width, height) = rect
        .bottom_up(height as f32)
        .pixel_box(dim(width), dim(height));
    (width > 0 && height > 0).then_some(DamageRegion {
        x,
        y,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::repainted_regions;
    use bmc_render::interaction::Rect;
    use bmc_widget::surface::DamageRegion;

    /// The regions of a 100×50 buffer where the runtime committed `changed`.
    fn regions(changed: &[Rect]) -> Option<Vec<DamageRegion>> {
        repainted_regions(Some(changed), 100, 50)
    }

    #[test]
    fn no_committed_damage_damages_the_whole_buffer() {
        assert_eq!(repainted_regions(None, 100, 50), None);
    }

    #[test]
    fn a_region_near_the_top_lands_near_the_bottom_rows() {
        assert_eq!(
            regions(&[Rect::new(10.0, 0.0, 20.0, 5.0)]),
            Some(vec![DamageRegion {
                x: 10,
                y: 45,
                width: 20,
                height: 5,
            }]),
            "rows run bottom-up in the buffer"
        );
    }

    #[test]
    fn fractional_edges_round_outwards() {
        assert_eq!(
            regions(&[Rect::new(1.5, 2.5, 3.0, 3.0)]),
            Some(vec![DamageRegion {
                x: 1,
                y: 44,
                width: 4,
                height: 4,
            }])
        );
    }

    #[test]
    fn a_region_past_the_edge_is_clipped_to_the_buffer() {
        assert_eq!(
            regions(&[Rect::new(90.0, -5.0, 20.0, 10.0)]),
            Some(vec![DamageRegion {
                x: 90,
                y: 45,
                width: 10,
                height: 5,
            }])
        );
    }

    #[test]
    fn a_repaint_entirely_off_the_buffer_damages_all_of_it() {
        assert_eq!(
            regions(&[Rect::new(200.0, 0.0, 10.0, 10.0)]),
            None,
            "no region would leave the compositor nothing to redraw for a new buffer"
        );
    }
}

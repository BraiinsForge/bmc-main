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

use bmc_wasm_protocol::{ArcAnchor, ArcTextFacing};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphPlacement {
    pub theta: f32,
    pub rotation: f32,
}

#[must_use]
pub fn arc_glyph_layout(
    widths: &[f32],
    radius: f32,
    angle: f32,
    anchor: ArcAnchor,
    facing: ArcTextFacing,
) -> Vec<GlyphPlacement> {
    if radius <= 0.0 || widths.is_empty() {
        return Vec::new();
    }

    let total_advance = widths.iter().sum::<f32>();
    let anchor_offset = match anchor {
        ArcAnchor::Start => 0.0,
        ArcAnchor::Center => -total_advance / 2.0,
        ArcAnchor::End => -total_advance,
    };
    let direction = match facing {
        ArcTextFacing::Outward => 1.0,
        ArcTextFacing::Inward => -1.0,
    };
    let rotation_bias = match facing {
        ArcTextFacing::Outward => 0.0,
        ArcTextFacing::Inward => std::f32::consts::PI,
    };

    let mut leading_edge = 0.0;
    let mut placements = Vec::with_capacity(widths.len());
    for width in widths {
        let glyph_center = leading_edge + width / 2.0;
        let arc_offset = glyph_center + anchor_offset;
        let theta = angle + direction * (arc_offset / radius);
        placements.push(GlyphPlacement {
            theta,
            rotation: theta + rotation_bias,
        });
        leading_edge += width;
    }
    placements
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 0.000_01;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < EPS,
            "expected {actual} to be within {EPS} of {expected}",
        );
    }

    #[test]
    fn center_anchor_mirrors_symmetric_widths_around_angle() {
        let layout = arc_glyph_layout(
            &[10.0, 20.0, 10.0],
            100.0,
            1.0,
            ArcAnchor::Center,
            ArcTextFacing::Outward,
        );

        assert_close(layout[0].theta, 0.85);
        assert_close(layout[1].theta, 1.0);
        assert_close(layout[2].theta, 1.15);
    }

    #[test]
    fn start_anchor_places_first_leading_edge_at_angle() {
        let layout = arc_glyph_layout(&[20.0], 80.0, 0.5, ArcAnchor::Start, ArcTextFacing::Outward);

        assert_close(layout[0].theta, 0.625);
    }

    #[test]
    fn end_anchor_places_last_trailing_edge_at_angle() {
        let layout = arc_glyph_layout(&[20.0], 80.0, 0.5, ArcAnchor::End, ArcTextFacing::Outward);

        assert_close(layout[0].theta, 0.375);
    }

    #[test]
    fn inward_facing_reverses_advance_and_adds_pi_to_rotation() {
        let outward = arc_glyph_layout(
            &[10.0, 10.0],
            50.0,
            1.0,
            ArcAnchor::Start,
            ArcTextFacing::Outward,
        );
        let inward = arc_glyph_layout(
            &[10.0, 10.0],
            50.0,
            1.0,
            ArcAnchor::Start,
            ArcTextFacing::Inward,
        );

        assert_close(outward[0].theta, 1.1);
        assert_close(outward[1].theta, 1.3);
        assert_close(inward[0].theta, 0.9);
        assert_close(inward[1].theta, 0.7);
        assert_close(inward[0].rotation, inward[0].theta + std::f32::consts::PI);
        assert_close(inward[1].rotation, inward[1].theta + std::f32::consts::PI);
    }

    #[test]
    fn known_sweep_matches_hand_computed_boundaries() {
        let layout = arc_glyph_layout(
            &[8.0, 12.0, 20.0],
            40.0,
            0.0,
            ArcAnchor::Center,
            ArcTextFacing::Outward,
        );

        assert_close(layout[0].theta, -0.4);
        assert_close(layout[2].theta, 0.25);
    }

    #[test]
    fn non_positive_radius_draws_nothing() {
        assert!(
            arc_glyph_layout(&[10.0], 0.0, 0.0, ArcAnchor::Center, ArcTextFacing::Outward)
                .is_empty()
        );
        assert!(
            arc_glyph_layout(
                &[10.0],
                -1.0,
                0.0,
                ArcAnchor::Center,
                ArcTextFacing::Outward
            )
            .is_empty()
        );
    }
}

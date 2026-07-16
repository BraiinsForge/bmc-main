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

//! Viewport-aware layout policy for the flip-clock face.

const PANEL_COUNT: f32 = 6.0;
const COLON_COUNT: f32 = 2.0;
const GAP_COUNT: f32 = 7.0;

const PANEL_ASPECT_RATIO: f32 = 257.0_f32 / 200.0_f32;
const BASE_SCALE_FACTOR: f32 = 0.85_f32;
const BASE_PANEL_WIDTH: f32 = (200.0_f32 * BASE_SCALE_FACTOR) / 480.0_f32;
const BASE_COLON_WIDTH: f32 = 0.05_f32;
const BASE_GAP: f32 = 0.02_f32;
const BASE_BORDER_WIDTH: f32 = 0.008_f32;
const GAP_HEIGHT_RATIO: f32 = 4.0_f32 / 200.0_f32;

const COLON_WIDTH_RATIO: f32 = BASE_COLON_WIDTH / BASE_PANEL_WIDTH;
const GAP_RATIO: f32 = BASE_GAP / BASE_PANEL_WIDTH;
const BORDER_WIDTH_RATIO: f32 = BASE_BORDER_WIDTH / BASE_PANEL_WIDTH;
const FRAME_WIDTH_RATIO: f32 = PANEL_COUNT
    + COLON_COUNT * COLON_WIDTH_RATIO
    + GAP_COUNT * GAP_RATIO
    + 2.0 * BORDER_WIDTH_RATIO;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ClockLayout {
    pub(crate) panel_width: f32,
    pub(crate) panel_height: f32,
    pub(crate) colon_width: f32,
    pub(crate) gap: f32,
    pub(crate) total_width: f32,
    pub(crate) start_x: f32,
    pub(crate) border_width: f32,
    pub(crate) gap_height: f32,
}

impl ClockLayout {
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "widget viewports are small fixed pixel sizes"
    )]
    pub(crate) fn for_viewport(width: u32, height: u32) -> Self {
        assert_ne!(height, 0, "BUG: viewport height must be non-zero");

        let viewport_aspect = width as f32 / height as f32;
        let panel_width = BASE_PANEL_WIDTH.min(viewport_aspect / FRAME_WIDTH_RATIO);
        let panel_height = panel_width * PANEL_ASPECT_RATIO;
        let colon_width = panel_width * COLON_WIDTH_RATIO;
        let gap = panel_width * GAP_RATIO;
        let total_width = PANEL_COUNT * panel_width + COLON_COUNT * colon_width + GAP_COUNT * gap;
        let start_x = -total_width / 2.0 + panel_width / 2.0;
        let border_width = panel_width * BORDER_WIDTH_RATIO;
        let gap_height = panel_width * GAP_HEIGHT_RATIO;

        Self {
            panel_width,
            panel_height,
            colon_width,
            gap,
            total_width,
            start_x,
            border_width,
            gap_height,
        }
    }

    #[must_use]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by layout tests to assert viewport fit")
    )]
    pub(crate) fn frame_total_width(self) -> f32 {
        self.total_width + self.border_width * 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::ClockLayout;

    const EPSILON: f32 = 0.0001_f32;

    #[expect(
        clippy::cast_precision_loss,
        reason = "test viewports are small fixed pixel sizes"
    )]
    fn viewport_aspect(width: u32, height: u32) -> f32 {
        width as f32 / height as f32
    }

    #[test]
    fn fullscreen_layout_matches_existing_geometry() {
        let layout = ClockLayout::for_viewport(1280, 480);

        assert!((layout.panel_width - 0.354_166_66_f32).abs() < EPSILON);
        assert!((layout.panel_height - 0.455_104_17_f32).abs() < EPSILON);
        assert!((layout.colon_width - 0.05_f32).abs() < EPSILON);
        assert!((layout.gap - 0.02_f32).abs() < EPSILON);
    }

    #[test]
    fn supported_viewports_fit_the_clock_face() {
        for (width, height) in [(317, 238), (638, 238), (638, 480), (1280, 480)] {
            let layout = ClockLayout::for_viewport(width, height);

            assert!(
                layout.frame_total_width() <= viewport_aspect(width, height) + EPSILON,
                "layout overflowed viewport {width}x{height}: {layout:?}"
            );
        }
    }

    #[test]
    fn layout_covers_all_platform_viewports() {
        let cases = [
            ("BMC100", 1_280_u32, 480_u32),
            ("BMM100", 320, 240),
            ("BMM101", 480, 320),
            ("BFM100", 480, 480),
        ];
        for (name, w, h) in cases {
            let layout = ClockLayout::for_viewport(w, h);
            let viewport_aspect = viewport_aspect(w, h);
            assert!(
                layout.frame_total_width() <= viewport_aspect + EPSILON,
                "{name}: frame width must fit viewport {w}x{h}"
            );
            assert!(
                layout.panel_height <= 1.0 + EPSILON,
                "{name}: panel height must fit viewport {w}x{h}"
            );
            assert!(
                layout.frame_total_width() > 0.0 && layout.panel_height > 0.0,
                "{name}: layout must produce positive dimensions"
            );
        }
    }

    #[test]
    fn narrow_viewports_scale_down_from_fullscreen() {
        let full = ClockLayout::for_viewport(1280, 480);
        let large = ClockLayout::for_viewport(638, 480);
        let small = ClockLayout::for_viewport(317, 238);

        assert!(large.panel_width < full.panel_width);
        assert!(small.panel_width < full.panel_width);
        assert!(large.total_width < full.total_width);
        assert!(small.total_width < full.total_width);
    }
}

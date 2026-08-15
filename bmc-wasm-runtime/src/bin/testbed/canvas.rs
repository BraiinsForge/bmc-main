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

//! The pannable, zoomable plane the device windows float over.
//!
//! The transform is applied by hand rather than to the windows' layers,
//! because only the device pixels should scale. egui rasterises glyphs
//! without knowing about a layer transform, so a scaled layer resamples
//! its own title text; placing windows ourselves leaves the chrome alone.

use std::collections::HashMap;

use egui::emath::TSTransform;

/// Zoom range. The floor keeps a canvas full of devices legible.
/// The ceiling is the device's own pixels: beyond them, type is judged
/// at a size it never has on hardware.
pub(super) const MIN_ZOOM: f32 = 0.1;
pub(super) const MAX_ZOOM: f32 = 1.0;

/// Breathing room left around the windows when fitting them.
const FIT_MARGIN: f32 = 16.0;

/// A fit measures what the last frame painted, and the chrome in those bounds
/// will not shrink with the rest, so one pass overshoots. Each pass divides
/// the error, and a fit that has converged stops early.
const FIT_PASSES: usize = 8;
const FIT_TOLERANCE: f32 = 0.01;

pub(super) struct Canvas {
    /// Where the canvas sits on screen, taken from the central panel.
    ///
    /// Chrome panels are shown inside the root `Ui`, so they shrink it and
    /// not the context — which would hand back the whole window.
    pub(super) rect: egui::Rect,
    /// Canvas space to screen: one scale and offset for every device at once,
    /// so they stay comparable — seeing them together is the point.
    to_screen: TSTransform,
    /// Where each window sits in canvas space, so a zoom moves them as one
    /// and a drag reads back into the space it was placed from.
    positions: HashMap<egui::Id, egui::Pos2>,
    /// Passes left of a pending fit.
    fit_passes: usize,
    /// What the device windows covered last frame, on screen.
    bounds: Option<egui::Rect>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            rect: egui::Rect::ZERO,
            to_screen: TSTransform::IDENTITY,
            positions: HashMap::new(),
            fit_passes: 0,
            bounds: None,
        }
    }
}

impl Canvas {
    pub(super) fn zoom(&self) -> f32 {
        self.to_screen.scaling
    }

    /// Where a window belongs on screen this frame, placing it at
    /// `default_pos` the first time it is asked for.
    pub(super) fn screen_pos(&mut self, id: egui::Id, default_pos: egui::Pos2) -> egui::Pos2 {
        let canvas = *self.positions.entry(id).or_insert(default_pos);
        self.to_screen * canvas
    }

    /// Move a window, in canvas space.
    pub(super) fn place(&mut self, id: egui::Id, pos: egui::Pos2) {
        self.positions.insert(id, pos);
    }

    /// Screen back to canvas space, for anything measured the way the
    /// operator sees it — an arrangement, say.
    pub(super) fn to_canvas(&self, screen: egui::Pos2) -> egui::Pos2 {
        self.to_screen.inverse() * screen
    }

    /// Adopt a zoom outright, holding the canvas's own origin still.
    pub(super) fn set_zoom(&mut self, zoom: f32) {
        let origin = self.rect.min;
        self.zoom_about(zoom, origin);
    }

    /// Take back where a window ended up, carrying any drag into canvas space.
    pub(super) fn record(&mut self, id: egui::Id, screen: egui::Rect) {
        self.positions
            .insert(id, self.to_screen.inverse() * screen.min);
        self.bounds = Some(self.bounds.map_or(screen, |seen| seen.union(screen)));
    }

    pub(super) fn pan_by(&mut self, delta: egui::Vec2) {
        self.to_screen.translation += delta;
    }

    /// Re-zoom while holding `anchor` still on screen.
    pub(super) fn zoom_about(&mut self, zoom: f32, anchor: egui::Pos2) {
        let held = self.to_screen.inverse() * anchor;
        let scaling = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.to_screen = TSTransform::new(anchor.to_vec2() - scaling * held.to_vec2(), scaling);
    }

    pub(super) fn request_fit(&mut self) {
        self.fit_passes = FIT_PASSES;
    }

    /// Start a frame's worth of window reports.
    pub(super) fn forget_bounds(&mut self) {
        self.bounds = None;
    }

    /// Frame every reported window, while a fit is pending.
    ///
    /// Runs after the windows have painted, since only then has egui settled
    /// where they are — one dragged this frame reports where it now sits,
    /// not where it was placed.
    pub(super) fn apply_pending_fit(&mut self) {
        if self.fit_passes == 0 {
            return;
        }
        let Some(bounds) = self.bounds.filter(|b| b.width() > 0.0 && b.height() > 0.0) else {
            return;
        };
        self.fit_passes -= 1;

        let room = self.rect.shrink(FIT_MARGIN);
        let ratio = (room.width() / bounds.width()).min(room.height() / bounds.height());
        if (ratio - 1.0).abs() < FIT_TOLERANCE {
            self.fit_passes = 0;
            return;
        }
        let held = self.to_screen.inverse() * bounds.min;
        let scaling = (self.to_screen.scaling * ratio).clamp(MIN_ZOOM, MAX_ZOOM);
        self.to_screen = TSTransform::new(room.min.to_vec2() - scaling * held.to_vec2(), scaling);
    }
}

#[cfg(test)]
mod tests {
    use super::Canvas;

    fn canvas() -> Canvas {
        Canvas {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1000.0)),
            ..Canvas::default()
        }
    }

    #[test]
    fn zooming_holds_the_anchor_still() {
        let mut canvas = canvas();
        canvas.pan_by(egui::vec2(37.0, -12.0));
        let anchor = egui::pos2(400.0, 300.0);
        let before = canvas.to_screen.inverse() * anchor;

        // Out, not in: the ceiling is 1× and the canvas starts there, so
        // zooming in would clamp to a transform identical to the one under
        // test and hold the anchor for no reason.
        canvas.zoom_about(0.4, anchor);

        let after = canvas.to_screen.inverse() * anchor;
        assert!(
            (before - after).length() < 0.01,
            "the canvas point under {anchor:?} moved from {before:?} to {after:?}"
        );
    }

    #[test]
    fn fitting_one_small_device_stops_at_its_own_pixels() {
        let mut canvas = canvas();
        canvas.request_fit();
        // A BMM100 panel on a canvas with room for ten of them.
        canvas.record(
            egui::Id::new("window"),
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 240.0)),
        );

        canvas.apply_pending_fit();

        assert!(
            canvas.zoom() <= super::MAX_ZOOM,
            "a fit magnified the device to {}×, which shows pixels the hardware does not have",
            canvas.zoom(),
        );
    }

    #[test]
    fn a_drag_survives_the_zoom_it_was_made_at() {
        let mut canvas = canvas();
        let id = egui::Id::new("window");
        canvas.place(id, egui::pos2(100.0, 100.0));
        canvas.zoom_about(0.5, egui::Pos2::ZERO);

        // Dragged 40 screen px right, which is 80 canvas px at half scale.
        let placed = canvas.screen_pos(id, egui::Pos2::ZERO);
        canvas.record(
            id,
            egui::Rect::from_min_size(placed + egui::vec2(40.0, 0.0), egui::vec2(10.0, 10.0)),
        );

        assert_eq!(canvas.positions[&id], egui::pos2(180.0, 100.0));
    }

    #[test]
    fn a_fit_brings_scattered_windows_into_the_canvas() {
        let mut canvas = canvas();
        canvas.request_fit();
        // Fitting measures the screen, so converging takes several passes.
        for _ in 0..8 {
            canvas.forget_bounds();
            for (id, size) in [
                ("wide", egui::vec2(2000.0, 1000.0)),
                ("small", egui::vec2(200.0, 200.0)),
            ] {
                let id = egui::Id::new(id);
                let pos = canvas.screen_pos(id, egui::pos2(-500.0, -500.0));
                canvas.record(id, egui::Rect::from_min_size(pos, size * canvas.zoom()));
            }
            canvas.apply_pending_fit();
        }

        let bounds = canvas.bounds.expect("BUG: both windows were reported");
        assert!(
            canvas.rect.contains_rect(bounds),
            "{bounds:?} escapes the canvas {:?}",
            canvas.rect
        );
    }

    #[test]
    fn a_fit_without_windows_stays_pending() {
        let mut canvas = canvas();
        canvas.request_fit();
        canvas.apply_pending_fit();
        assert!(
            canvas.fit_passes > 0,
            "nothing was reported, so there was nothing to frame yet"
        );
    }
}

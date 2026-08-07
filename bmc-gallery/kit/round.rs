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

//! The round face: a Deck variant whose display is circular, so a square
//! render has to be masked back down to the inscribed circle.

use gallery::eframe::egui;

/// Segments around the circle — enough that the edge reads as smooth at the
/// sizes a face is previewed at.
const SEGMENTS: u32 = 96;

/// How much of what the mask covers still shows through. The face reads as
/// round at a glance, and a widget drawing into the corners — which the device
/// would simply cut off — is still there to be noticed.
const MASK_OPACITY: f32 = 0.82;

/// Mask the square image already drawn in `rect` to its inscribed circle, and
/// ring it with a bezel.
///
/// Painted over the image rather than clipping it: egui draws a texture as one
/// quad, and the corners have to go somewhere. An annulus from the circle's
/// edge out past the corners covers them with the canvas behind.
pub(crate) fn mask(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    let backdrop = ui.visuals().panel_fill.gamma_multiply(MASK_OPACITY);

    let mut ring = egui::epaint::Mesh::default();
    for i in 0..=SEGMENTS {
        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let dir = egui::vec2(angle.cos(), angle.sin());
        ring.colored_vertex(center + dir * radius, backdrop);
        ring.colored_vertex(center + dir * (radius * 2.0), backdrop);
    }
    for i in 0..SEGMENTS {
        let base = i * 2;
        ring.add_triangle(base, base + 1, base + 2);
        ring.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(egui::Shape::mesh(ring));

    // Inset, so the stroke stays inside the rect: the inscribed circle touches
    // the edges, and a ring drawn at `radius` would clip at the tangents.
    painter.circle_stroke(
        center,
        radius - 1.5,
        egui::Stroke::new(1.5, egui::Color32::from_gray(80)),
    );
}

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

//! Toolbar icons: SVG assets rasterized to tintable masks.
//!
//! `resvg` renders each icon at the exact pixel size it is drawn at, so
//! nothing is rescaled. Only coverage is kept: the colour arrives as a tint
//! at draw time, which lets an icon take the text colour of its button state.

use std::collections::HashMap;

/// A bundled SVG, rasterized on demand and cached per pixel size.
pub(crate) struct Icon {
    svg: &'static [u8],
    cache: HashMap<u32, egui::TextureHandle>,
}

impl Icon {
    fn new(svg: &'static [u8]) -> Self {
        Self {
            svg,
            cache: HashMap::new(),
        }
    }

    /// Paint the icon into `rect`, in `color`.
    ///
    /// Both the destination and the raster are snapped to whole physical
    /// pixels. Layout hands out fractional coordinates, and an icon drawn
    /// across a half pixel — or rasterized at one size and drawn at another —
    /// is resampled, which reads as a blur beside the hinted text next to it.
    pub(crate) fn paint(&mut self, ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
        let ppp = ui.ctx().pixels_per_point();
        let to_grid = |value: f32| (value * ppp).round() / ppp;
        let side = to_grid(rect.width().min(rect.height()));
        let rect = egui::Rect::from_min_size(
            egui::pos2(to_grid(rect.min.x), to_grid(rect.min.y)),
            egui::Vec2::splat(side),
        );

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an icon is a handful of physical pixels square"
        )]
        let px = (side * ppp).round() as u32;
        if px == 0 {
            return;
        }
        let svg = self.svg;
        let texture = self.cache.entry(px).or_insert_with(|| {
            ui.ctx().load_texture(
                format!("icon_{px}"),
                mask(svg, px),
                egui::TextureOptions::LINEAR,
            )
        });
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

/// Every icon the toolbar paints.
pub(crate) struct Icons {
    pub(crate) theme_auto: Icon,
    pub(crate) theme_dark: Icon,
    pub(crate) theme_light: Icon,
    pub(crate) arrange_grid: Icon,
    pub(crate) debug: Icon,
    pub(crate) offline: Icon,
    pub(crate) sleep: Icon,
    pub(crate) record: Icon,
    pub(crate) delay: Icon,
    pub(crate) devices: Icon,
    pub(crate) reload: Icon,
    pub(crate) timer: Icon,
    pub(crate) scale_in: Icon,
    pub(crate) scale_out: Icon,
    pub(crate) camera: Icon,
    pub(crate) save: Icon,
    pub(crate) close: Icon,
    pub(crate) saved: Icon,
    pub(crate) warning: Icon,
    // The recording log's per-event-kind marks.
    pub(crate) touch: Icon,
    pub(crate) delivery: Icon,
    pub(crate) network: Icon,
    pub(crate) output: Icon,
}

impl Icons {
    pub(crate) fn new() -> Self {
        Self {
            theme_auto: Icon::new(include_bytes!("assets/icons/theme-auto.svg")),
            theme_dark: Icon::new(include_bytes!("assets/icons/theme-dark.svg")),
            theme_light: Icon::new(include_bytes!("assets/icons/theme-light.svg")),
            arrange_grid: Icon::new(include_bytes!("assets/icons/arrange-grid.svg")),
            debug: Icon::new(include_bytes!("assets/icons/debug.svg")),
            offline: Icon::new(include_bytes!("assets/icons/offline.svg")),
            sleep: Icon::new(include_bytes!("assets/icons/sleep.svg")),
            record: Icon::new(include_bytes!("assets/icons/record.svg")),
            delay: Icon::new(include_bytes!("assets/icons/delay.svg")),
            devices: Icon::new(include_bytes!("assets/icons/devices.svg")),
            reload: Icon::new(include_bytes!("assets/icons/reset.svg")),
            timer: Icon::new(include_bytes!("assets/icons/timer.svg")),
            scale_in: Icon::new(include_bytes!("assets/icons/scale-in.svg")),
            scale_out: Icon::new(include_bytes!("assets/icons/scale-out.svg")),
            camera: Icon::new(include_bytes!("assets/icons/camera.svg")),
            save: Icon::new(include_bytes!("assets/icons/save.svg")),
            close: Icon::new(include_bytes!("assets/icons/close.svg")),
            saved: Icon::new(include_bytes!("assets/icons/saved.svg")),
            warning: Icon::new(include_bytes!("assets/icons/warning.svg")),
            touch: Icon::new(include_bytes!("assets/icons/touch.svg")),
            delivery: Icon::new(include_bytes!("assets/icons/delivery.svg")),
            network: Icon::new(include_bytes!("assets/icons/network.svg")),
            output: Icon::new(include_bytes!("assets/icons/output.svg")),
        }
    }
}

/// Rasterize to white-on-transparent, `px` square.
///
/// # Panics
/// If the bytes aren't valid SVG. The icons are compiled in, so that is a
/// build error rather than anything an operator can hit.
fn mask(svg: &[u8], px: u32) -> egui::ColorImage {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .expect("BUG: a bundled icon must be valid SVG");

    let size = tree.size();
    #[expect(clippy::cast_precision_loss, reason = "an icon is a few pixels square")]
    let scale = px as f32 / size.width().max(size.height());

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(px, px).expect("BUG: an icon must have a nonzero size");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let pixels = pixmap
        .pixels()
        .iter()
        .map(|p| {
            let a = p.alpha();
            egui::Color32::from_rgba_premultiplied(a, a, a, a)
        })
        .collect();
    egui::ColorImage::new([px as usize, px as usize], pixels)
}

#[cfg(test)]
mod tests {
    use super::{Icon, Icons, mask};

    #[test]
    fn every_bundled_icon_rasterizes_to_visible_coverage() {
        // Parsing is not drawing: an empty or mis-scaled viewBox rasterizes
        // blank, so check coverage rather than trusting `new` not to panic.
        let icons = Icons::new();
        let named: [(&str, &Icon); 23] = [
            ("theme-auto", &icons.theme_auto),
            ("theme-dark", &icons.theme_dark),
            ("theme-light", &icons.theme_light),
            ("arrange-grid", &icons.arrange_grid),
            ("debug", &icons.debug),
            ("offline", &icons.offline),
            ("sleep", &icons.sleep),
            ("record", &icons.record),
            ("delay", &icons.delay),
            ("devices", &icons.devices),
            ("reset", &icons.reload),
            ("timer", &icons.timer),
            ("saved", &icons.saved),
            ("warning", &icons.warning),
            ("touch", &icons.touch),
            ("delivery", &icons.delivery),
            ("network", &icons.network),
            ("output", &icons.output),
            ("scale-in", &icons.scale_in),
            ("scale-out", &icons.scale_out),
            ("camera", &icons.camera),
            ("save", &icons.save),
            ("close", &icons.close),
        ];
        for (name, icon) in named {
            let image = mask(icon.svg, 32);
            assert!(
                image.pixels.iter().any(|p| p.a() > 0),
                "`{name}` rasterized to nothing"
            );
        }
    }
}

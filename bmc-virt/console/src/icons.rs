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

//! SVG icon rasterizer for the console UI.
//!
//! Each icon is rasterized to white-on-transparent at the exact requested pixel
//! size (no texture rescaling), then tinted at draw time via `egui::Image::tint()`.
//! Textures are cached per pixel size.
//!
//! Usage:
//! ```ignore
//! let icon = svg_icon!("../assets/icons/power.svg");
//! ui.add(icon.image(12.0, Color32::WHITE));
//! ```

use std::collections::HashMap;

/// Compile-time SVG embed + runtime rasterizer.
///
/// Use the [`svg_icon!`] macro to create instances.
pub struct SvgIcon {
    svg_bytes: &'static [u8],
    cache: HashMap<u32, egui::TextureHandle>,
}

impl SvgIcon {
    /// Create from statically embedded SVG bytes. Prefer [`svg_icon!`].
    #[must_use]
    pub fn new(svg_bytes: &'static [u8]) -> Self {
        Self {
            svg_bytes,
            cache: HashMap::new(),
        }
    }

    /// Get or rasterize the texture at the given pixel size.
    pub fn texture(&mut self, ctx: &egui::Context, px: u32) -> egui::TextureHandle {
        self.cache
            .entry(px)
            .or_insert_with(|| rasterize_white(self.svg_bytes, ctx, px))
            .clone()
    }

    /// Render as an `egui::Image` at the given logical size and tint color.
    /// Rasterizes at the exact pixel size on first use, caches the texture.
    pub fn image(
        &mut self,
        ctx: &egui::Context,
        size: f32,
        tint: egui::Color32,
    ) -> egui::Image<'_> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "icon size is small and positive"
        )]
        let px = size.ceil() as u32;
        let tex = self
            .cache
            .entry(px)
            .or_insert_with(|| rasterize_white(self.svg_bytes, ctx, px));
        egui::Image::new(tex as &egui::TextureHandle)
            .fit_to_exact_size(egui::vec2(size, size))
            .tint(tint)
    }
}

/// Embed an SVG file as an [`SvgIcon`] with compile-time `include_bytes!`.
macro_rules! svg_icon {
    ($path:literal) => {
        $crate::icons::SvgIcon::new(include_bytes!($path))
    };
}
pub(crate) use svg_icon;

/// Rasterize SVG to white-on-transparent at exact pixel size.
fn rasterize_white(svg_bytes: &[u8], ctx: &egui::Context, size: u32) -> egui::TextureHandle {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &resvg::usvg::Options::default())
        .expect("BUG: failed to parse SVG icon");

    let svg_size = tree.size();
    #[expect(clippy::cast_precision_loss, reason = "icon size is small")]
    let scale = (size as f32) / svg_size.width().max(svg_size.height());

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size, size).expect("BUG: failed to create pixmap");

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Normalize to white-on-transparent: original fill color is discarded,
    // only alpha is preserved. Tint color is applied at render time via
    // egui::Image::tint().
    let pixels: Vec<egui::Color32> = pixmap
        .pixels()
        .iter()
        .map(|p| {
            let a = p.alpha();
            egui::Color32::from_rgba_premultiplied(a, a, a, a)
        })
        .collect();

    ctx.load_texture(
        format!("icon_{size}"),
        egui::ColorImage::new([size as usize, size as usize], pixels),
        egui::TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
    )
}

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

//! SVG icon loading — rasterizes embedded SVGs into egui textures at startup.
//!
//! Icons are monochrome (white on transparent) and tinted at render time via
//! the egui `Image::tint()` API.

use egui::{ColorImage, TextureHandle, TextureOptions};
use std::path::Path;

/// Paint a tinted icon at a given size. Returns the response rect.
pub fn icon_image(texture: &TextureHandle, size: f32, tint: egui::Color32) -> egui::Image<'_> {
    egui::Image::new(texture)
        .fit_to_exact_size(egui::vec2(size, size))
        .tint(tint)
}

/// Rasterize an SVG to a white-on-transparent egui texture at `ICON_SIZE`.
fn load_svg_from_bytes(ctx: &egui::Context, svg_path: &str, svg_bytes: &[u8]) -> TextureHandle {
    let name = Path::new(svg_path)
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_else(|| panic!("BUG: failed to derive file stem from {svg_path}"));

    let tree = resvg::usvg::Tree::from_data(svg_bytes, &resvg::usvg::Options::default())
        .unwrap_or_else(|e| panic!("BUG: failed to parse {svg_path}: {e}"));

    let svg_size = tree.size();
    #[expect(clippy::cast_precision_loss)]
    let scale = (ICON_SIZE as f32) / svg_size.width().max(svg_size.height());

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE).expect("BUG: failed to create pixmap");

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Normalize to white-on-transparent: any pixel with alpha > 0 becomes
    // white at that alpha. This makes the source SVG fill color irrelevant —
    // the actual display color is applied via egui's Image::tint() at render time.
    let pixels: Vec<egui::Color32> = pixmap
        .pixels()
        .iter()
        .map(|p| {
            let a = p.alpha();
            egui::Color32::from_rgba_premultiplied(a, a, a, a)
        })
        .collect();

    let size = [ICON_SIZE as usize, ICON_SIZE as usize];
    let image = ColorImage::new(size, pixels);

    ctx.load_texture(
        format!("icon_{name}"),
        image,
        TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
    )
}

macro_rules! load_svg {
    ($ctx:expr, $path:literal) => {
        load_svg_from_bytes($ctx, $path, include_bytes!($path))
    };
}

/// Icon size in logical pixels.
const ICON_SIZE: u32 = 16;

/// All loaded icon textures.
pub struct Icons {
    pub app: TextureHandle,
    pub caret_down: TextureHandle,
    pub caret_left: TextureHandle,
    pub caret_right: TextureHandle,
    pub caret_up: TextureHandle,
    pub close: TextureHandle,
    pub code: TextureHandle,
    pub color_palette: TextureHandle,
    pub folder: TextureHandle,
    pub pause: TextureHandle,
    pub play: TextureHandle,
    pub renew: TextureHandle,
    pub search: TextureHandle,
    pub touch: TextureHandle,
}

impl Icons {
    /// Rasterize all embedded SVGs and register as egui textures.
    pub fn load(ctx: &egui::Context) -> Self {
        Self {
            app: load_svg!(ctx, "../assets/icons/app.svg"),
            caret_down: load_svg!(ctx, "../assets/icons/caret-down.svg"),
            caret_left: load_svg!(ctx, "../assets/icons/caret-left.svg"),
            caret_right: load_svg!(ctx, "../assets/icons/caret-right.svg"),
            caret_up: load_svg!(ctx, "../assets/icons/caret-up.svg"),
            close: load_svg!(ctx, "../assets/icons/close.svg"),
            code: load_svg!(ctx, "../assets/icons/code.svg"),
            color_palette: load_svg!(ctx, "../assets/icons/color-palette.svg"),
            folder: load_svg!(ctx, "../assets/icons/folder.svg"),
            pause: load_svg!(ctx, "../assets/icons/pause.svg"),
            play: load_svg!(ctx, "../assets/icons/play.svg"),
            renew: load_svg!(ctx, "../assets/icons/renew.svg"),
            search: load_svg!(ctx, "../assets/icons/search.svg"),
            touch: load_svg!(ctx, "../assets/icons/touch.svg"),
        }
    }
}

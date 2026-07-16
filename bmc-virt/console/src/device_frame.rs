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

// Device frame overlay rendered from the embedded SVG asset.
// Rasterized once at startup via resvg, composited as a GL texture.
// The screen cutout area is defined by the SVG's #screen rect coordinates.

use eframe::glow;
use glow::HasContext;

/// Embedded SVG asset.
const SVG_DATA: &[u8] = include_bytes!("../assets/bmc.svg");

// Screen rect in SVG viewbox coordinates (from the #screen path):
//   M37.223 32.963 h426.014 V191.77 H37.223 Z
const SVG_WIDTH: f32 = 500.0;
const SVG_HEIGHT: f32 = 262.543;
const SCREEN_LEFT: f32 = 37.223;
const SCREEN_TOP: f32 = 32.963;
const SCREEN_RIGHT: f32 = 37.223 + 426.014;
const SCREEN_BOTTOM: f32 = 191.77;

/// Screen rect as fractions of the device frame.
pub const SCREEN_FRAC_LEFT: f32 = SCREEN_LEFT / SVG_WIDTH;
pub const SCREEN_FRAC_TOP: f32 = SCREEN_TOP / SVG_HEIGHT;
pub const SCREEN_FRAC_RIGHT: f32 = SCREEN_RIGHT / SVG_WIDTH;
pub const SCREEN_FRAC_BOTTOM: f32 = SCREEN_BOTTOM / SVG_HEIGHT;

/// Device frame aspect ratio (width / height).
pub const DEVICE_ASPECT: f32 = SVG_WIDTH / SVG_HEIGHT;

/// Rasterized device frame texture.
pub struct DeviceFrame {
    egui_texture_id: egui::TextureId,
}

impl DeviceFrame {
    /// Rasterize the SVG and create a GL texture.
    /// `render_width` is the target rasterization width in pixels.
    pub fn new(gl: &glow::Context, frame: &mut eframe::Frame, render_width: u32) -> Self {
        let tree = resvg::usvg::Tree::from_data(SVG_DATA, &resvg::usvg::Options::default())
            .unwrap_or_else(|e| panic!("failed to parse device frame SVG: {e}"));

        let svg_size = tree.size();
        #[expect(
            clippy::cast_precision_loss,
            reason = "render_width is ~2000, exact in f32"
        )]
        let scale = render_width as f32 / svg_size.width();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "scaled SVG height is a small positive value"
        )]
        let render_height = (svg_size.height() * scale).ceil() as u32;

        let mut pixmap = resvg::tiny_skia::Pixmap::new(render_width, render_height)
            .unwrap_or_else(|| panic!("failed to create pixmap"));

        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Upload as GL texture (RGBA premultiplied)
        #[expect(
            clippy::cast_possible_wrap,
            reason = "GL enums and render dimensions fit in i32"
        )]
        let gl_texture = unsafe {
            let tex = gl
                .create_texture()
                .unwrap_or_else(|e| panic!("failed to create frame texture: {e}"));
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                render_width as i32,
                render_height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixmap.data())),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            tex
        };

        let native = glow::NativeTexture(gl_texture.0);
        let egui_texture_id = frame.register_native_glow_texture(native);

        tracing::info!("rasterized device frame: {render_width}x{render_height}");

        Self { egui_texture_id }
    }

    /// Paint the device frame into the given rect.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        let mesh = egui::Mesh {
            texture_id: self.egui_texture_id,
            vertices: vec![
                egui::epaint::Vertex {
                    pos: rect.left_top(),
                    uv: egui::pos2(0.0, 0.0),
                    color: egui::Color32::WHITE,
                },
                egui::epaint::Vertex {
                    pos: rect.right_top(),
                    uv: egui::pos2(1.0, 0.0),
                    color: egui::Color32::WHITE,
                },
                egui::epaint::Vertex {
                    pos: rect.right_bottom(),
                    uv: egui::pos2(1.0, 1.0),
                    color: egui::Color32::WHITE,
                },
                egui::epaint::Vertex {
                    pos: rect.left_bottom(),
                    uv: egui::pos2(0.0, 1.0),
                    color: egui::Color32::WHITE,
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        painter.add(egui::Shape::mesh(mesh));
    }

    /// Given the device frame rect, compute the sub-rect for the screen area.
    pub fn screen_rect(device_rect: egui::Rect) -> egui::Rect {
        let w = device_rect.width();
        let h = device_rect.height();
        egui::Rect::from_min_max(
            egui::pos2(
                device_rect.min.x + SCREEN_FRAC_LEFT * w,
                device_rect.min.y + SCREEN_FRAC_TOP * h,
            ),
            egui::pos2(
                device_rect.min.x + SCREEN_FRAC_RIGHT * w,
                device_rect.min.y + SCREEN_FRAC_BOTTOM * h,
            ),
        )
    }
}

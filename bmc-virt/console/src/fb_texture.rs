// Copyright (C) 2026  Braiins Systems s.r.o.

// GL texture management for the framebuffer.
// Uploads raw pixels from shared memory (RGB565 or BGRA8888 depending on
// the guest's DRM format) and renders as a rotated quad via UV swizzling.

use bmc_virt_ipc::{Bpp, FB_HEIGHT, FB_WIDTH, PixelFormat, Stride};
use eframe::glow;
use glow::HasContext;

/// Manages a GL texture that holds the device framebuffer.
pub struct FbTexture {
    gl_texture: glow::Texture,
    egui_texture_id: egui::TextureId,
    last_seq: u64,
    /// GL pixel format for upload (set from frame_bpp: BGRA for 32, RGB565 for 16).
    gl_format: u32,
    gl_type: u32,
}

impl FbTexture {
    /// Create a new framebuffer texture and register it with egui.
    /// `bpp` is the source bits-per-pixel (16 or 32) from the control block.
    /// `format` is the byte order for 32-bit frames (ignored for 16-bit).
    #[expect(
        clippy::cast_possible_wrap,
        reason = "GL enum values and FB dimensions fit in i32"
    )]
    pub fn new(
        gl: &glow::Context,
        frame: &mut eframe::Frame,
        bpp: Bpp,
        format: PixelFormat,
    ) -> Self {
        // Pick GL format based on source bpp + byte order.
        // Use plain RGBA8 (not SRGB8) — egui_glow disables GL_FRAMEBUFFER_SRGB
        // and handles gamma in its own shaders, so sRGB internal formats would
        // cause double gamma correction (darker image).
        let (internal_format, gl_format, gl_type) = match bpp.0 {
            16 => (glow::RGB8 as i32, glow::RGB, glow::UNSIGNED_SHORT_5_6_5),
            32 => {
                let src_format = match format {
                    PixelFormat::Bgra8888 => glow::BGRA,
                    PixelFormat::Rgba8888 => glow::RGBA,
                };
                (glow::RGBA8 as i32, src_format, glow::UNSIGNED_BYTE)
            }
            other => panic!("unsupported bpp: {other}"),
        };

        let gl_texture = unsafe {
            let tex = gl
                .create_texture()
                .unwrap_or_else(|e| panic!("failed to create GL texture: {e}"));
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
                internal_format,
                FB_WIDTH as i32,
                FB_HEIGHT as i32,
                0,
                gl_format,
                gl_type,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            tex
        };

        let native = glow::NativeTexture(gl_texture.0);
        let egui_texture_id = frame.register_native_glow_texture(native);

        Self {
            gl_texture,
            egui_texture_id,
            last_seq: 0,
            gl_format,
            gl_type,
        }
    }

    /// Update the texture from shared memory if the frame sequence has changed.
    /// `pixel_data` is the raw framebuffer bytes (stride * height).
    #[expect(clippy::cast_possible_wrap, reason = "FB dimensions fit in i32")]
    pub fn update_if_changed(
        &mut self,
        gl: &glow::Context,
        seq: u64,
        pixel_data: &[u8],
        stride: Stride,
    ) -> bool {
        if seq == self.last_seq || seq == 0 {
            return false;
        }
        self.last_seq = seq;

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.gl_texture));
            // Set row length so GL handles stride != width * bpp correctly
            {
                let bytes_per_pixel: i32 = if self.gl_type == glow::UNSIGNED_SHORT_5_6_5 {
                    2
                } else {
                    4
                };
                #[expect(
                    clippy::integer_division,
                    reason = "stride is always a multiple of bytes_per_pixel"
                )]
                let row_length = stride.0 as i32 / bytes_per_pixel;
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, row_length);
            }
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                FB_WIDTH as i32,
                FB_HEIGHT as i32,
                self.gl_format,
                self.gl_type,
                glow::PixelUnpackData::Slice(Some(pixel_data)),
            );
            // Reset row length to default
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        true
    }

    /// Returns the egui texture ID for painting.
    #[expect(dead_code, reason = "will be used by device frame overlay")]
    pub fn texture_id(&self) -> egui::TextureId {
        self.egui_texture_id
    }

    /// Paint the framebuffer rotated 90° CW into the given rect.
    ///
    /// The source texture is 480×1280 (portrait), displayed as
    /// 1280×480 (landscape) via UV coordinate swizzling.
    pub fn paint_rotated(&self, painter: &egui::Painter, rect: egui::Rect) {
        // UV swizzle for 90° CW rotation:
        //   screen top-left    → texture bottom-left  (0, 1)
        //   screen top-right   → texture top-left     (0, 0)
        //   screen bottom-right→ texture top-right    (1, 0)
        //   screen bottom-left → texture bottom-right (1, 1)
        let mesh = egui::Mesh {
            texture_id: self.egui_texture_id,
            vertices: vec![
                egui::epaint::Vertex {
                    pos: rect.left_top(),
                    uv: egui::pos2(0.0, 1.0),
                    color: egui::Color32::WHITE,
                },
                egui::epaint::Vertex {
                    pos: rect.right_top(),
                    uv: egui::pos2(0.0, 0.0),
                    color: egui::Color32::WHITE,
                },
                egui::epaint::Vertex {
                    pos: rect.right_bottom(),
                    uv: egui::pos2(1.0, 0.0),
                    color: egui::Color32::WHITE,
                },
                egui::epaint::Vertex {
                    pos: rect.left_bottom(),
                    uv: egui::pos2(1.0, 1.0),
                    color: egui::Color32::WHITE,
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        };

        painter.add(egui::Shape::mesh(mesh));
    }
}

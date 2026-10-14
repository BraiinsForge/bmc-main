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

//! Offscreen targets: the static layer, its stencils, shadow FBOs and the render-to-texture probe.

#![expect(clippy::cast_precision_loss)]

use super::FemtoVgRenderer;
use super::geometry::scissor_box;
use crate::gpu::text::to_femtovg_color;
use crate::interaction::Rect;
use crate::renderer::prelude::*;
use bmc_wasm_protocol::colors::Color;
use femtovg::Paint;
use femtovg::Path;
use femtovg::RenderTarget;
use glow::HasContext;
use std::num::{NonZeroU16, NonZeroU32};

impl FemtoVgRenderer {
    /// Create a femtovg-managed render target image.
    ///
    /// Returns `(ImageId, raw_gl_texture_name)`. femtovg owns the texture and
    /// its internal FBO+stencil. The raw GL name can be registered with egui
    /// for display.
    pub fn create_render_target(&mut self, width: u32, height: u32) -> (femtovg::ImageId, u32) {
        let image_id = self
            .canvas
            .create_image_empty(
                width as usize,
                height as usize,
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::empty(),
            )
            .expect("BUG: failed to create femtovg render target image");
        let native = self
            .canvas
            .get_native_texture(image_id)
            .expect("BUG: failed to get native texture from femtovg image");
        let gl_name = native.0.get();
        tracing::debug!(?image_id, gl_name, width, height, "created render target");
        (image_id, gl_name)
    }

    /// Begin a frame targeting a femtovg Image (offscreen render target).
    ///
    /// Like [`crate::renderer::RenderFrame::begin_frame`] but renders to the given Image instead of
    /// the screen FBO. femtovg manages its own FBO with stencil for the Image.
    ///
    /// The image must be `width·dpi_scale × height·dpi_scale` texels.
    pub fn begin_frame_to_image(
        &mut self,
        image_id: femtovg::ImageId,
        width: u32,
        height: u32,
        dpi_scale: f32,
    ) {
        self.width = width as f32;
        self.height = height as f32;
        self.dpi_scale = dpi_scale;
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (pw, ph) = (
            (width as f32 * dpi_scale) as u32,
            (height as f32 * dpi_scale) as u32,
        );
        self.canvas.set_size(pw, ph, dpi_scale);
        // femtovg 0.20.4: `set_size` queues a switch to the screen without updating the
        // target it remembers, so `set_render_target` skips the switch back as redundant
        // and a second frame into the same image lands on the screen, freezing the image.
        self.canvas.set_render_target(RenderTarget::Screen);
        self.canvas.set_render_target(RenderTarget::Image(image_id));
        self.frame_target = RenderTarget::Image(image_id);
        self.canvas
            .clear_rect(0, 0, pw, ph, femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0));
        self.scale_to_logical(dpi_scale);
        self.paragraph_cache.begin_frame();
        self.brightness = 1.0;
    }

    /// Restore rendering to the screen FBO.
    pub fn set_render_target_screen(&mut self) {
        self.canvas.set_render_target(RenderTarget::Screen);
    }

    /// Check whether an offscreen RGBA8 render target is usable on this driver.
    ///
    /// Allocates a femtovg image the way a cached static layer would,
    /// then builds the framebuffer femtovg's `Framebuffer::new` would:
    /// the texture plus a `STENCIL_INDEX8` renderbuffer,
    /// which a driver can reject even when it accepts the texture alone.
    /// Worth probing rather than assuming:
    /// GLES2 only guarantees RGBA4/RGB5_A1/RGB565 *renderbuffers* are colour-renderable,
    /// and RGBA8 textures need `GL_OES_rgb8_rgba8`.
    pub(super) fn probe_render_to_texture(&mut self) -> RenderTargetProbe {
        const PROBE_PX: u32 = 16;

        let Ok(image_id) = self.canvas.create_image_empty(
            PROBE_PX as usize,
            PROBE_PX as usize,
            femtovg::PixelFormat::Rgba8,
            femtovg::ImageFlags::empty(),
        ) else {
            return RenderTargetProbe::ImageAllocFailed;
        };

        let probe = unsafe {
            // Drain anything an earlier call left queued so `get_error` below
            // reports only this probe.
            self.drain_gl_errors();

            match self.canvas.get_native_texture(image_id) {
                Ok(native) => {
                    // femtovg hands back the raw GL name; glow wants its newtype.
                    let texture = glow::NativeTexture(native.0);
                    match (self.gl.create_framebuffer(), self.gl.create_renderbuffer()) {
                        (Ok(fbo), Ok(stencil)) => {
                            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                            self.gl.framebuffer_texture_2d(
                                glow::FRAMEBUFFER,
                                glow::COLOR_ATTACHMENT0,
                                glow::TEXTURE_2D,
                                Some(texture),
                                0,
                            );
                            self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(stencil));
                            self.gl.renderbuffer_storage(
                                glow::RENDERBUFFER,
                                glow::STENCIL_INDEX8,
                                PROBE_PX.cast_signed(),
                                PROBE_PX.cast_signed(),
                            );
                            self.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                            self.gl.framebuffer_renderbuffer(
                                glow::FRAMEBUFFER,
                                glow::STENCIL_ATTACHMENT,
                                glow::RENDERBUFFER,
                                Some(stencil),
                            );
                            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
                            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                            self.gl.delete_framebuffer(fbo);
                            self.gl.delete_renderbuffer(stencil);
                            let gl_error = self.gl.get_error();
                            if gl_error != glow::NO_ERROR {
                                RenderTargetProbe::GlError(gl_error)
                            } else if status == glow::FRAMEBUFFER_COMPLETE {
                                RenderTargetProbe::Working
                            } else {
                                RenderTargetProbe::FramebufferIncomplete(status)
                            }
                        }
                        (fbo, stencil) => {
                            if let Ok(fbo) = fbo {
                                self.gl.delete_framebuffer(fbo);
                            }
                            if let Ok(stencil) = stencil {
                                self.gl.delete_renderbuffer(stencil);
                            }
                            self.drain_gl_errors();
                            RenderTargetProbe::FramebufferAllocFailed
                        }
                    }
                }
                Err(_) => RenderTargetProbe::NoNativeTexture,
            }
        };

        self.canvas.delete_image(image_id);
        probe
    }

    /// Delete the bound framebuffer and its stencil renderbuffer.
    ///
    /// `Framebuffer::new` returns `Err` without owning either, so nothing else frees them,
    /// and the framebuffer would keep the layer's texture alive after `delete_image`.
    ///
    /// # Safety
    /// The GL context must be current,
    /// and the framebuffer `Framebuffer::new` has just failed on still bound:
    /// on any later retarget the bound one is femtovg's.
    pub(super) unsafe fn delete_orphaned_framebuffer(&self) {
        unsafe {
            let Some(fbo) = self.gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING) else {
                return;
            };
            if let Some(stencil) = self.attached_stencil() {
                self.gl.delete_renderbuffer(stencil);
            }
            self.gl.delete_framebuffer(fbo);
        }
    }

    /// Borrow the pooled FBO pair at `width × height`,
    /// allocating or resizing as needed.
    /// `None` only if the GPU refuses the allocation.
    ///
    /// Single pair: two `drop_shadow` calls in one frame
    /// at different sizes would realloc each call.
    ///
    /// Clock shadows are all canvas-sized, so they don't;
    /// a multi-entry pool would be needed otherwise.
    fn acquire_shadow_fbos(
        &mut self,
        width: u32,
        height: u32,
    ) -> Option<(femtovg::ImageId, femtovg::ImageId)> {
        if let Some(pool) = &self.shadow_fbo_pool
            && pool.width == width
            && pool.height == height
        {
            return Some((pool.unblurred, pool.blurred));
        }
        if let Some(stale) = self.shadow_fbo_pool.take() {
            self.canvas.delete_image(stale.unblurred);
            self.canvas.delete_image(stale.blurred);
        }
        let unblurred = self
            .canvas
            .create_image_empty(
                width as usize,
                height as usize,
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::empty(),
            )
            .ok()?;
        let Ok(blurred) = self.canvas.create_image_empty(
            width as usize,
            height as usize,
            femtovg::PixelFormat::Rgba8,
            femtovg::ImageFlags::empty(),
        ) else {
            self.canvas.delete_image(unblurred);
            return None;
        };
        self.shadow_fbo_pool = Some(ShadowFboPool {
            width,
            height,
            unblurred,
            blurred,
        });
        Some((unblurred, blurred))
    }

    /// Seed the stencil pool with the screen target's own stencil.
    ///
    /// The screen target is typically display-sized, so a layer spanning the whole display
    /// borrows its stencil instead of allocating one.
    pub(super) fn adopt_screen_stencil(&mut self) {
        let Some(fbo) = self.screen_fbo else {
            tracing::debug!("default framebuffer as the screen target; no stencil to share");
            return;
        };
        // SAFETY: the GL context is current for the renderer's lifetime,
        // and no frame is in flight at construction whose bindings this could disturb.
        let adopted = unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            let adopted = self.attached_stencil().map(|rbo| {
                self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
                let width = self
                    .gl
                    .get_renderbuffer_parameter_i32(glow::RENDERBUFFER, glow::RENDERBUFFER_WIDTH);
                let height = self
                    .gl
                    .get_renderbuffer_parameter_i32(glow::RENDERBUFFER, glow::RENDERBUFFER_HEIGHT);
                (rbo, width, height)
            });
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            adopted
        };
        // Either miss costs display-sized layers a full-size stencil of their own,
        // which nothing else in the log shows.
        let Some((rbo, width, height)) = adopted else {
            tracing::warn!(
                "screen target has no stencil renderbuffer to share; \
                 display-sized layers pool one of their own"
            );
            return;
        };
        let Some(size) = PhysicalSize::new(width, height) else {
            tracing::warn!(
                width,
                height,
                "screen stencil size cannot key the pool; leaving it unshared"
            );
            return;
        };
        self.stencil_pool.insert(size, PoolSlot::Borrowed(rbo));
        tracing::debug!(
            width = size.width,
            height = size.height,
            "static layers share the screen stencil"
        );
    }

    /// The renderbuffer attached as the bound framebuffer's stencil, if any.
    ///
    /// # Safety
    /// The GL context must be current and the framebuffer of interest bound.
    pub(super) unsafe fn attached_stencil(&self) -> Option<glow::Renderbuffer> {
        let kind = unsafe {
            self.gl.get_framebuffer_attachment_parameter_i32(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
            )
        };
        if gl_uint(kind) != glow::RENDERBUFFER {
            return None;
        }
        let name = unsafe {
            self.gl.get_framebuffer_attachment_parameter_i32(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
            )
        };
        gl_name(name).map(glow::NativeRenderbuffer)
    }

    /// The stencil renderbuffer shared by static layers of `size`,
    /// allocating one on first use. `None` for a refused size or a failed allocation.
    fn pooled_stencil(&mut self, size: PhysicalSize) -> Option<glow::Renderbuffer> {
        match self.stencil_pool.get(&size) {
            Some(PoolSlot::Owned { rbo, .. } | PoolSlot::Borrowed(rbo)) => return Some(*rbo),
            Some(PoolSlot::Refused { .. }) => return None,
            None => {}
        }
        // SAFETY: the renderer's GL context is current for the whole frame.
        let rbo = unsafe {
            let rbo = self.gl.create_renderbuffer().ok()?;
            self.drain_gl_errors();
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
            self.gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::STENCIL_INDEX8,
                i32::from(size.width.get()),
                i32::from(size.height.get()),
            );
            let storage_error = self.gl.get_error();
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            if storage_error != glow::NO_ERROR {
                // Deliberately not pooled: a renderbuffer without storage
                // fails completeness for every later layer of this size,
                // and the error left queued would surface against an unrelated call.
                self.gl.delete_renderbuffer(rbo);
                tracing::warn!(
                    width = size.width,
                    height = size.height,
                    storage_error,
                    "static-layer stencil allocation failed"
                );
                return None;
            }
            rbo
        };
        self.stencil_pool.insert(
            size,
            PoolSlot::Owned {
                rbo,
                serving: false,
            },
        );
        tracing::debug!(
            width = size.width,
            height = size.height,
            "allocated a shared static-layer stencil"
        );
        Some(rbo)
    }

    /// Put the pooled stencil on the bound layer framebuffer
    /// in place of the one femtovg allocated for it, and shrink femtovg's to 1×1.
    ///
    /// femtovg gives every render-target image a `STENCIL_INDEX8` renderbuffer
    /// of its own, costing a full-slot layer about what its RGBA8 texture does.
    /// Sharing is safe because nothing lives in a stencil between draw commands:
    /// every fill femtovg runs leaves the region it used zeroed.
    ///
    /// # Safety
    /// The GL context must be current and the layer's framebuffer bound:
    /// the swap rewrites the stencil of whichever framebuffer is bound.
    pub(super) unsafe fn share_layer_stencil(&mut self, size: PhysicalSize) {
        // SAFETY: guaranteed by the caller.
        let Some(attached) = (unsafe { self.attached_stencil() }) else {
            return;
        };
        let Some(shared) = self.pooled_stencil(size) else {
            return;
        };
        if attached == shared {
            return;
        }
        let femtovg_rbo = attached;
        // SAFETY: as above.
        unsafe {
            // An incomplete framebuffer would fail the check below
            // for reasons that have nothing to do with the stencil,
            // and refusing the size on its account
            // would give up sharing for every later layer of it.
            let before = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if before != glow::FRAMEBUFFER_COMPLETE {
                tracing::warn!(
                    status = before,
                    width = size.width,
                    height = size.height,
                    "layer framebuffer incomplete before the stencil swap; leaving it alone"
                );
                return;
            }
            self.drain_gl_errors();
            self.gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(shared),
            );
            // A stale name fails the attach but leaves femtovg's stencil attached,
            // which the status check alone would read as complete.
            let attach_error = self.gl.get_error();
            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if attach_error != glow::NO_ERROR || status != glow::FRAMEBUFFER_COMPLETE {
                self.gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(femtovg_rbo),
                );
                self.refuse_stencil_size(size);
                if attach_error == glow::NO_ERROR {
                    tracing::warn!(
                        status,
                        width = size.width,
                        height = size.height,
                        "driver refused a shared layer stencil; \
                         layers of this size keep femtovg's own"
                    );
                } else {
                    tracing::warn!(
                        attach_error,
                        width = size.width,
                        height = size.height,
                        "shared layer stencil failed to attach; \
                         layers of this size keep femtovg's own"
                    );
                }
                return;
            }
            // Shrunk rather than deleted: femtovg deletes this name
            // with the image's framebuffer, and a driver may hand the freed name
            // to a later allocation that the delete would then take out.
            self.gl
                .bind_renderbuffer(glow::RENDERBUFFER, Some(femtovg_rbo));
            self.gl
                .renderbuffer_storage(glow::RENDERBUFFER, glow::STENCIL_INDEX8, 1, 1);
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        }
        if let Some(PoolSlot::Owned { serving, .. }) = self.stencil_pool.get_mut(&size) {
            *serving = true;
        }
    }

    /// Stop offering the pool to layers of `size`.
    pub(super) fn refuse_stencil_size(&mut self, size: PhysicalSize) {
        let kept = match self.stencil_pool.remove(&size) {
            Some(PoolSlot::Owned { rbo, serving: true }) => Some(rbo),
            Some(PoolSlot::Refused { kept }) => kept,
            Some(previous @ (PoolSlot::Owned { serving: false, .. } | PoolSlot::Borrowed(_))) => {
                previous.release(&self.gl);
                None
            }
            None => None,
        };
        self.stencil_pool.insert(size, PoolSlot::Refused { kept });
    }

    /// Delete the pooled stencils this renderer allocated.
    pub(super) fn release_pooled_stencils(&mut self) {
        for (_, slot) in self.stencil_pool.drain() {
            slot.release(&self.gl);
        }
    }

    /// Composite the static layer with [`RawBlit`] instead of femtovg.
    ///
    /// Flushes first: femtovg has the frame's clear queued and raw GL would
    /// otherwise land underneath it. Returns `false` if the program or the
    /// layer's texture is unavailable, so a failure degrades to femtovg's own
    /// blit rather than to a blank frame.
    fn blit_static_layer_raw(&mut self, image: femtovg::ImageId) -> bool {
        let Ok(texture) = self.canvas.get_native_texture(image) else {
            return false;
        };
        self.canvas.flush();
        if self.raw_blit.is_none() {
            self.raw_blit = RawBlit::new(&self.gl);
        }
        let Some(blit) = self.raw_blit.as_ref() else {
            return false;
        };

        // SAFETY: the renderer's GL context is current for the whole frame.
        unsafe {
            self.gl.disable(glow::BLEND);
            self.gl.disable(glow::STENCIL_TEST);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.use_program(Some(blit.program));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            if let Some(vao) = blit.vao {
                self.gl.bind_vertex_array(Some(vao));
            }
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(blit.vbo));
            let stride = 4 * F32_BYTES;
            self.gl.enable_vertex_attrib_array(blit.pos_loc);
            self.gl
                .vertex_attrib_pointer_f32(blit.pos_loc, 2, glow::FLOAT, false, stride, 0);
            self.gl.enable_vertex_attrib_array(blit.uv_loc);
            self.gl.vertex_attrib_pointer_f32(
                blit.uv_loc,
                2,
                glow::FLOAT,
                false,
                stride,
                2 * F32_BYTES,
            );
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            self.gl.disable_vertex_attrib_array(blit.pos_loc);
            self.gl.disable_vertex_attrib_array(blit.uv_loc);
            if blit.vao.is_some() {
                self.gl.bind_vertex_array(None);
            }
        }
        true
    }

    /// Run `inner` translated to the canvas origin `(cx, cy)`.
    /// Used by the `drop_shadow` fallbacks,
    /// since the closure draws at FBO-local `(0, 0)`.
    fn render_inner_translated(
        &mut self,
        cx: f32,
        cy: f32,
        inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
        self.canvas.save();
        self.canvas.translate(cx, cy);
        inner(self);
        self.canvas.restore();
    }
}

/// Offscreen textures for `drop_shadow`, kept alive across frames
/// so the two textures aren't reallocated every frame;
/// resized only on a size change.
pub(super) struct ShadowFboPool {
    width: u32,
    height: u32,
    pub(super) unblurred: femtovg::ImageId,
    pub(super) blurred: femtovg::ImageId,
}

/// A textured-quad program: two triangles, one `texture2D`, nothing else.
///
/// Compositing the static layer is a straight copy, and femtovg's own fast path
/// picks a costlier shader for it. What remains is dominated by the write
/// itself, so the way past it is drawing fewer pixels, not cheaper ones.
pub(super) struct RawBlit {
    pub(super) program: glow::Program,
    pub(super) vbo: glow::Buffer,
    /// Required on desktop GL core profile and ES 3.0+, where there is no
    /// default vertex array to set attributes on. Only an extension on ES 2.0,
    /// so it is tried and skipped when unavailable.
    pub(super) vao: Option<glow::VertexArray>,
    pub(super) pos_loc: u32,
    pub(super) uv_loc: u32,
}

const F32_BYTES: i32 = 4;

impl RawBlit {
    const VERT: &'static str = r"#version 100
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
";

    const FRAG: &'static str = r"#version 100
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}
";

    pub(super) fn new(gl: &glow::Context) -> Option<Self> {
        // SAFETY: the caller holds the renderer's current GL context.
        unsafe {
            let program = gl.create_program().ok()?;
            for (kind, src) in [
                (glow::VERTEX_SHADER, Self::VERT),
                (glow::FRAGMENT_SHADER, Self::FRAG),
            ] {
                let Ok(shader) = gl.create_shader(kind) else {
                    gl.delete_program(program);
                    return None;
                };
                gl.shader_source(shader, src);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    tracing::warn!(log = %gl.get_shader_info_log(shader), "raw blit shader");
                    gl.delete_shader(shader);
                    gl.delete_program(program);
                    return None;
                }
                gl.attach_shader(program, shader);
                gl.delete_shader(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                gl.delete_program(program);
                return None;
            }

            // No Y flip: layer and frame are both GL framebuffers with a
            // bottom-left origin, so a straight copy keeps orientation.
            // femtovg's paint needs `ImageFlags::FLIP_Y` only because its image
            // sampling applies a convention of its own.
            let verts: [f32; 16] = [
                -1.0, -1.0, 0.0, 0.0, //
                1.0, -1.0, 1.0, 0.0, //
                -1.0, 1.0, 0.0, 1.0, //
                1.0, 1.0, 1.0, 1.0,
            ];
            let vao = gl.create_vertex_array().ok();
            if let Some(vao) = vao {
                gl.bind_vertex_array(Some(vao));
            }

            let Ok(vbo) = gl.create_buffer() else {
                Self::discard(gl, program, None, vao);
                return None;
            };
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let bytes: Vec<u8> = verts.iter().flat_map(|v| v.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);

            if vao.is_some() {
                gl.bind_vertex_array(None);
            }

            let (Some(pos_loc), Some(uv_loc)) = (
                gl.get_attrib_location(program, "a_pos"),
                gl.get_attrib_location(program, "a_uv"),
            ) else {
                Self::discard(gl, program, Some(vbo), vao);
                return None;
            };

            Some(Self {
                program,
                vbo,
                vao,
                pos_loc,
                uv_loc,
            })
        }
    }

    /// Release what a failed [`Self::new`] had already allocated, and leave
    /// no vertex array bound behind for whatever draws next.
    ///
    /// # Safety
    ///
    /// The caller's GL context must be current.
    unsafe fn discard(
        gl: &glow::Context,
        program: glow::Program,
        vbo: Option<glow::Buffer>,
        vao: Option<glow::VertexArray>,
    ) {
        unsafe {
            if let Some(vao) = vao {
                gl.bind_vertex_array(None);
                gl.delete_vertex_array(vao);
            }
            if let Some(vbo) = vbo {
                gl.delete_buffer(vbo);
            }
            gl.delete_program(program);
        }
    }
}

/// Cached rasterisation of the static half of a widget tree.
///
/// Geometry is recorded so a resize can be detected: the texture is the wrong
/// shape then and must be reallocated rather than reused.
pub(super) struct StaticLayer {
    pub(super) image: femtovg::ImageId,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) dpi_scale: f32,
    /// `false` between allocation and the first completed capture. Blitting an
    /// uncaptured layer would paint a transparent rectangle over the frame.
    pub(super) captured: bool,
}

/// A static layer's size in physical pixels, which also keys its pooled stencil.
///
/// `NonZeroU16` because every consumer widens it without a check —
/// GL's signed `GLsizei` as well as femtovg's `usize` and `u32` —
/// so the only fallible conversion is the one into this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PhysicalSize {
    pub(super) width: NonZeroU16,
    pub(super) height: NonZeroU16,
}

impl PhysicalSize {
    /// `None` for a size no layer can have: zero, or beyond `u16`.
    pub(super) fn new<T>(width: T, height: T) -> Option<Self>
    where
        u16: TryFrom<T>,
    {
        Some(Self {
            width: NonZeroU16::new(u16::try_from(width).ok()?)?,
            height: NonZeroU16::new(u16::try_from(height).ok()?)?,
        })
    }
}

/// What [`FemtoVgRenderer::stencil_pool`] holds for one layer size.
pub(super) enum PoolSlot {
    /// A shared stencil this renderer allocated, and so deletes.
    Owned {
        rbo: glow::Renderbuffer,
        /// Set once a layer framebuffer has it attached.
        serving: bool,
    },
    /// The screen target's own stencil, which the embedder allocated and frees.
    Borrowed(glow::Renderbuffer),
    /// The shared stencil failed to attach,
    /// or the driver called a layer framebuffer incomplete with it attached,
    /// so layers of this size captured from now on keep femtovg's own.
    Refused {
        /// An owned stencil that layers captured before the refusal still draw through.
        /// It lives until the renderer goes:
        /// GLES 2.0 leaves a deleted renderbuffer on an unbound framebuffer undefined.
        kept: Option<glow::Renderbuffer>,
    },
}

impl PoolSlot {
    fn release(self, gl: &glow::Context) {
        if let Self::Owned { rbo, .. } | Self::Refused { kept: Some(rbo) } = self {
            // SAFETY: the renderer only releases on its own current context.
            unsafe { gl.delete_renderbuffer(rbo) };
        }
    }
}

/// Outcome of [`FemtoVgRenderer::probe_render_to_texture`].
///
/// Only [`Self::Working`] means a cached static layer is viable; the rest name
/// distinct failures so a device log says *why*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTargetProbe {
    /// RGBA8 offscreen target with an S8 stencil,
    /// allocated and framebuffer-complete.
    Working,
    /// femtovg could not allocate the image at all.
    ImageAllocFailed,
    /// Image allocated but exposes no GL texture to attach.
    NoNativeTexture,
    /// The driver refused to create a framebuffer or renderbuffer object.
    FramebufferAllocFailed,
    /// Attached, but the driver reports the framebuffer incomplete.
    FramebufferIncomplete(u32),
    /// GL raised an error during the probe.
    GlError(u32),
}

/// An unsigned value — a `GLuint` name or a `GLenum` —
/// that a GL query returns through a `GLint`.
///
/// The bits are reinterpreted rather than range-checked:
/// a value past `i32::MAX` arrives negative. glow's own name getters do the same.
#[expect(
    clippy::cast_sign_loss,
    reason = "GL returns unsigned values through a GLint"
)]
fn gl_uint(value: i32) -> u32 {
    value as u32
}

/// The object name a GL query returns through a `GLint`; zero is "no object".
pub(super) fn gl_name(value: i32) -> Option<NonZeroU32> {
    NonZeroU32::new(gl_uint(value))
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-int `as` saturates, and `PhysicalSize::new` rejects the extremes"
)]
fn physical_size(width: u32, height: u32, dpi_scale: f32) -> Option<PhysicalSize> {
    PhysicalSize::new(
        (width as f32 * dpi_scale) as u32,
        (height as f32 * dpi_scale) as u32,
    )
}

impl RenderOffscreen for FemtoVgRenderer {
    // -- Drop shadow --

    fn drop_shadow(
        &mut self,
        cx: f32,
        cy: f32,
        fbo_w: u32,
        fbo_h: u32,
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
        if fbo_w == 0 || fbo_h == 0 {
            self.render_inner_translated(cx, cy, inner);
            return;
        }

        // Physical-pixel FBO size, so the offscreen pass matches screen DPI
        // and the composite doesn't upscale a half-resolution buffer.
        let dpi = self.dpi_scale.max(1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "FBO dimensions are small positive pixel counts after ceil()"
        )]
        let (pw, ph) = (
            ((fbo_w as f32) * dpi).ceil() as u32,
            ((fbo_h as f32) * dpi).ceil() as u32,
        );

        let Some((unblurred, blurred)) = self.acquire_shadow_fbos(pw, ph) else {
            tracing::warn!("drop_shadow: FBO pool allocation failed; falling back to no shadow");
            self.render_inner_translated(cx, cy, inner);
            return;
        };

        // Rasterise the inner draw into `unblurred` at FBO-local coordinates.
        // `reset` drops transform and scissor, and the scale has to go back on:
        // the FBO is sized in physical pixels while the inner draw is logical.
        self.canvas.save();
        self.canvas.reset();
        self.scale_to_logical(dpi);
        self.canvas
            .set_render_target(RenderTarget::Image(unblurred));
        self.canvas
            .clear_rect(0, 0, pw, ph, femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0));

        inner(self);

        // Sigma is logical pixels; scale to physical.
        // Clamp mirrors the SDK cap so a malformed
        // wire value can't request an unbounded blur.
        let sigma = blur.clamp(0.0, bmc_wasm_protocol::DROP_SHADOW_BLUR_MAX) * dpi;
        if sigma > 0.0 {
            self.canvas.filter_image(
                blurred,
                femtovg::ImageFilter::GaussianBlur { sigma },
                unblurred,
            );
        }

        // No mid-frame flush: femtovg replays its queue in submission order
        // and the target switch is an ordering barrier, so the composites
        // below read fully-rendered textures. That same ordering lets one
        // pooled pair serve every shadow in a frame — draw N's composites
        // are queued before draw N+1's `clear_rect`. A flush here,
        // or out-of-order shadow rendering, would break that.
        // Back to whatever the frame draws into rather than `Screen`.
        // Under `begin_frame_to_image` that is a stage's image,
        // and assuming the window strands everything queued after this shadow.
        self.canvas.set_render_target(self.frame_target);
        self.canvas.restore();

        let composite_src = if sigma > 0.0 { blurred } else { unblurred };
        // Not dimmed: the tint multiplies the inner draw, which already was.
        let tint = to_femtovg_color(color.to_u32());
        let fbo_w_f = fbo_w as f32;
        let fbo_h_f = fbo_h as f32;

        // Each composite flips Y so the GL FBO's bottom-left-origin texture
        // lands right-side-up; paint and path anchor at (0, 0) in flipped space.
        self.canvas.save();
        self.canvas.translate(cx + dx, cy + dy + fbo_h_f);
        self.canvas.scale(1.0, -1.0);
        let shadow_paint = Paint::image_tint(composite_src, 0.0, 0.0, fbo_w_f, fbo_h_f, 0.0, tint);
        let mut shadow_path = Path::new();
        shadow_path.rect(0.0, 0.0, fbo_w_f, fbo_h_f);
        self.canvas.fill_path(&shadow_path, &shadow_paint);
        self.canvas.restore();

        self.canvas.save();
        self.canvas.translate(cx, cy + fbo_h_f);
        self.canvas.scale(1.0, -1.0);
        let asset_paint = Paint::image(unblurred, 0.0, 0.0, fbo_w_f, fbo_h_f, 0.0, 1.0);
        let mut asset_path = Path::new();
        asset_path.rect(0.0, 0.0, fbo_w_f, fbo_h_f);
        self.canvas.fill_path(&asset_path, &asset_paint);
        self.canvas.restore();
    }

    // -- Static-layer cache --

    fn begin_static_layer(&mut self, key: &str, width: u32, height: u32) -> bool {
        if self.render_to_texture != RenderTargetProbe::Working {
            return false;
        }
        let dpi_scale = self.dpi_scale;
        let Some(size) = physical_size(width, height, dpi_scale) else {
            // No layer can have this size, so whatever the key held is stale.
            self.invalidate_static_layer(key);
            return false;
        };
        if self.incomplete_static_layers.get(key) == Some(&size) {
            // A layer captured at another size will never be blitted again.
            self.invalidate_static_layer(key);
            return false;
        }
        let first_retarget = !self.static_layers.get(key).is_some_and(|layer| {
            layer.width == width
                && layer.height == height
                && (layer.dpi_scale - dpi_scale).abs() < f32::EPSILON
        });
        if first_retarget {
            // A geometry change makes the existing texture the wrong shape.
            self.invalidate_static_layer(key);
            // FLIP_Y, not a canvas transform: femtovg's cheap
            // `is_straight_tinted_image` path ignores the canvas transform,
            // while `FLIP_Y` folds into the paint's inverse transform and so
            // survives it.
            let Ok(image) = self.canvas.create_image_empty(
                usize::from(size.width.get()),
                usize::from(size.height.get()),
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::FLIP_Y,
            ) else {
                return false;
            };
            self.static_layers.insert(
                key.to_owned(),
                StaticLayer {
                    image,
                    width,
                    height,
                    dpi_scale,
                    captured: false,
                },
            );
        }
        let Some(layer) = self.static_layers.get(key) else {
            return false;
        };
        let image = layer.image;
        // Deliberately not `begin_frame_to_image`: that opens a new frame on
        // the paragraph cache, and a capture is a second pass over the frame
        // already in progress rather than a new one — the double step would
        // charge this frame's shaping twice over. Canvas size and dpi are
        // already the frame's, so only the target and the clear change here.
        self.canvas.set_render_target(RenderTarget::Image(image));

        // femtovg reports no failed retarget,
        // and a skipped one leaves the caller's export buffer bound.
        // The static pass would then paint over the frame,
        // `end_static_layer` would mark the layer captured anyway,
        // and every later blit would copy an uninitialised texture over the widget.
        // Ask GL what is bound instead of trusting the call.
        //
        // The flush is what makes the question answerable: `set_render_target`
        // only queues the command, so without it GL still reports the frame's
        // own target and the check reads as a failure on every capture.
        self.canvas.flush();
        // SAFETY: the renderer's GL context is current for the whole frame.
        let bound = unsafe { self.gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING) };
        // femtovg never renders a layer into the default framebuffer,
        // so finding that bound is as much a failed retarget as finding the screen's.
        if bound.is_none_or(|fbo| Some(fbo) == self.screen_fbo) {
            tracing::error!(
                key,
                "static layer retarget failed; falling back to a full pass"
            );
            self.canvas.set_render_target(RenderTarget::Screen);
            self.invalidate_static_layer(key);
            return false;
        }
        // A non-screen binding is not yet a usable one:
        // a failing `Framebuffer::new` leaves its incomplete framebuffer bound.
        // The probe has settled the format, so what fails here is this layer,
        // most likely its size; it is refused at that size rather than retried every frame.
        // SAFETY: the renderer's GL context is current for the whole frame.
        let status = unsafe { self.gl.check_framebuffer_status(glow::FRAMEBUFFER) };
        if status != glow::FRAMEBUFFER_COMPLETE {
            tracing::error!(
                key,
                status,
                width,
                height,
                "static layer framebuffer incomplete; not caching it at this size"
            );
            self.incomplete_static_layers.insert(key.to_owned(), size);
            self.canvas.set_render_target(RenderTarget::Screen);
            // SAFETY: the context is current for the frame,
            // and on a first retarget the framebuffer `Framebuffer::new` failed on is still bound.
            unsafe {
                if first_retarget {
                    self.delete_orphaned_framebuffer();
                }
                // Rebind now rather than at the next flush,
                // so raw passes before it do not draw into the incomplete framebuffer.
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.screen_fbo);
                // The failed allocation leaves its error queued,
                // where a later `get_error` would pin it on an unrelated draw.
                self.drain_gl_errors();
            }
            self.invalidate_static_layer(key);
            return false;
        }
        // femtovg builds the layer's framebuffer during the retarget above,
        // so the swap has nothing to act on any earlier than here.
        // SAFETY: the context is current for the frame, and the check above
        // found the layer's framebuffer bound.
        unsafe { self.share_layer_stencil(size) };
        // `drop_shadow` restores `frame_target` after its offscreen pass, so the
        // capture has to own it: a shadowed *static* draw sits in `Band::Below`
        // and is emitted here, and leaving this on `Screen` would send that
        // shadow — and every static draw queued after it — to the export buffer
        // while the layer is still marked captured.
        debug_assert!(
            matches!(self.frame_target, RenderTarget::Screen),
            "a static-layer capture may only borrow a screen-backed frame; \
             `end_static_layer` hands the target back as `Screen`",
        );
        self.frame_target = RenderTarget::Image(image);

        // Opaque, because the frame itself starts opaque black: rasterising
        // static content against transparency and compositing it later blends
        // those edges twice, which shows as antialiasing drift on thin geometry
        // and text. Opaque makes the blit a copy rather than a blend.
        self.canvas.clear_rect(
            0,
            0,
            u32::from(size.width.get()),
            u32::from(size.height.get()),
            femtovg::Color::rgbf(0.0, 0.0, 0.0),
        );
        true
    }

    fn end_static_layer(&mut self, key: &str) {
        // The capture is still only queued, and femtovg binds a target when it
        // *executes* a command rather than at flush start. Draining here, while
        // the layer's framebuffer is still bound, is what puts the static half
        // into the layer; without it the rebind below sends the whole capture
        // to the screen and leaves the layer undefined.
        self.canvas.flush();
        self.frame_target = RenderTarget::Screen;
        self.canvas.set_render_target(RenderTarget::Screen);
        // Keep GL agreeing with the retarget just queued, so raw passes before
        // the next flush do not land in the layer.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.screen_fbo);
        }
        if let Some(layer) = self.static_layers.get_mut(key) {
            layer.captured = true;
        }
    }

    fn blit_static_layer(&mut self, key: &str) -> bool {
        let Some(layer) = self.static_layers.get(key) else {
            return false;
        };
        if !layer.captured {
            return false;
        }
        let (image, w, h) = (layer.image, layer.width as f32, layer.height as f32);
        if self.blit_static_layer_raw(image) {
            return true;
        }

        // No canvas transform: the fast path below ignores it, and the Y-flip
        // comes from the `ImageFlags::FLIP_Y` set at allocation.
        self.canvas.save();
        let paint = Paint::image(image, 0.0, 0.0, w, h, 0.0, 1.0).with_anti_alias(false);
        let mut path = Path::new();
        path.rect(0.0, 0.0, w, h);
        self.canvas.fill_path(&path, &paint);
        self.canvas.restore();
        true
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "damage rectangles are small positive values within the surface"
    )]
    fn blit_static_layer_rects(&mut self, key: &str, rects: &[Rect]) -> bool {
        if rects.is_empty() {
            return false;
        }
        let Some(layer) = self.static_layers.get(key) else {
            return false;
        };
        if !layer.captured {
            return false;
        }
        let image = layer.image;
        let Ok(texture) = self.canvas.get_native_texture(image) else {
            return false;
        };
        self.canvas.flush();
        if self.raw_blit.is_none() {
            self.raw_blit = RawBlit::new(&self.gl);
        }
        let Some(blit) = self.raw_blit.as_ref() else {
            return false;
        };

        let dpi = self.dpi_scale;
        let (width_px, height_px) = ((self.width * dpi) as i32, (self.height * dpi) as i32);
        // SAFETY: the renderer's GL context is current for the whole frame.
        unsafe {
            self.gl.disable(glow::BLEND);
            self.gl.disable(glow::STENCIL_TEST);
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl.use_program(Some(blit.program));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            if let Some(vao) = blit.vao {
                self.gl.bind_vertex_array(Some(vao));
            }
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(blit.vbo));
            let stride = 4 * F32_BYTES;
            self.gl.enable_vertex_attrib_array(blit.pos_loc);
            self.gl
                .vertex_attrib_pointer_f32(blit.pos_loc, 2, glow::FLOAT, false, stride, 0);
            self.gl.enable_vertex_attrib_array(blit.uv_loc);
            self.gl.vertex_attrib_pointer_f32(
                blit.uv_loc,
                2,
                glow::FLOAT,
                false,
                stride,
                2 * F32_BYTES,
            );

            for &rect in rects {
                // The quad covers the whole surface and the scissor restricts
                // what is written, so cost tracks damage area, not rect count.
                let (px, py, pw, ph) = scissor_box(rect, dpi, width_px, height_px);
                self.gl.scissor(px, py, pw, ph);
                self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }

            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.disable_vertex_attrib_array(blit.pos_loc);
            self.gl.disable_vertex_attrib_array(blit.uv_loc);
            if blit.vao.is_some() {
                self.gl.bind_vertex_array(None);
            }
        }
        true
    }

    fn invalidate_static_layer(&mut self, key: &str) {
        if let Some(layer) = self.static_layers.remove(key) {
            self.canvas.delete_image(layer.image);
        }
    }
}

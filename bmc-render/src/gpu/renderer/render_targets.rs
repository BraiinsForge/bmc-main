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

use super::FemtoVgRenderer;
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
    /// Like [`crate::renderer::Renderer::begin_frame`] but renders to the given Image instead of
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
    pub(super) fn acquire_shadow_fbos(
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
    pub(super) fn blit_static_layer_raw(&mut self, image: femtovg::ImageId) -> bool {
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

pub(super) const F32_BYTES: i32 = 4;

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
pub(super) fn physical_size(width: u32, height: u32, dpi_scale: f32) -> Option<PhysicalSize> {
    PhysicalSize::new(
        (width as f32 * dpi_scale) as u32,
        (height as f32 * dpi_scale) as u32,
    )
}

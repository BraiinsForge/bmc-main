// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Two-FBO EGL rendering pipeline for FemtoVG widgets.
//!
//! FemtoVG renders Y-flipped when targeting an FBO. This module wraps
//! [`bmc_widget::egl`] with a staging FBO (regular GL texture + stencil)
//! and a Y-flip blit pass to the DMA-BUF export buffer.
//!
//! Pipeline: FemtoVG → staging FBO → blit (Y-flip) → export FBO → DMA-BUF

use anyhow::{Context, Result};
use glow::HasContext;

pub use bmc_widget::egl::DmaBufInfo;
use bmc_widget::egl::{EglContext, ExportBuffer};

/// Staging buffer for FemtoVG rendering (regular GL texture, not EGLImage).
struct StagingBuffer {
    texture: glow::Texture,
    fbo: glow::Framebuffer,
    stencil_rbo: glow::Renderbuffer,
}

/// Resources for the Y-flip blit pass.
struct BlitResources {
    program: glow::Program,
    vbo: glow::Buffer,
    pos_loc: u32,
    uv_loc: u32,
}

/// Two-FBO EGL state for FemtoVG rendering with Y-flip correction.
///
/// Double-buffers two [`ExportBuffer`]s. Each frame:
/// 1. Bind staging FBO (FemtoVG renders here with stencil)
/// 2. `blit_to_export()` copies staging → export with flipped V
/// 3. `end_frame()` calls `gl.flush()` and exports DMA-BUF
pub struct EglState {
    ctx: EglContext,
    buffers: [Option<ExportBuffer>; 2],
    current_buffer: usize,
    staging: Option<StagingBuffer>,
    blit_program: Option<BlitResources>,
    width: u32,
    height: u32,
}

impl EglState {
    /// Create EGL context and prepare for two-FBO rendering.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let ctx = EglContext::new()?;
        Ok(Self {
            ctx,
            buffers: [None, None],
            current_buffer: 0,
            staging: None,
            blit_program: None,
            width,
            height,
        })
    }

    /// Begin a frame — allocate resources if needed, bind the staging FBO.
    ///
    /// Returns the raw GL framebuffer name of the staging FBO for FemtoVG's
    /// `set_screen_target`. Call `blit_to_export()` after FemtoVG flushes.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn begin_frame(&mut self) -> Result<u32> {
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(self.ctx.allocate_export_buffer(self.width, self.height)?);
        }

        if self.staging.is_none() {
            self.staging = Some(self.allocate_staging()?);
        }

        if self.blit_program.is_none() {
            self.blit_program = Some(self.create_blit_resources()?);
        }

        let staging = self
            .staging
            .as_ref()
            .expect("BUG: staging should exist after allocation");

        unsafe {
            let gl = self.ctx.gl();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(staging.fbo));
            gl.viewport(0, 0, self.width as i32, self.height as i32);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
        }

        Ok(staging.fbo.0.get())
    }

    /// Blit staging FBO → export FBO with Y-flip.
    ///
    /// Call after FemtoVG `flush()` and before `end_frame()`.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn blit_to_export(&self) -> Result<()> {
        let staging = self
            .staging
            .as_ref()
            .context("BUG: staging not allocated")?;
        let blit = self
            .blit_program
            .as_ref()
            .context("BUG: blit resources not allocated")?;
        let export = self.buffers[self.current_buffer]
            .as_ref()
            .context("BUG: export buffer not allocated")?;

        unsafe {
            let gl = self.ctx.gl();

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(export.fbo));
            gl.viewport(0, 0, self.width as i32, self.height as i32);

            gl.use_program(Some(blit.program));

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(staging.texture));

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(blit.vbo));
            gl.enable_vertex_attrib_array(blit.pos_loc);
            gl.vertex_attrib_pointer_f32(blit.pos_loc, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(blit.uv_loc);
            gl.vertex_attrib_pointer_f32(blit.uv_loc, 2, glow::FLOAT, false, 16, 8);

            gl.disable(glow::STENCIL_TEST);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            gl.disable_vertex_attrib_array(blit.pos_loc);
            gl.disable_vertex_attrib_array(blit.uv_loc);
        }

        Ok(())
    }

    /// End frame — `gl.flush()`, export DMA-BUF, swap buffers.
    pub fn end_frame(&mut self) -> Result<DmaBufInfo> {
        unsafe {
            self.ctx.gl().flush();
        }

        let idx = self.current_buffer;
        let buf = self.buffers[idx]
            .as_mut()
            .expect("BUG: buffer should exist after begin_frame");

        let info = EglContext::export_dmabuf(buf)?;
        self.current_buffer = 1 - self.current_buffer;
        Ok(info)
    }

    /// Resize — deallocate buffers so they're reallocated at the new size.
    #[expect(dead_code)]
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }

        tracing::debug!(
            "Resizing from {}x{} to {}x{}",
            self.width,
            self.height,
            width,
            height
        );

        for buffer in &mut self.buffers {
            if let Some(buf) = buffer.take() {
                self.ctx.destroy_export_buffer(buf);
            }
        }

        if let Some(ref staging) = self.staging.take() {
            Self::destroy_staging(self.ctx.gl(), staging);
        }

        self.width = width;
        self.height = height;
    }

    /// Get the glow GL context (for test mode FemtoVG renderer and GL loaders).
    #[expect(
        dead_code,
        reason = "used by test mode which is re-added in a later commit"
    )]
    pub fn gl(&self) -> &glow::Context {
        self.ctx.gl()
    }

    /// Re-export `get_proc_address` for GL loaders (e.g. `bmc-wasm-runtime`).
    pub fn get_proc_address(symbol: &str) -> *const std::ffi::c_void {
        EglContext::get_proc_address(symbol)
    }

    // -- Private helpers --

    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    fn allocate_staging(&self) -> Result<StagingBuffer> {
        let gl = self.ctx.gl();

        let texture = unsafe {
            let tex = gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("Failed to create staging texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                self.width as i32,
                self.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
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
            tex
        };

        let stencil_rbo = unsafe {
            let rbo = gl
                .create_renderbuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create stencil RBO: {e}"))?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::STENCIL_INDEX8,
                self.width as i32,
                self.height as i32,
            );
            rbo
        };

        let fbo = unsafe {
            let fbo = gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create staging FBO: {e}"))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(stencil_rbo),
            );

            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Staging framebuffer incomplete: 0x{status:x}");
            }
            fbo
        };

        Ok(StagingBuffer {
            texture,
            fbo,
            stencil_rbo,
        })
    }

    fn create_blit_resources(&self) -> Result<BlitResources> {
        let gl = self.ctx.gl();

        let vert_src = r"#version 100
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
";
        let frag_src = r"#version 100
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}
";
        let program = unsafe {
            let vs = gl
                .create_shader(glow::VERTEX_SHADER)
                .map_err(|e| anyhow::anyhow!("Blit VS create: {e}"))?;
            gl.shader_source(vs, vert_src);
            gl.compile_shader(vs);
            if !gl.get_shader_compile_status(vs) {
                let log = gl.get_shader_info_log(vs);
                anyhow::bail!("Blit VS compile: {log}");
            }

            let fs = gl
                .create_shader(glow::FRAGMENT_SHADER)
                .map_err(|e| anyhow::anyhow!("Blit FS create: {e}"))?;
            gl.shader_source(fs, frag_src);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                let log = gl.get_shader_info_log(fs);
                anyhow::bail!("Blit FS compile: {log}");
            }

            let prog = gl
                .create_program()
                .map_err(|e| anyhow::anyhow!("Blit program create: {e}"))?;
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            gl.link_program(prog);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(prog) {
                let log = gl.get_program_info_log(prog);
                anyhow::bail!("Blit program link: {log}");
            }
            prog
        };

        // Fullscreen quad with Y-flipped UVs
        #[rustfmt::skip]
        let vertices: [f32; 24] = [
            -1.0, -1.0,  0.0, 1.0,
             1.0, -1.0,  1.0, 1.0,
             1.0,  1.0,  1.0, 0.0,
            -1.0, -1.0,  0.0, 1.0,
             1.0,  1.0,  1.0, 0.0,
            -1.0,  1.0,  0.0, 0.0,
        ];

        let vbo = unsafe {
            let buf = gl
                .create_buffer()
                .map_err(|e| anyhow::anyhow!("Blit VBO create: {e}"))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            let bytes: &[u8] = std::slice::from_raw_parts(
                vertices.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&vertices),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            buf
        };

        let pos_loc = unsafe {
            gl.get_attrib_location(program, "a_pos")
                .expect("BUG: a_pos not found in blit shader")
        };
        let uv_loc = unsafe {
            gl.get_attrib_location(program, "a_uv")
                .expect("BUG: a_uv not found in blit shader")
        };

        Ok(BlitResources {
            program,
            vbo,
            pos_loc,
            uv_loc,
        })
    }

    fn destroy_staging(gl: &glow::Context, staging: &StagingBuffer) {
        unsafe {
            gl.delete_framebuffer(staging.fbo);
            gl.delete_renderbuffer(staging.stencil_rbo);
            gl.delete_texture(staging.texture);
        }
    }

    fn destroy_blit(gl: &glow::Context, blit: &BlitResources) {
        unsafe {
            gl.delete_program(blit.program);
            gl.delete_buffer(blit.vbo);
        }
    }
}

impl Drop for EglState {
    fn drop(&mut self) {
        let gl = self.ctx.gl();
        for buffer in &mut self.buffers {
            if let Some(buf) = buffer.take() {
                self.ctx.destroy_export_buffer(buf);
            }
        }
        if let Some(ref staging) = self.staging.take() {
            Self::destroy_staging(gl, staging);
        }
        if let Some(ref blit) = self.blit_program.take() {
            Self::destroy_blit(gl, blit);
        }
    }
}

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
use bmc_widget::egl::{Depth, DoubleBufferedEglState, EglContext, WidgetExportBuffer};

/// Resources for the Y-flip blit pass.
struct BlitResources {
    program: glow::Program,
    vbo: glow::Buffer,
    pos_loc: u32,
    uv_loc: u32,
}

/// Two-FBO EGL state for FemtoVG rendering with Y-flip correction.
///
/// Uses [`DoubleBufferedEglState`] for export buffer management. Each frame:
/// 1. Bind staging FBO (FemtoVG renders here with stencil)
/// 2. `blit_to_export()` copies staging → export with flipped V
/// 3. `end_frame()` calls `gl.flush()` and exports DMA-BUF
pub struct EglState {
    egl: DoubleBufferedEglState,
    staging: Option<WidgetExportBuffer>,
    blit_program: Option<BlitResources>,
}

impl EglState {
    /// Create EGL context and prepare for two-FBO rendering.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            egl: DoubleBufferedEglState::new(width, height, Depth::Disabled)?,
            staging: None,
            blit_program: None,
        })
    }

    /// Begin a frame — allocate resources if needed, bind the staging FBO.
    ///
    /// Returns the raw GL framebuffer name of the staging FBO for FemtoVG's
    /// `set_screen_target`. Call `blit_to_export()` after FemtoVG flushes.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn begin_frame(&mut self) -> Result<u32> {
        self.egl.ensure_current()?;

        if self.staging.is_none() {
            self.staging = Some(self.egl.ctx().allocate_widget_export_buffer(
                self.egl.width(),
                self.egl.height(),
                Depth::Disabled,
            )?);
        }

        if self.blit_program.is_none() {
            self.blit_program = Some(self.create_blit_resources()?);
        }

        let staging = self
            .staging
            .as_ref()
            .expect("BUG: staging should exist after allocation");

        let (w, h) = (self.egl.width(), self.egl.height());

        unsafe {
            let gl = self.egl.gl();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(staging.fbo()));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
        }

        Ok(staging.fbo_id())
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
        let export = self
            .egl
            .current_ref()
            .context("BUG: export buffer not allocated")?;

        let (w, h) = (self.egl.width(), self.egl.height());

        unsafe {
            let gl = self.egl.gl();

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(export.fbo));
            gl.viewport(0, 0, w as i32, h as i32);

            gl.use_program(Some(blit.program));

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(staging.texture()));

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
    ///
    /// Returns the DMA-BUF info and the slot index of the exported buffer.
    pub fn end_frame(&mut self) -> Result<(DmaBufInfo, usize)> {
        unsafe {
            self.egl.gl().flush();
        }
        self.egl.export_and_swap()
    }

    /// Resize — deallocate buffers so they're reallocated at the new size.
    #[expect(
        dead_code,
        reason = "resize support for when protocol adds resize events"
    )]
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.egl.width() == width && self.egl.height() == height {
            return;
        }

        tracing::debug!(
            "Resizing from {}x{} to {}x{}",
            self.egl.width(),
            self.egl.height(),
            width,
            height
        );

        if let Some(staging) = self.staging.take() {
            self.egl.ctx().destroy_widget_export_buffer(staging);
        }

        self.egl.resize(width, height);
    }

    /// Get the glow GL context (for test mode FemtoVG renderer and GL loaders).
    #[expect(
        dead_code,
        reason = "used by test mode which is re-added in a later commit"
    )]
    pub fn gl(&self) -> &glow::Context {
        self.egl.gl()
    }

    /// Re-export `get_proc_address` for GL loaders (e.g. `bmc-wasm-runtime`).
    pub fn get_proc_address(symbol: &str) -> *const std::ffi::c_void {
        EglContext::get_proc_address(symbol)
    }

    // -- Private helpers --

    fn create_blit_resources(&self) -> Result<BlitResources> {
        let gl = self.egl.gl();

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
                gl.delete_shader(vs);
                anyhow::bail!("Blit VS compile: {log}");
            }

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| {
                gl.delete_shader(vs);
                anyhow::anyhow!("Blit FS create: {e}")
            })?;
            gl.shader_source(fs, frag_src);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                let log = gl.get_shader_info_log(fs);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                anyhow::bail!("Blit FS compile: {log}");
            }

            let prog = gl.create_program().map_err(|e| {
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                anyhow::anyhow!("Blit program create: {e}")
            })?;
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            gl.link_program(prog);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(prog) {
                let log = gl.get_program_info_log(prog);
                gl.delete_program(prog);
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

    fn destroy_blit(gl: &glow::Context, blit: &BlitResources) {
        unsafe {
            gl.delete_program(blit.program);
            gl.delete_buffer(blit.vbo);
        }
    }
}

impl Drop for EglState {
    fn drop(&mut self) {
        if let Some(staging) = self.staging.take() {
            self.egl.ctx().destroy_widget_export_buffer(staging);
        }
        if let Some(ref blit) = self.blit_program.take() {
            Self::destroy_blit(self.egl.gl(), blit);
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! EGL rendering pipeline for flip-clock (direct FBO, no staging).
//!
//! Thin wrapper around [`bmc_widget::egl`] that provides the flip-clock's
//! direct-FBO pipeline: render straight to an EGLImage-backed export buffer,
//! `glFinish()`, then export as DMA-BUF.

use anyhow::Result;
use glow::HasContext;

pub use bmc_widget::egl::DmaBufInfo;
use bmc_widget::egl::{Depth, DoubleBufferedEglState};

/// EGL state for the flip-clock's direct-FBO rendering pipeline.
///
/// Double-buffers two export buffers via [`DoubleBufferedEglState`]. Each
/// frame:
/// bind the back buffer's FBO, render, `glFinish()`, export DMA-BUF, swap.
pub struct EglState {
    egl: DoubleBufferedEglState,
}

impl EglState {
    /// Create EGL context and prepare for rendering at the given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            egl: DoubleBufferedEglState::new(width, height, Depth::Enabled)?,
        })
    }

    /// Begin a frame -- allocate the back buffer if needed, bind its FBO.
    #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
    pub fn begin_frame(&mut self) -> Result<()> {
        let buf = self.egl.ensure_current()?;
        let fbo = buf.fbo;
        let (w, h) = (self.egl.width(), self.egl.height());

        unsafe {
            self.egl.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.egl.gl().viewport(0, 0, w as i32, h as i32);
        }
        Ok(())
    }

    /// Clear the screen with a color (and depth buffer for 3D rendering).
    pub fn clear(&self, r: f32, g: f32, b: f32, a: f32) {
        unsafe {
            self.egl.gl().clear_color(r, g, b, a);
            self.egl
                .gl()
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
    }

    /// End frame -- `glFinish()`, export DMA-BUF, swap buffers.
    ///
    /// Returns the DMA-BUF info and the slot index of the exported buffer.
    pub fn end_frame(&mut self) -> Result<(DmaBufInfo, usize)> {
        unsafe {
            self.egl.gl().finish();
        }
        self.egl.export_and_swap()
    }

    /// Resize -- deallocate existing buffers so they're reallocated at the new size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.egl.resize(width, height);
    }

    /// Release all DMA-BUF export buffers; the next [`Self::begin_frame`]
    /// call will lazily reallocate them.
    pub fn destroy_buffers(&mut self) {
        self.egl.destroy_buffers();
    }

    /// Get the glow OpenGL ES context.
    pub fn gl(&self) -> &glow::Context {
        self.egl.gl()
    }
}

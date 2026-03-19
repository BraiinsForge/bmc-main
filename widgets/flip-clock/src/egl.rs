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
use bmc_widget::egl::{DoubleBufferState, EglContext};

/// EGL state for the flip-clock's direct-FBO rendering pipeline.
///
/// Double-buffers two export buffers via [`DoubleBufferState`]. Each frame:
/// bind the back buffer's FBO, render, `glFinish()`, export DMA-BUF, swap.
pub struct EglState {
    ctx: EglContext,
    db: DoubleBufferState,
}

impl EglState {
    /// Create EGL context and prepare for rendering at the given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let ctx = EglContext::new()?;
        Ok(Self {
            ctx,
            db: DoubleBufferState::new(width, height),
        })
    }

    /// Begin a frame -- allocate the back buffer if needed, bind its FBO.
    #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
    pub fn begin_frame(&mut self) -> Result<()> {
        let buf = self.db.ensure_current(&self.ctx)?;
        let fbo = buf.fbo;
        let (w, h) = (self.db.width(), self.db.height());

        unsafe {
            self.ctx.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.ctx.gl().viewport(0, 0, w as i32, h as i32);
        }
        Ok(())
    }

    /// Clear the screen with a color.
    pub fn clear(&self, r: f32, g: f32, b: f32, a: f32) {
        unsafe {
            self.ctx.gl().clear_color(r, g, b, a);
            self.ctx.gl().clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// End frame -- `glFinish()`, export DMA-BUF, swap buffers.
    ///
    /// Returns the DMA-BUF info and the slot index of the exported buffer.
    pub fn end_frame(&mut self) -> Result<(DmaBufInfo, usize)> {
        unsafe {
            self.ctx.gl().finish();
        }
        self.db.export_and_swap()
    }

    /// Resize -- deallocate existing buffers so they're reallocated at the new size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.db.resize(&self.ctx, width, height);
    }

    /// Get the glow OpenGL ES context.
    pub fn gl(&self) -> &glow::Context {
        self.ctx.gl()
    }
}

impl Drop for EglState {
    fn drop(&mut self) {
        self.db.destroy_all(&self.ctx);
    }
}

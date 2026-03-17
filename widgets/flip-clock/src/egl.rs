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
use bmc_widget::egl::{EglContext, ExportBuffer};

/// EGL state for the flip-clock's direct-FBO rendering pipeline.
///
/// Double-buffers two [`ExportBuffer`]s. Each frame: bind the back buffer's
/// FBO, render, `glFinish()`, export DMA-BUF, swap.
pub struct EglState {
    ctx: EglContext,
    buffers: [Option<ExportBuffer>; 2],
    current_buffer: usize,
    width: u32,
    height: u32,
}

impl EglState {
    /// Create EGL context and prepare for rendering at the given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let ctx = EglContext::new()?;
        Ok(Self {
            ctx,
            buffers: [None, None],
            current_buffer: 0,
            width,
            height,
        })
    }

    /// Begin a frame — allocate the back buffer if needed, bind its FBO.
    #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
    pub fn begin_frame(&mut self) -> Result<()> {
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(self.ctx.allocate_export_buffer(self.width, self.height)?);
        }

        let fbo = self.buffers[idx]
            .as_ref()
            .expect("BUG: buffer should exist after allocation")
            .fbo;

        unsafe {
            self.ctx.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.ctx
                .gl()
                .viewport(0, 0, self.width as i32, self.height as i32);
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

    /// End frame — `glFinish()`, export DMA-BUF, swap buffers.
    pub fn end_frame(&mut self) -> Result<DmaBufInfo> {
        unsafe {
            self.ctx.gl().finish();
        }

        let idx = self.current_buffer;
        let buf = self.buffers[idx]
            .as_mut()
            .expect("BUG: buffer should exist after begin_frame");

        let info = EglContext::export_dmabuf(buf)?;

        self.current_buffer = 1 - self.current_buffer;
        Ok(info)
    }

    /// Resize — deallocate existing buffers so they're reallocated at the new size.
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

        self.width = width;
        self.height = height;
    }

    /// Get the glow OpenGL ES context.
    pub fn gl(&self) -> &glow::Context {
        self.ctx.gl()
    }

    /// Index of the buffer that was just rendered (valid after `end_frame`).
    pub fn last_rendered_slot(&self) -> usize {
        1 - self.current_buffer
    }
}

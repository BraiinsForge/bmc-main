// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! Two-FBO EGL rendering pipeline for FemtoVG widgets.
//!
//! FemtoVG renders Y-flipped when targeting an FBO. This module composes
//! [`bmc_widget::egl`] primitives — one [`EglContext`], one
//! [`SharedRenderScratch`] (staging FBO + Y-flip blit program), and one
//! [`DoubleBufferState`] for the DMA-BUF export pair — into the per-widget
//! render path.
//!
//! Pipeline: FemtoVG → staging FBO → blit (Y-flip) → export FBO → DMA-BUF

use std::mem::ManuallyDrop;

use anyhow::{Context, Result};
use glow::HasContext;

pub use bmc_widget::egl::DmaBufInfo;
use bmc_widget::egl::{Depth, DoubleBufferState, EglContext, SharedRenderScratch};

/// Per-widget EGL state composed of a context, a shared scratch (staging +
/// blit), and a double-buffered DMA-BUF export pair.
pub struct EglState {
    scratch: ManuallyDrop<SharedRenderScratch>,
    buffers: DoubleBufferState,
    ctx: EglContext,
    /// Current render size in pixels — also the per-frame viewport.
    width: u32,
    height: u32,
    /// Maximum the staging was sized to in `SharedRenderScratch::new`.
    /// Persists across `resize` so a downsize followed by a re-upsize back
    /// to the original maximum is still bounded by the staging.
    scratch_max_width: u32,
    scratch_max_height: u32,
}

impl EglState {
    /// Create EGL context, allocate scratch at `(width, height)`, prepare an
    /// empty double-buffer state. Export buffers allocate lazily on the first
    /// `begin_frame`. The width/height passed here bound any subsequent
    /// `resize`; consumers that may grow beyond their initial size should
    /// construct at the eventual maximum and downsize via `resize`.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let ctx = EglContext::new()?;
        let scratch = ManuallyDrop::new(SharedRenderScratch::new(&ctx, width, height)?);
        let buffers = DoubleBufferState::new(width, height, Depth::Disabled);
        Ok(Self {
            scratch,
            buffers,
            ctx,
            width,
            height,
            scratch_max_width: width,
            scratch_max_height: height,
        })
    }

    /// Begin a frame — ensure an export buffer exists, bind the staging FBO,
    /// clear it. Returns the raw GL framebuffer name of the staging FBO for
    /// FemtoVG's `set_screen_target`.
    pub fn begin_frame(&mut self) -> Result<u32> {
        self.buffers.ensure_current(&self.ctx)?;
        Ok(self.scratch.begin_frame(&self.ctx, self.width, self.height))
    }

    /// Blit staging FBO → current export FBO with Y-flip. Call after FemtoVG
    /// `flush()` and before `end_frame()`.
    pub fn blit_to_export(&self) -> Result<()> {
        let export = self
            .buffers
            .current_ref()
            .context("BUG: export buffer not allocated")?;
        self.scratch
            .blit_to(&self.ctx, export.fbo, self.width, self.height);
        Ok(())
    }

    /// End frame — `gl.flush()`, export DMA-BUF, swap buffers.
    ///
    /// Returns the DMA-BUF info and the slot index of the exported buffer.
    pub fn end_frame(&mut self) -> Result<(DmaBufInfo, usize)> {
        unsafe {
            self.ctx.gl().flush();
        }
        self.buffers.export_and_swap()
    }

    /// Resize — deallocate export buffers so they are reallocated at the new
    /// size. The scratch staging stays at the size set in `new`; the new
    /// dimensions must fit within the maximum chosen at construction, or the
    /// call returns an error and leaves state untouched.
    #[expect(
        dead_code,
        reason = "resize support for when protocol adds resize events"
    )]
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.width == width && self.height == height {
            return Ok(());
        }
        anyhow::ensure!(
            width <= self.scratch_max_width && height <= self.scratch_max_height,
            "resize {width}x{height} exceeds scratch maximum {}x{} set at construction",
            self.scratch_max_width,
            self.scratch_max_height,
        );
        tracing::debug!(
            "Resizing from {}x{} to {}x{} (scratch max {}x{})",
            self.width,
            self.height,
            width,
            height,
            self.scratch_max_width,
            self.scratch_max_height,
        );
        self.width = width;
        self.height = height;
        self.buffers.resize(&self.ctx, width, height);
        Ok(())
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
}

impl Drop for EglState {
    fn drop(&mut self) {
        // SAFETY: `scratch` is a private field with no consuming accessor, so
        // this is the only call to `ManuallyDrop::take` on it across the
        // whole crate; `Drop::drop` runs exactly once per value, so the take
        // happens at most once. `SharedRenderScratch::destroy` needs `ctx`
        // current on this thread — we are still inside `&mut self` so `ctx`
        // is alive (it drops after this method returns).
        let scratch = unsafe { ManuallyDrop::take(&mut self.scratch) };
        scratch.destroy(&self.ctx);
        self.buffers.destroy_all(&self.ctx);
    }
}

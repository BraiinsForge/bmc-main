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
    release_state: ExportSlotReleaseState,
}

impl EglState {
    /// Create EGL context and prepare for rendering at the given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            egl: DoubleBufferedEglState::new(width, height, Depth::Enabled)?,
            release_state: ExportSlotReleaseState::new(),
        })
    }

    #[must_use]
    pub fn current_buffer_available(&self) -> bool {
        self.release_state.is_available(self.egl.current_slot())
    }

    pub fn mark_released_slots(&mut self, slots: impl IntoIterator<Item = usize>) {
        for slot in slots {
            self.release_state.mark_released(slot);
        }
    }

    pub fn destroy_released_buffers(&mut self) -> Vec<usize> {
        let slots = self
            .release_state
            .destroyable_slots(self.egl.allocated_slots());
        for slot in &slots {
            self.egl.destroy_slot(*slot);
        }
        slots
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
        let (info, slot) = self.egl.export_and_swap()?;
        self.release_state.mark_presented(slot);
        Ok((info, slot))
    }

    /// Get the glow OpenGL ES context.
    pub fn gl(&self) -> &glow::Context {
        self.egl.gl()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSlotReleaseState {
    available: [bool; 2],
}

impl ExportSlotReleaseState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            available: [true, true],
        }
    }

    #[must_use]
    pub fn is_available(self, slot: usize) -> bool {
        self.available.get(slot).copied().unwrap_or(false)
    }

    pub fn mark_presented(&mut self, slot: usize) {
        if let Some(available) = self.available.get_mut(slot) {
            *available = false;
        }
    }

    pub fn mark_released(&mut self, slot: usize) {
        if let Some(available) = self.available.get_mut(slot) {
            *available = true;
        }
    }

    #[must_use]
    pub fn destroyable_slots(self, allocated_slots: [bool; 2]) -> Vec<usize> {
        allocated_slots
            .iter()
            .enumerate()
            .filter_map(|(slot, allocated)| {
                if *allocated && self.is_available(slot) {
                    Some(slot)
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Default for ExportSlotReleaseState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ExportSlotReleaseState;

    #[test]
    fn release_state_blocks_presented_slot_until_release() {
        let mut state = ExportSlotReleaseState::new();

        assert!(state.is_available(0));

        state.mark_presented(0);
        assert!(!state.is_available(0));
        assert_eq!(state.destroyable_slots([true, true]), vec![1]);

        state.mark_released(0);
        assert!(state.is_available(0));
        assert_eq!(state.destroyable_slots([true, true]), vec![0, 1]);
    }
}

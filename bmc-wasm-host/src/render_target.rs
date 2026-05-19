// Copyright (C) 2026  Braiins Systems s.r.o.

use std::any::Any;

use crate::lifecycle::{LifecycleEgl, LifecycleSurface};

#[derive(Debug, thiserror::Error)]
pub enum RenderTargetError {
    #[error("EGL allocation failed: {0}")]
    Egl(#[from] anyhow::Error),
    #[error("Wayland buffer creation failed: {0}")]
    Wayland(String),
}

#[expect(missing_debug_implementations)]
pub struct RenderTarget {
    inner: Box<dyn Any>,
    pub width: u32,
    pub height: u32,
}

/// Concrete payload for the production EGL factory. Constructed in Task 8 by
/// `EglRenderTargetFactory::allocate` and unwrapped by `WidgetSlot::render` /
/// `EglRenderTargetFactory::destroy` via `as_egl_mut` / `into_egl`.
#[expect(missing_debug_implementations)]
pub struct EglRenderTarget {
    pub buffers: bmc_widget::egl::DoubleBufferState,
    pub wl_buffers: [wayland_client::protocol::wl_buffer::WlBuffer; 2],
    release_state: RenderSlotReleaseState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderSlotReleaseState {
    available: [bool; 2],
}

impl RenderSlotReleaseState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            available: [true, true],
        }
    }

    #[must_use]
    pub fn is_available(&self, slot: usize) -> bool {
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
}

impl Default for RenderSlotReleaseState {
    fn default() -> Self {
        Self::new()
    }
}

impl EglRenderTarget {
    #[must_use]
    pub fn current_slot_available(&self) -> bool {
        self.release_state.is_available(self.buffers.current_slot())
    }

    pub fn mark_presented(&mut self, slot: usize) {
        self.release_state.mark_presented(slot);
    }

    pub fn mark_released(&mut self, slot: usize) {
        self.release_state.mark_released(slot);
    }
}

impl RenderTarget {
    #[must_use]
    pub fn new_egl(
        buffers: bmc_widget::egl::DoubleBufferState,
        wl_buffers: [wayland_client::protocol::wl_buffer::WlBuffer; 2],
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            inner: Box::new(EglRenderTarget {
                buffers,
                wl_buffers,
                release_state: RenderSlotReleaseState::new(),
            }),
            width,
            height,
        }
    }

    /// Unit-stub constructor for the lifecycle state-machine unit tests. Stays
    /// in the public surface (not feature-gated) because integration tests in
    /// `tests/` build against the library in its cfg(test)-off shape and need
    /// a way to mint a `RenderTarget` without standing up real EGL / Wayland.
    /// The cost of one public `Stub` zero-sized type is dwarfed by the test
    /// coverage it unlocks — full 5×5 lifecycle matrix in Task 7 alone.
    #[must_use]
    pub fn new_stub(width: u32, height: u32) -> Self {
        struct Stub;
        Self {
            inner: Box::new(Stub),
            width,
            height,
        }
    }

    pub fn as_egl_mut(&mut self) -> Option<&mut EglRenderTarget> {
        self.inner.downcast_mut::<EglRenderTarget>()
    }

    #[must_use]
    pub fn as_egl(&self) -> Option<&EglRenderTarget> {
        self.inner.downcast_ref::<EglRenderTarget>()
    }

    pub fn into_egl(self) -> Result<EglRenderTarget, Self> {
        let Self {
            inner,
            width,
            height,
        } = self;
        match inner.downcast::<EglRenderTarget>() {
            Ok(boxed) => Ok(*boxed),
            Err(inner) => Err(Self {
                inner,
                width,
                height,
            }),
        }
    }
}

pub trait RenderTargetFactory {
    fn allocate(
        &self,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
        width: u32,
        height: u32,
    ) -> Result<RenderTarget, RenderTargetError>;

    fn destroy(
        &self,
        target: RenderTarget,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    );
}

#[derive(Debug)]
pub struct EglRenderTargetFactory;

impl RenderTargetFactory for EglRenderTargetFactory {
    fn allocate(
        &self,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
        width: u32,
        height: u32,
    ) -> Result<RenderTarget, RenderTargetError> {
        use bmc_widget::egl::{Depth, DoubleBufferState};

        let egl = egl.as_egl_context();

        let mut buffers = DoubleBufferState::new(width, height, Depth::Disabled);
        buffers
            .ensure_current(egl)
            .map_err(RenderTargetError::Egl)?;
        let (dmabuf_a, slot_a) = match buffers.export_and_swap() {
            Ok(export) => export,
            Err(e) => {
                buffers.destroy_all(egl);
                return Err(RenderTargetError::Egl(e));
            }
        };
        if let Err(e) = buffers.ensure_current(egl) {
            buffers.destroy_all(egl);
            return Err(RenderTargetError::Egl(e));
        }
        let (dmabuf_b, slot_b) = match buffers.export_and_swap() {
            Ok(export) => export,
            Err(e) => {
                buffers.destroy_all(egl);
                return Err(RenderTargetError::Egl(e));
            }
        };

        if !matches!((slot_a, slot_b), (0, 1) | (1, 0)) {
            buffers.destroy_all(egl);
            return Err(RenderTargetError::Egl(anyhow::anyhow!(
                "DoubleBufferState returned invalid slot ids: {slot_a}, {slot_b}"
            )));
        }

        let wl_buffer_a = match surface.mint_wl_buffer(&dmabuf_a, slot_a) {
            Ok(buffer) => buffer,
            Err(e) => {
                buffers.destroy_all(egl);
                return Err(RenderTargetError::Wayland(e));
            }
        };
        let wl_buffer_b = match surface.mint_wl_buffer(&dmabuf_b, slot_b) {
            Ok(buffer) => buffer,
            Err(e) => {
                surface.destroy_minted_wl_buffer(wl_buffer_a);
                buffers.destroy_all(egl);
                return Err(RenderTargetError::Wayland(e));
            }
        };
        let wl_buffers = if slot_a == 0 {
            [wl_buffer_a, wl_buffer_b]
        } else {
            [wl_buffer_b, wl_buffer_a]
        };

        Ok(RenderTarget::new_egl(buffers, wl_buffers, width, height))
    }

    fn destroy(
        &self,
        target: RenderTarget,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) {
        let egl = egl.as_egl_context();
        let Ok(mut target) = target.into_egl() else {
            tracing::error!("BUG: EglRenderTargetFactory::destroy received a non-EGL RenderTarget");
            return;
        };
        target.buffers.destroy_all(egl);
        let [a, b] = target.wl_buffers;
        surface.destroy_minted_wl_buffer(a);
        surface.destroy_minted_wl_buffer(b);
    }
}

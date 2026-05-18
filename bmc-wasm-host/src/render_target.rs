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
        surface: &dyn LifecycleSurface,
        width: u32,
        height: u32,
    ) -> Result<RenderTarget, RenderTargetError>;

    fn destroy(&self, target: RenderTarget, egl: &dyn LifecycleEgl);
}

#[derive(Debug)]
pub struct EglRenderTargetFactory;

impl RenderTargetFactory for EglRenderTargetFactory {
    fn allocate(
        &self,
        egl: &dyn LifecycleEgl,
        surface: &dyn LifecycleSurface,
        width: u32,
        height: u32,
    ) -> Result<RenderTarget, RenderTargetError> {
        use bmc_widget::egl::{Depth, DoubleBufferState};

        let egl = egl.as_egl_context();
        let surface = surface.as_deck_widget_surface();

        let mut buffers = DoubleBufferState::new(width, height, Depth::Disabled);
        buffers
            .ensure_current(egl)
            .map_err(RenderTargetError::Egl)?;
        let (dmabuf_a, _) = buffers.export_and_swap().map_err(RenderTargetError::Egl)?;
        buffers
            .ensure_current(egl)
            .map_err(RenderTargetError::Egl)?;
        let (dmabuf_b, _) = buffers.export_and_swap().map_err(RenderTargetError::Egl)?;

        let wl_buffer_a = surface
            .mint_wl_buffer(&dmabuf_a)
            .map_err(RenderTargetError::Wayland)?;
        let wl_buffer_b = surface
            .mint_wl_buffer(&dmabuf_b)
            .map_err(RenderTargetError::Wayland)?;

        Ok(RenderTarget::new_egl(
            buffers,
            [wl_buffer_a, wl_buffer_b],
            width,
            height,
        ))
    }

    fn destroy(&self, target: RenderTarget, egl: &dyn LifecycleEgl) {
        let egl = egl.as_egl_context();
        let Ok(mut target) = target.into_egl() else {
            tracing::error!("BUG: EglRenderTargetFactory::destroy received a non-EGL RenderTarget");
            return;
        };
        target.buffers.destroy_all(egl);
        let [a, b] = target.wl_buffers;
        a.destroy();
        b.destroy();
    }
}

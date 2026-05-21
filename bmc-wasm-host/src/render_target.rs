// Copyright (C) 2026  Braiins Systems s.r.o.

use std::any::Any;

use crate::lifecycle::{LifecycleEgl, LifecycleSurface};
use bmc_widget::surface::ReleasedBuffer;

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
    pub wl_buffers: [Option<wayland_client::protocol::wl_buffer::WlBuffer>; 2],
    release_state: RenderSlotReleaseState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTargetCleanup {
    Complete,
    PendingRelease,
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

    #[must_use]
    pub fn destroyable_slots(&self, allocated_slots: [bool; 2]) -> Vec<usize> {
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

    #[must_use]
    pub fn prepared_compaction_slots(
        &self,
        allocated_slots: [bool; 2],
        current_slot: usize,
    ) -> Vec<usize> {
        let allocated_count = allocated_slots
            .iter()
            .filter(|allocated| **allocated)
            .count();
        if allocated_count <= 1 {
            return Vec::new();
        }

        let Some(slot) = (0..allocated_slots.len())
            .find(|slot| {
                allocated_slots[*slot] && self.is_available(*slot) && *slot != current_slot
            })
            .or_else(|| {
                (0..allocated_slots.len())
                    .find(|slot| allocated_slots[*slot] && self.is_available(*slot))
            })
        else {
            return Vec::new();
        };

        vec![slot]
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

    pub fn mark_released_buffer(&mut self, released: &ReleasedBuffer) {
        for (slot, wl_buffer) in self.wl_buffers.iter().enumerate() {
            if wl_buffer
                .as_ref()
                .is_some_and(|buffer| released.matches(buffer))
            {
                self.release_state.mark_released(slot);
                return;
            }
        }
    }

    pub fn wl_buffer_for_slot(
        &mut self,
        surface: &mut dyn LifecycleSurface,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
        slot: usize,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        let Some(wl_buffer) = self.wl_buffers.get_mut(slot) else {
            return Err(format!(
                "DoubleBufferState returned invalid slot id: {slot}"
            ));
        };
        if wl_buffer.is_none() {
            *wl_buffer = Some(surface.mint_wl_buffer(dmabuf, slot)?);
        }
        Ok(wl_buffer
            .as_ref()
            .expect("BUG: wl_buffer should exist after mint above")
            .clone())
    }

    pub fn compact_for_prepared(
        &mut self,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) {
        let egl = egl.as_egl_context();
        let slots = self
            .release_state
            .prepared_compaction_slots(self.buffers.allocated_slots(), self.buffers.current_slot());
        for slot in slots {
            if self.buffers.destroy_slot(egl, slot)
                && let Some(buffer) = self.wl_buffers[slot].take()
            {
                surface.destroy_minted_wl_buffer(buffer);
            }
        }
    }

    pub fn destroy_released_slots(
        &mut self,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) -> RenderTargetCleanup {
        let egl = egl.as_egl_context();
        let slots = self
            .release_state
            .destroyable_slots(self.buffers.allocated_slots());
        for slot in slots {
            if self.buffers.destroy_slot(egl, slot)
                && let Some(buffer) = self.wl_buffers[slot].take()
            {
                surface.destroy_minted_wl_buffer(buffer);
            }
        }
        if self
            .buffers
            .allocated_slots()
            .iter()
            .any(|allocated| *allocated)
        {
            RenderTargetCleanup::PendingRelease
        } else {
            RenderTargetCleanup::Complete
        }
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
        let [wl_buffer_a, wl_buffer_b] = wl_buffers;
        Self {
            inner: Box::new(EglRenderTarget {
                buffers,
                wl_buffers: [Some(wl_buffer_a), Some(wl_buffer_b)],
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

    fn compact_for_prepared(
        &self,
        _target: &mut RenderTarget,
        _egl: &dyn LifecycleEgl,
        _surface: &mut dyn LifecycleSurface,
    ) {
    }

    fn destroy_released_slots(
        &self,
        target: &mut RenderTarget,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) -> RenderTargetCleanup;
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
        for buffer in target.wl_buffers.into_iter().flatten() {
            surface.destroy_minted_wl_buffer(buffer);
        }
    }

    fn compact_for_prepared(
        &self,
        target: &mut RenderTarget,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) {
        let Some(target) = target.as_egl_mut() else {
            tracing::error!(
                "BUG: EglRenderTargetFactory::compact_for_prepared received a non-EGL RenderTarget"
            );
            return;
        };
        target.compact_for_prepared(egl, surface);
    }

    fn destroy_released_slots(
        &self,
        target: &mut RenderTarget,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) -> RenderTargetCleanup {
        let Some(target) = target.as_egl_mut() else {
            tracing::error!(
                "BUG: EglRenderTargetFactory::destroy_released_slots received a non-EGL RenderTarget"
            );
            return RenderTargetCleanup::Complete;
        };
        target.destroy_released_slots(egl, surface)
    }
}

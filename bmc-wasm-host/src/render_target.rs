// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::any::Any;

use crate::lifecycle::{LifecycleEgl, LifecycleSurface};
use bmc_widget::egl::{SlotReleaseState, TwoSlotBufferCache};
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
    pub wl_buffers: TwoSlotBufferCache<wayland_client::protocol::wl_buffer::WlBuffer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTargetCleanup {
    Complete,
    PendingRelease,
}

/// Pick the single spare slot a `Prepared` slot can drop, or `None` when it is
/// already down to one buffer.
///
/// A `Prepared` slot pre-renders one frame and then needs only the buffer the
/// compositor is displaying; the other allocated slot is the spare. The
/// fallback intentionally keeps the in-flight (currently displayed) pre-render
/// and drops the still-available slot — never more than one.
#[must_use]
fn prepared_compaction_slot(
    release: SlotReleaseState,
    allocated_slots: [bool; 2],
    current_slot: usize,
) -> Option<usize> {
    if allocated_slots
        .iter()
        .filter(|allocated| **allocated)
        .count()
        <= 1
    {
        return None;
    }
    (0..allocated_slots.len())
        .find(|&slot| allocated_slots[slot] && release.is_available(slot) && slot != current_slot)
        .or_else(|| {
            (0..allocated_slots.len())
                .find(|&slot| allocated_slots[slot] && release.is_available(slot))
        })
}

impl EglRenderTarget {
    #[must_use]
    pub fn has_prepared_compaction_work(&self) -> bool {
        prepared_compaction_slot(
            self.wl_buffers.release_state(),
            self.buffers.allocated_slots(),
            self.buffers.current_slot(),
        )
        .is_some()
    }

    #[must_use]
    pub fn has_released_slot_cleanup(&self) -> bool {
        self.wl_buffers
            .release_state()
            .destroyable_slots(self.buffers.allocated_slots())
            .next()
            .is_some()
    }

    #[must_use]
    pub fn current_slot_available(&self) -> bool {
        self.wl_buffers.is_available(self.buffers.current_slot())
    }

    pub fn mark_presented(&mut self, slot: usize) {
        self.wl_buffers.mark_presented(slot);
    }

    pub fn mark_released(&mut self, slot: usize) {
        self.wl_buffers.mark_released(slot);
    }

    pub fn mark_released_buffer(&mut self, released: &ReleasedBuffer) {
        self.wl_buffers
            .mark_released_matching(|buffer| released.matches(buffer));
    }

    pub fn wl_buffer_for_slot(
        &mut self,
        surface: &mut dyn LifecycleSurface,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
        slot: usize,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        let Some(wl_buffer) = self
            .wl_buffers
            .get_or_try_insert_with(slot, || surface.mint_wl_buffer(dmabuf, slot))?
        else {
            return Err(format!(
                "DoubleBufferState returned invalid slot id: {slot}"
            ));
        };
        Ok(wl_buffer.clone())
    }

    pub fn compact_for_prepared(
        &mut self,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) {
        let egl = egl.as_egl_context();
        let Some(slot) = prepared_compaction_slot(
            self.wl_buffers.release_state(),
            self.buffers.allocated_slots(),
            self.buffers.current_slot(),
        ) else {
            return;
        };
        if self.buffers.destroy_slot(egl, slot)
            && let Some(buffer) = self.wl_buffers.take_slot(slot)
        {
            surface.destroy_minted_wl_buffer(buffer);
        }
    }

    pub fn destroy_released_slots(
        &mut self,
        egl: &dyn LifecycleEgl,
        surface: &mut dyn LifecycleSurface,
    ) -> RenderTargetCleanup {
        let egl = egl.as_egl_context();
        let release_state = self.wl_buffers.release_state();
        let slots: Vec<_> = release_state
            .destroyable_slots(self.buffers.allocated_slots())
            .collect();
        for slot in slots {
            if self.buffers.destroy_slot(egl, slot)
                && let Some(buffer) = self.wl_buffers.take_slot(slot)
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
        let mut wl_buffers = TwoSlotBufferCache::new();
        let _ = wl_buffers
            .get_or_try_insert_with(0, || Ok::<_, std::convert::Infallible>(wl_buffer_a))
            .expect("BUG: infallible wl_buffer insert")
            .expect("BUG: two-slot cache has slot 0");
        let _ = wl_buffers
            .get_or_try_insert_with(1, || Ok::<_, std::convert::Infallible>(wl_buffer_b))
            .expect("BUG: infallible wl_buffer insert")
            .expect("BUG: two-slot cache has slot 1");
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
        for buffer in target.wl_buffers.take_all().into_iter().flatten() {
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

#[cfg(test)]
mod tests {
    use super::prepared_compaction_slot;
    use bmc_widget::egl::SlotReleaseState;

    #[test]
    fn prepared_compaction_drops_available_spare_slot_before_first_render() {
        let state = SlotReleaseState::new();

        assert_eq!(prepared_compaction_slot(state, [true, true], 0), Some(1));
    }

    #[test]
    fn prepared_compaction_drops_available_back_slot_after_submit() {
        let mut state = SlotReleaseState::new();
        state.mark_presented(0);

        assert_eq!(prepared_compaction_slot(state, [true, true], 1), Some(1));
    }

    #[test]
    fn prepared_compaction_keeps_the_only_allocated_slot() {
        let mut state = SlotReleaseState::new();
        state.mark_presented(0);

        assert_eq!(prepared_compaction_slot(state, [true, false], 1), None);
    }
}

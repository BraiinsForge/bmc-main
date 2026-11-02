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

//! GPU frame helpers for system overlays: an EGL fence wait that makes an
//! exported DMA-BUF safe to hand to the compositor, and a double-buffered
//! render target wrapping `bmc_widget`'s export-buffer machinery with the
//! `wl_buffer.release` bookkeeping the compositor drives.

use bmc_intl::Intl;
use bmc_render::renderer::Renderer;
use bmc_render::repaint::TargetContents;
use bmc_widget::egl::{
    Attachment, DmaBufInfo, DoubleBufferState, EglContext, ExportContents, ExportFormat,
    SharedRenderScratch, TwoSlotBufferCache,
};
use bmc_widget::surface::ReleasedBuffer;
use glow::HasContext as _;
use wayland_client::protocol::wl_buffer;

use crate::{FrameKind, OverlayViewport, RenderOutcome, SystemOverlay, paint_surface_frame};

/// Stall the CPU until the GPU has finished the submitted commands, so the
/// exported DMA-BUF is safe to hand to the compositor. Uses an EGL fence;
/// falls back to `glFinish` if the fence fails.
///
/// Mirrors `bmc-wasm-host`'s private `Host::wait_for_egl_fence`; keep the
/// fence/`glFinish` fallback policy in sync with it.
pub fn wait_for_gpu(egl: &EglContext) {
    if let Err(e) = egl.wait_for_egl_fence() {
        tracing::warn!(?e, "EGL fence wait failed; falling back to glFinish");
        unsafe {
            egl.gl().finish();
        }
    }
}

/// The damage history counts how many frames must repaint in full after a buffer goes stale,
/// and cannot see how many buffers there are. This crate sees both, so a host that started
/// triple-buffering fails to compile here rather than flickering a stale buffer back on screen.
const _: () = assert!(
    bmc_widget::egl::EXPORT_BUFFER_SLOTS == bmc_render::repaint::EXPORT_BUFFERS,
    "bmc-widget rotates a different number of export buffers than bmc-render's damage history spans"
);

/// What the export buffer a frame binds holds, as the damage history decided.
#[must_use]
pub fn export_contents(contents: TargetContents) -> ExportContents {
    match contents {
        TargetContents::Preserved => ExportContents::Preserved,
        TargetContents::Cleared => ExportContents::Cleared,
    }
}

/// Double-buffered DMA-BUF render target with `wl_buffer.release` tracking.
///
/// Pairs [`DoubleBufferState`] (two lazily-allocated export buffers) with a
/// cache of the minted `wl_buffer`s and release state so that a
/// compositor `wl_buffer.release` frees the matching export slot for reuse.
#[expect(missing_debug_implementations)]
pub struct OverlayRenderTarget {
    buffers: DoubleBufferState,
    wl_buffers: TwoSlotBufferCache<wl_buffer::WlBuffer>,
}

impl OverlayRenderTarget {
    /// Create an empty render target at `w`×`h`. Export buffers are allocated
    /// lazily on the first [`Self::paint`]. The `_egl` argument is
    /// accepted to match the host factory.
    pub fn new(_egl: &EglContext, w: u32, h: u32) -> anyhow::Result<Self> {
        Ok(Self {
            buffers: DoubleBufferState::new_with_format(
                w,
                h,
                Attachment::Stencil,
                ExportFormat::Alpha,
            ),
            wl_buffers: TwoSlotBufferCache::new(),
        })
    }

    /// Paint a `kind` frame of `overlay` into the current export buffer,
    /// through `scratch`'s frame framebuffer, as [`paint_surface_frame`] does.
    #[expect(
        clippy::too_many_arguments,
        reason = "the GPU stack the frame paints through, the overlay and what it is drawn as"
    )]
    pub fn paint(
        &mut self,
        egl: &EglContext,
        scratch: &SharedRenderScratch,
        renderer: &mut dyn Renderer,
        overlay: &mut dyn SystemOverlay,
        viewport: OverlayViewport,
        intl: &Intl,
        kind: FrameKind,
    ) -> anyhow::Result<RenderOutcome> {
        let (w, h) = self.size();
        let buffers = &mut self.buffers;
        paint_surface_frame(
            overlay,
            <dyn SystemOverlay>::tree_ui,
            renderer,
            (w, h),
            kind,
            |contents| scratch.bind_export_buffer(egl, buffers, w, h, export_contents(contents)),
            |overlay, renderer| overlay.render(renderer, viewport, intl, kind),
        )
    }

    /// Export the current buffer as DMA-BUF and swap to the other slot.
    /// Returns the DMA-BUF info and the slot index of the exported buffer.
    pub fn export_and_swap(&mut self) -> anyhow::Result<(DmaBufInfo, usize)> {
        self.buffers.export_and_swap()
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.buffers.width(), self.buffers.height())
    }

    pub fn resize(
        &mut self,
        egl: &EglContext,
        client: &mut crate::surface::LayerSurfaceClient,
        w: u32,
        h: u32,
    ) -> anyhow::Result<()> {
        if self.size() == (w, h) {
            return Ok(());
        }
        for buffer in self.wl_buffers.take_all().into_iter().flatten() {
            client.destroy_minted_wl_buffer(buffer);
        }
        self.wl_buffers.reset_release_state();
        self.buffers.resize(egl, w, h);
        client.flush()
    }

    /// True when the next export slot's buffer has been released by the
    /// compositor (or has never been submitted).
    #[must_use]
    pub fn available(&self) -> bool {
        self.wl_buffers.is_available(self.buffers.current_slot())
    }

    /// Record that `slot`'s buffer has been submitted to the compositor and is
    /// pinned until its `wl_buffer.release`.
    pub fn mark_presented(&mut self, slot: usize) {
        self.wl_buffers.mark_presented(slot);
    }

    /// Translate a compositor `wl_buffer.release` into a freed export slot by
    /// matching the released buffer against the cached `wl_buffer`s.
    pub fn mark_released_buffer(&mut self, released: &ReleasedBuffer) {
        self.wl_buffers
            .mark_released_matching(|buffer| released.matches(buffer));
    }

    /// Mint (once) and cache the `wl_buffer` for `slot` via the layer-surface
    /// client. Subsequent calls return the cached buffer.
    pub fn wl_buffer_for_slot(
        &mut self,
        client: &mut crate::surface::LayerSurfaceClient,
        info: &DmaBufInfo,
        slot: usize,
    ) -> anyhow::Result<wl_buffer::WlBuffer> {
        let Some(wl_buffer) = self
            .wl_buffers
            .get_or_try_insert_with(slot, || client.mint_wl_buffer(info, slot))?
        else {
            anyhow::bail!("invalid export slot id: {slot}");
        };
        Ok(wl_buffer.clone())
    }

    /// Free the GBM/GL export buffers and cached `wl_buffer`s for a hide, but
    /// keep the target reusable: a later `paint` reallocates lazily.
    pub fn free_for_hide(
        &mut self,
        egl: &EglContext,
        client: &mut crate::surface::LayerSurfaceClient,
    ) -> anyhow::Result<()> {
        let cached_wl_buffers = self.wl_buffers.cached_count();
        tracing::info!(cached_wl_buffers, "free_for_hide: freeing overlay buffers");
        self.buffers.destroy_all(egl);
        for buffer in self.wl_buffers.take_all().into_iter().flatten() {
            client.destroy_minted_wl_buffer(buffer);
        }
        self.wl_buffers.reset_release_state();
        client.flush()
    }

    /// Free all GL/EGL/GBM resources and destroy the cached `wl_buffer`s.
    ///
    /// [`DoubleBufferState`] does not clean up on `Drop`, so the owner must
    /// call this explicitly while the EGL context is still current.
    ///
    /// Unlike [`Self::free_for_hide`], the `wl_buffer`s are destroyed directly
    /// rather than through `LayerSurfaceClient::destroy_minted_wl_buffer`, so the
    /// client's slot/released-buffer maps keep their now-dead ids. This is sound
    /// only because `destroy` is terminal: every caller drops the client right
    /// after, so the stale entries are never observed. Route through the client
    /// instead if a post-`destroy` surface is ever made reusable.
    pub fn destroy(&mut self, egl: &EglContext) {
        self.buffers.destroy_all(egl);
        for buffer in self.wl_buffers.take_all().into_iter().flatten() {
            buffer.destroy();
        }
    }
}

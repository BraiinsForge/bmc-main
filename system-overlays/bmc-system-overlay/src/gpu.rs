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

use anyhow::Context as _;
use bmc_widget::egl::{
    Depth, DmaBufInfo, DoubleBufferState, EglContext, ExportFormat, SharedRenderScratch,
    TwoSlotBufferCache, WidgetExportBuffer,
};
use bmc_widget::surface::ReleasedBuffer;
use glow::HasContext as _;
use wayland_client::protocol::wl_buffer;

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

/// Double-buffered DMA-BUF render target with `wl_buffer.release` tracking.
///
/// Pairs [`DoubleBufferState`] (two lazily-allocated export buffers) with a
/// cache of the minted `wl_buffer`s and release state so that a
/// compositor `wl_buffer.release` frees the matching export slot for reuse.
#[expect(missing_debug_implementations)]
pub struct OverlayRenderTarget {
    buffers: DoubleBufferState,
    wl_buffers: TwoSlotBufferCache<wl_buffer::WlBuffer>,
    /// Once-painted panel source for the blit-only slide: an overlay paints the
    /// panel band into this GL texture/FBO once (and again only when content
    /// changes), then each animation frame copies it into the export buffer at
    /// the current slide offset. `None` until first captured; retained across
    /// hides so the next reveal can blit immediately, freed on resize (stale
    /// size) and destroy.
    panel_cache: Option<WidgetExportBuffer>,
}

impl OverlayRenderTarget {
    /// Create an empty render target at `w`×`h`. Export buffers are allocated
    /// lazily on the first [`Self::ensure_current`]. The `_egl` argument is
    /// accepted to match the host factory; allocation is lazy inside
    /// `ensure_current`.
    pub fn new(_egl: &EglContext, w: u32, h: u32) -> anyhow::Result<Self> {
        Ok(Self {
            buffers: DoubleBufferState::new_with_format(w, h, Depth::Disabled, ExportFormat::Alpha),
            wl_buffers: TwoSlotBufferCache::new(),
            panel_cache: None,
        })
    }

    /// Ensure the current back-buffer slot is allocated and ready to render to.
    pub fn ensure_current(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        self.buffers.ensure_current(egl)?;
        Ok(())
    }

    /// GL framebuffer of the current back buffer. Must be called after
    /// [`Self::ensure_current`].
    #[must_use]
    pub fn current_fbo(&self) -> glow::Framebuffer {
        self.buffers
            .current_ref()
            .expect("BUG: current_fbo called before ensure_current succeeded")
            .fbo
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

    /// Capture the just-painted panel band from `scratch`'s staging into the
    /// cache source (GPU→GPU shader copy, no CPU read-back). Allocates the cache
    /// lazily, reallocating if the band size changed. Call right after the
    /// overlay's `render` + `flush`, inside the GPU render lock, gated on a
    /// content change. The cache then holds the upright panel for
    /// [`Self::blit_cached_panel`] to slide.
    pub fn capture_panel(
        &mut self,
        egl: &EglContext,
        scratch: &SharedRenderScratch,
        w: u32,
        panel_h: u32,
    ) -> anyhow::Result<()> {
        let needs_alloc = self
            .panel_cache
            .as_ref()
            .is_none_or(|c| c.width != w || c.height != panel_h);
        if needs_alloc {
            if let Some(old) = self.panel_cache.take() {
                egl.destroy_widget_export_buffer(old);
            }
            self.panel_cache = Some(
                egl.allocate_widget_export_buffer(w, panel_h, Depth::Disabled)
                    .context("allocate overlay panel cache")?,
            );
        }
        let cache = self
            .panel_cache
            .as_ref()
            .expect("BUG: panel cache allocated above");
        scratch.blit_to(egl, cache.fbo(), w, panel_h);
        Ok(())
    }

    /// Whether a captured panel matching `size` is ready to blit. Size-checked:
    /// a cache retained across hides may predate a surface resize.
    #[must_use]
    pub fn cached_ready(&self, size: (u32, u32)) -> bool {
        self.panel_cache
            .as_ref()
            .is_some_and(|c| c.width == size.0 && c.height == size.1)
    }

    /// Present an animation frame by copying the cached panel into the current
    /// export buffer translated by `offset_y` (clear-transparent + shader copy
    /// at offset). No layout, no femtovg. Must be called after
    /// [`Self::ensure_current`]; returns an error if no panel has been captured.
    pub fn blit_cached_panel(
        &self,
        egl: &EglContext,
        scratch: &SharedRenderScratch,
        export_fbo: glow::Framebuffer,
        size: (u32, u32),
        panel_h: f32,
        offset_y: f32,
    ) -> anyhow::Result<()> {
        let cache = self
            .panel_cache
            .as_ref()
            .context("blit_cached_panel called before capture_panel")?;
        scratch.blit_texture_at_offset(egl, cache.texture(), export_fbo, size, panel_h, offset_y);
        Ok(())
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
        if let Some(cache) = self.panel_cache.take() {
            egl.destroy_widget_export_buffer(cache);
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
    /// keep the target reusable: a later `ensure_current` reallocates lazily.
    /// The panel cache is retained (not freed here) so the next reveal can
    /// blit it instead of full-painting; [`Self::destroy`] is the terminal
    /// cleanup that frees it.
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
        if let Some(cache) = self.panel_cache.take() {
            egl.destroy_widget_export_buffer(cache);
        }
        for buffer in self.wl_buffers.take_all().into_iter().flatten() {
            buffer.destroy();
        }
    }
}

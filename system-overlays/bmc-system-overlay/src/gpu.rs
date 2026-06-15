// Copyright (C) 2026  Braiins Systems s.r.o.

//! GPU frame helpers for system overlays: a GL-fence wait that makes an
//! exported DMA-BUF safe to hand to the compositor, and a double-buffered
//! render target wrapping `bmc_widget`'s export-buffer machinery with the
//! `wl_buffer.release` bookkeeping the compositor drives.

use anyhow::Context as _;
use bmc_widget::egl::{
    Depth, DmaBufInfo, DoubleBufferState, EglContext, ExportFormat, SharedRenderScratch,
    SlotReleaseState, WidgetExportBuffer,
};
use bmc_widget::surface::ReleasedBuffer;
use glow::HasContext as _;
use wayland_client::protocol::wl_buffer;

/// Maximum time a single `client_wait_sync` poll blocks before looping.
const FENCE_WAIT_TIMEOUT_NS: i32 = 1_000_000;

/// Stall the CPU until the GPU has finished the submitted commands, so the
/// exported DMA-BUF is safe to hand to the compositor. Mirrors the host's
/// `flush_and_wait_gl`. Uses a GL fence sync when available, else `glFinish`.
pub fn wait_for_gpu(egl: &EglContext) {
    // SAFETY: `EglContext::new` makes its context current on the creating
    // thread and keeps it current for its lifetime; overlays render on that
    // same thread.
    let gl = egl.gl();
    let fence = match unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0) } {
        Ok(fence) => fence,
        Err(e) => {
            tracing::warn!(?e, "GL fence creation failed; falling back to glFinish");
            unsafe {
                gl.finish();
            }
            return;
        }
    };
    unsafe {
        gl.flush();
    }
    loop {
        match unsafe { gl.client_wait_sync(fence, 0, FENCE_WAIT_TIMEOUT_NS) } {
            glow::ALREADY_SIGNALED | glow::CONDITION_SATISFIED => break,
            glow::TIMEOUT_EXPIRED => continue,
            glow::WAIT_FAILED => {
                tracing::warn!("GL fence wait failed; falling back to glFinish");
                unsafe {
                    gl.finish();
                }
                break;
            }
            status => {
                tracing::warn!(status, "GL fence wait returned unexpected status");
                unsafe {
                    gl.finish();
                }
                break;
            }
        }
    }
    unsafe {
        gl.delete_sync(fence);
    }
}

/// Double-buffered DMA-BUF render target with `wl_buffer.release` tracking.
///
/// Pairs [`DoubleBufferState`] (two lazily-allocated export buffers) with a
/// cache of the minted `wl_buffer`s and a [`SlotReleaseState`] so that a
/// compositor `wl_buffer.release` frees the matching export slot for reuse.
#[expect(missing_debug_implementations)]
pub struct OverlayRenderTarget {
    buffers: DoubleBufferState,
    wl_buffers: [Option<wl_buffer::WlBuffer>; 2],
    release: SlotReleaseState,
    /// Once-painted panel source for the blit-only slide: an overlay paints the
    /// panel band into this GL texture/FBO once (and again only when content
    /// changes), then each animation frame copies it into the export buffer at
    /// the current slide offset. `None` until first captured; freed on hide so
    /// no fullscreen allocation survives an unmap.
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
            wl_buffers: [None, None],
            release: SlotReleaseState::new(),
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

    /// Whether a panel source has been captured and can be blitted.
    #[must_use]
    pub fn is_cached(&self) -> bool {
        self.panel_cache.is_some()
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
        for wl_buffer in &mut self.wl_buffers {
            if let Some(buffer) = wl_buffer.take() {
                client.destroy_minted_wl_buffer(buffer);
            }
        }
        self.release = SlotReleaseState::new();
        self.buffers.resize(egl, w, h);
        client.flush()
    }

    /// True when the next export slot's buffer has been released by the
    /// compositor (or has never been submitted).
    #[must_use]
    pub fn available(&self) -> bool {
        self.release.is_available(self.buffers.current_slot())
    }

    /// Record that `slot`'s buffer has been submitted to the compositor and is
    /// pinned until its `wl_buffer.release`.
    pub fn mark_presented(&mut self, slot: usize) {
        self.release.mark_presented(slot);
    }

    /// Translate a compositor `wl_buffer.release` into a freed export slot by
    /// matching the released buffer against the cached `wl_buffer`s.
    pub fn mark_released_buffer(&mut self, released: &ReleasedBuffer) {
        for (slot, wl_buffer) in self.wl_buffers.iter().enumerate() {
            if wl_buffer
                .as_ref()
                .is_some_and(|buffer| released.matches(buffer))
            {
                self.release.mark_released(slot);
                return;
            }
        }
    }

    /// Mint (once) and cache the `wl_buffer` for `slot` via the layer-surface
    /// client. Subsequent calls return the cached buffer.
    pub fn wl_buffer_for_slot(
        &mut self,
        client: &mut crate::surface::LayerSurfaceClient,
        info: &DmaBufInfo,
        slot: usize,
    ) -> anyhow::Result<wl_buffer::WlBuffer> {
        let wl_buffer = self
            .wl_buffers
            .get_mut(slot)
            .with_context(|| format!("invalid export slot id: {slot}"))?;
        if wl_buffer.is_none() {
            *wl_buffer = Some(client.mint_wl_buffer(info, slot)?);
        }
        Ok(wl_buffer
            .as_ref()
            .expect("BUG: wl_buffer should exist after mint above")
            .clone())
    }

    /// Free the GBM/GL export buffers and cached `wl_buffer`s for a hide, but
    /// keep the target reusable: a later `ensure_current` reallocates lazily.
    /// Distinct from [`Self::destroy`], which is terminal (shutdown only).
    pub fn free_for_hide(
        &mut self,
        egl: &EglContext,
        client: &mut crate::surface::LayerSurfaceClient,
    ) -> anyhow::Result<()> {
        let cached_wl_buffers = self
            .wl_buffers
            .iter()
            .filter(|buffer| buffer.is_some())
            .count();
        tracing::info!(cached_wl_buffers, "free_for_hide: freeing overlay buffers");
        self.buffers.destroy_all(egl);
        if let Some(cache) = self.panel_cache.take() {
            egl.destroy_widget_export_buffer(cache);
        }
        for wl_buffer in &mut self.wl_buffers {
            if let Some(buffer) = wl_buffer.take() {
                client.destroy_minted_wl_buffer(buffer);
            }
        }
        self.release = SlotReleaseState::new();
        client.flush()
    }

    /// Free all GL/EGL/GBM resources and destroy the cached `wl_buffer`s.
    ///
    /// [`DoubleBufferState`] does not clean up on `Drop`, so the owner must
    /// call this explicitly while the EGL context is still current.
    pub fn destroy(&mut self, egl: &EglContext) {
        self.buffers.destroy_all(egl);
        if let Some(cache) = self.panel_cache.take() {
            egl.destroy_widget_export_buffer(cache);
        }
        for wl_buffer in &mut self.wl_buffers {
            if let Some(buffer) = wl_buffer.take() {
                buffer.destroy();
            }
        }
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_gpu_render_lock::{GpuRenderLock, GpuRenderLockGuard};
use bmc_render::gpu::FemtoVgRenderer;
use bmc_widget::egl::{EglContext, SharedRenderScratch};
use glow::HasContext;

/// State shared across all widget slots within a host.
///
/// Aliasing invariant: `SharedHost` does NOT own the `Renderer`. The
/// renderer is owned by `main_loop::run` and parked behind a `NonNull` for
/// host imports to reborrow. Adding a `Renderer` (or anything containing
/// one) here would alias the parked pointer when the main loop passes
/// `&mut shared` and the parked `NonNull<dyn Renderer>` to the same call,
/// producing UB.
#[expect(missing_debug_implementations)]
pub struct SharedHost {
    pub egl: EglContext,
    pub scratch: SharedRenderScratch,
    pub font_cache: FontCache,
    gpu_render_lock: GpuRenderLock,
}

#[derive(Debug, Default)]
pub struct FontCache;

impl SharedHost {
    pub fn init(display_max_w: u32, display_max_h: u32) -> anyhow::Result<(Self, FemtoVgRenderer)> {
        tracing::info!(
            display_max_w,
            display_max_h,
            "initializing shared wasm host renderer"
        );
        let egl = EglContext::new()?;
        let scratch = SharedRenderScratch::new(&egl, display_max_w, display_max_h)?;
        let gpu_render_lock = GpuRenderLock::from_env()?;
        let renderer = unsafe {
            FemtoVgRenderer::new(
                |sym: &str| EglContext::get_proc_address(sym),
                display_max_w,
                display_max_h,
                scratch.staging_fbo_id(),
                0,
            )?
        };
        tracing::info!("shared wasm host renderer initialized");
        Ok((
            Self {
                egl,
                scratch,
                font_cache: FontCache,
                gpu_render_lock,
            },
            renderer,
        ))
    }

    /// Blit the staging color attachment into `dest_fbo` at `(w, h)`.
    pub fn blit_staging_to(&self, dest_fbo: glow::Framebuffer, w: u32, h: u32) {
        self.scratch.blit_to(&self.egl, dest_fbo, w, h);
    }

    /// Submit pending GL commands to the driver without blocking.
    pub fn flush_gl(&self) {
        // SAFETY: `EglContext::new` calls `make_current`; the context remains
        // current on this thread for the lifetime of `SharedHost`.
        unsafe {
            self.egl.gl().flush();
        }
    }

    pub(crate) fn acquire_gpu_render_lock(
        &self,
        scope: &'static str,
    ) -> anyhow::Result<GpuRenderLockGuard> {
        self.gpu_render_lock.lock(scope)
    }

    /// Whether the EGL context has been reported lost (e.g., GPU reset).
    pub fn is_context_lost(&self) -> bool {
        self.egl.is_context_lost()
    }
}

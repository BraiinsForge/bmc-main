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

use bmc_gpu_render_lock::{
    GlSyncEntryPoints, GpuCompletionWaitStrategy, GpuRenderLock, GpuRenderLockGuard,
    detect_gpu_completion_wait_strategy,
};
use bmc_render::gpu::FemtoVgRenderer;
use bmc_widget::egl::{EglContext, SharedRenderScratch};
use glow::HasContext;

use crate::module_cache::ModuleCache;

const FENCE_WAIT_TIMEOUT_NS: i32 = 1_000_000;

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
    pub(crate) module_cache: ModuleCache,
    /// Path to the device-wide image decode lock.
    pub image_decode_lock_path: std::path::PathBuf,
    gpu_render_lock: GpuRenderLock,
    gl_sync_support: GpuCompletionWaitStrategy,
}

#[derive(Debug, Default)]
pub struct FontCache;

impl SharedHost {
    pub fn init(
        display_max_w: u32,
        display_max_h: u32,
        image_decode_lock_path: std::path::PathBuf,
    ) -> anyhow::Result<(Self, FemtoVgRenderer)> {
        tracing::info!(
            display_max_w,
            display_max_h,
            "initializing shared wasm host renderer"
        );
        let gpu_render_lock = GpuRenderLock::from_env()?;
        let init_guard = gpu_render_lock.lock("host_init")?;
        let egl = EglContext::new()?;
        let gl_sync_support = Self::detect_gl_sync_support(&egl);
        let scratch = SharedRenderScratch::new(&egl, display_max_w, display_max_h)?;
        let renderer = unsafe {
            FemtoVgRenderer::new(
                |sym: &str| EglContext::get_proc_address(sym),
                display_max_w,
                display_max_h,
                scratch.staging_fbo_id(),
                0,
            )?
        };
        // SAFETY: `EglContext::new` made this context current on the host thread.
        unsafe {
            egl.gl().finish();
        }
        drop(init_guard);
        tracing::info!("shared wasm host renderer initialized");
        Ok((
            Self {
                egl,
                scratch,
                font_cache: FontCache,
                module_cache: ModuleCache::new(),
                image_decode_lock_path,
                gpu_render_lock,
                gl_sync_support,
            },
            renderer,
        ))
    }

    /// Blit the staging color attachment into `dest_fbo` at `(w, h)`.
    pub fn blit_staging_to(&self, dest_fbo: glow::Framebuffer, w: u32, h: u32) {
        self.scratch.blit_to(&self.egl, dest_fbo, w, h);
    }

    /// Submit pending GL commands and wait for GPU completion before handing
    /// exported buffers to the compositor.
    pub fn flush_and_wait_gl(&self) {
        match self.gl_sync_support {
            GpuCompletionWaitStrategy::GlFenceSync => self.wait_for_gl_fence(),
            GpuCompletionWaitStrategy::EglFenceSync => self.wait_for_egl_fence(),
            GpuCompletionWaitStrategy::Finish => unsafe {
                self.egl.gl().finish();
            },
        }
    }

    fn detect_gl_sync_support(egl: &EglContext) -> GpuCompletionWaitStrategy {
        // SAFETY: `EglContext::new` calls `make_current`; the context remains
        // current on this thread for the lifetime of `SharedHost`.
        let version = unsafe { egl.gl().get_parameter_string(glow::VERSION) };
        let gl_extensions = unsafe { egl.gl().get_parameter_string(glow::EXTENSIONS) };
        detect_gpu_completion_wait_strategy(
            &version,
            &gl_extensions,
            egl.egl_extensions(),
            GlSyncEntryPoints::load_with(EglContext::get_proc_address),
        )
    }

    fn wait_for_egl_fence(&self) {
        if let Err(e) = self.egl.wait_for_egl_fence() {
            tracing::warn!(?e, "EGL fence wait failed; falling back to glFinish");
            unsafe {
                self.egl.gl().finish();
            }
        }
    }

    fn wait_for_gl_fence(&self) {
        // SAFETY: `EglContext::new` calls `make_current`; the context remains
        // current on this thread for the lifetime of `SharedHost`.
        let gl = self.egl.gl();
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

    pub(crate) fn acquire_gpu_render_lock(
        &self,
        scope: &'static str,
    ) -> anyhow::Result<GpuRenderLockGuard> {
        self.gpu_render_lock.lock(scope)
    }

    pub(crate) fn with_gpu_render_lock<T>(
        &mut self,
        scope: &'static str,
        operation: impl FnOnce(&mut Self) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let guard = self.acquire_gpu_render_lock(scope)?;
        let result = operation(self);
        self.flush_and_wait_gl();
        drop(guard);
        result
    }

    /// Whether the EGL context has been reported lost (e.g., GPU reset).
    pub fn is_context_lost(&self) -> bool {
        self.egl.is_context_lost()
    }
}

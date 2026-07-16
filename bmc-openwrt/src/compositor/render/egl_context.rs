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

//! EGL/OpenGL ES context and rendering.

use anyhow::{Context, Result};
use bmc_gpu_render_lock::{
    GlSyncEntryPoints, GpuCompletionWaitStrategy, detect_gpu_completion_wait_strategy,
};
use smithay::{
    backend::{
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay, fence::EGLFence},
        renderer::gles::{GlesRenderer, ffi},
    },
    reexports::gbm::Device as GbmDevice,
};
use std::{ffi::CStr, fs::OpenOptions, os::unix::io::OwnedFd, path::Path};

const FENCE_WAIT_TIMEOUT_NS: u64 = 1_000_000;

pub struct EglContext {
    // Kept alive for EGL display lifetime
    _gpu_gbm: GbmDevice<DrmDeviceFd>,
    egl_display: EGLDisplay,
    renderer: GlesRenderer,
    gl_sync_support: GpuCompletionWaitStrategy,
}

impl EglContext {
    pub fn new(gpu_path: &Path) -> Result<Self> {
        tracing::info!("Opening GPU device: {:?}", gpu_path);

        let gpu_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(gpu_path)
            .context("Failed to open GPU device")?;

        let gpu_fd = DrmDeviceFd::new(OwnedFd::from(gpu_file).into());
        let gpu_gbm = GbmDevice::new(gpu_fd).context("Failed to create GBM device")?;

        let egl_display =
            unsafe { EGLDisplay::new(gpu_gbm.clone()) }.context("Failed to create EGL display")?;

        let egl_context = EGLContext::new(&egl_display).context("Failed to create EGL context")?;

        let mut renderer =
            unsafe { GlesRenderer::new(egl_context) }.context("Failed to create GLES renderer")?;
        let gl_sync_support = detect_gl_sync_support(&mut renderer, &egl_display)?;

        tracing::info!("EGL context initialized");

        Ok(Self {
            _gpu_gbm: gpu_gbm,
            egl_display,
            renderer,
            gl_sync_support,
        })
    }

    pub fn renderer(&mut self) -> &mut GlesRenderer {
        &mut self.renderer
    }

    pub fn wait_for_rendering_completion(&mut self) -> Result<()> {
        match self.gl_sync_support {
            GpuCompletionWaitStrategy::GlFenceSync => self.wait_for_gl_fence()?,
            GpuCompletionWaitStrategy::EglFenceSync => self.wait_for_egl_fence()?,
            GpuCompletionWaitStrategy::Finish => self.finish_rendering()?,
        }
        Ok(())
    }

    fn finish_rendering(&mut self) -> Result<()> {
        unsafe {
            self.renderer.with_context(|gl| {
                gl.Finish();
            })?;
        }
        Ok(())
    }

    fn wait_for_egl_fence(&mut self) -> Result<()> {
        let wait_result = self.renderer.with_context(|_| {
            let fence =
                EGLFence::create(&self.egl_display).context("Failed to create EGL fence")?;
            let completed = fence
                .client_wait(None, true)
                .context("Failed to wait for EGL fence")?;
            anyhow::ensure!(completed, "EGL fence wait returned before completion");
            Ok(())
        })?;
        if let Err(e) = wait_result {
            tracing::warn!(?e, "EGL fence wait failed; falling back to glFinish");
            self.finish_rendering()?;
        }
        Ok(())
    }

    fn wait_for_gl_fence(&mut self) -> Result<()> {
        unsafe {
            self.renderer.with_context(|gl| {
                let fence = gl.FenceSync(ffi::SYNC_GPU_COMMANDS_COMPLETE, 0);
                if fence.is_null() {
                    tracing::warn!("GL fence creation returned null; falling back to glFinish");
                    gl.Finish();
                    return;
                }

                gl.Flush();
                loop {
                    match gl.ClientWaitSync(fence, 0, FENCE_WAIT_TIMEOUT_NS) {
                        ffi::ALREADY_SIGNALED | ffi::CONDITION_SATISFIED => break,
                        ffi::TIMEOUT_EXPIRED => continue,
                        ffi::WAIT_FAILED => {
                            tracing::warn!("GL fence wait failed; falling back to glFinish");
                            gl.Finish();
                            break;
                        }
                        status => {
                            tracing::warn!(status, "GL fence wait returned unexpected status");
                            gl.Finish();
                            break;
                        }
                    }
                }
                gl.DeleteSync(fence);
            })?;
        }
        Ok(())
    }
}

fn detect_gl_sync_support(
    renderer: &mut GlesRenderer,
    egl_display: &EGLDisplay,
) -> Result<GpuCompletionWaitStrategy> {
    let (version, gl_extensions) = unsafe {
        renderer.with_context(|gl| {
            let version = gl.GetString(ffi::VERSION);
            let extensions = gl.GetString(ffi::EXTENSIONS);
            (
                gl_string(version).unwrap_or_else(|| "<unavailable>".to_owned()),
                gl_string(extensions).unwrap_or_default(),
            )
        })?
    };
    Ok(detect_gpu_completion_wait_strategy(
        &version,
        &gl_extensions,
        egl_display.extensions(),
        GlSyncEntryPoints::load_with(|name| unsafe {
            smithay::backend::egl::get_proc_address(name)
        }),
    ))
}

fn gl_string(value: *const ffi::types::GLubyte) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(value.cast()) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

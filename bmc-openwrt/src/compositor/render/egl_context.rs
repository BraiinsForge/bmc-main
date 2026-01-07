// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL/OpenGL ES context and rendering.

use anyhow::{Context, Result};
use smithay::{
    backend::{
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
        renderer::gles::GlesRenderer,
    },
    reexports::gbm::Device as GbmDevice,
};
use std::{fs::OpenOptions, os::unix::io::OwnedFd, path::Path};

pub struct EglContext {
    // Kept alive for EGL display lifetime
    _gpu_gbm: GbmDevice<DrmDeviceFd>,
    // Kept alive for renderer lifetime
    _egl_display: EGLDisplay,
    renderer: GlesRenderer,
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

        let renderer =
            unsafe { GlesRenderer::new(egl_context) }.context("Failed to create GLES renderer")?;

        tracing::info!("EGL context initialized");

        Ok(Self {
            _gpu_gbm: gpu_gbm,
            _egl_display: egl_display,
            renderer,
        })
    }

    pub fn renderer(&mut self) -> &mut GlesRenderer {
        &mut self.renderer
    }

    pub fn finish_rendering(&mut self) -> Result<()> {
        unsafe {
            self.renderer.with_context(|gl| gl.Finish())?;
        }
        Ok(())
    }
}

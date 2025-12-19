// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! EGL context management for OpenGL ES rendering via GBM
//!
//! This module uses GBM (Generic Buffer Manager) to create EGL context
//! and render to buffers that can be exported as DMA-BUF for Wayland.
//!
//! Uses EGLImage to connect GBM buffer objects to OpenGL textures,
//! enabling zero-copy rendering to DMA-BUF exportable buffers.

use anyhow::{Context, Result};
use drm_fourcc::{DrmFourcc, DrmModifier};
use glow::HasContext;
use smithay::{
    backend::{
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
    },
    reexports::gbm::{AsRaw, BufferObject, BufferObjectFlags, Device as GbmDevice},
};
use std::{ffi::c_void, fs::OpenOptions, os::fd::OwnedFd, ptr};

/// Default GPU render node
const GPU_PATH: &str = "/dev/dri/renderD128";

// EGL constants
const EGL_NATIVE_PIXMAP_KHR: u32 = 0x30B0;
const EGL_NONE: i32 = 0x3038;
const EGL_NO_IMAGE: *mut c_void = ptr::null_mut();

// GL_OES_EGL_image extension
const GL_TEXTURE_2D: u32 = 0x0DE1;

// Type aliases for EGL function pointers
type EglCreateImageKhr = unsafe extern "C" fn(
    dpy: *mut c_void,
    ctx: *mut c_void,
    target: u32,
    buffer: *mut c_void,
    attrib_list: *const i32,
) -> *mut c_void;

type GlEglImageTargetTexture2DOes = unsafe extern "C" fn(target: u32, image: *mut c_void);

/// EGL state for OpenGL ES rendering via GBM
pub struct EglState {
    /// GBM device for buffer allocation
    gbm: GbmDevice<DrmDeviceFd>,
    /// EGL display handle (raw pointer for EGL calls)
    egl_display_raw: *mut c_void,
    /// EGL display (smithay wrapper, kept for lifetime)
    #[expect(dead_code, reason = "kept alive for context lifetime")]
    egl_display: EGLDisplay,
    /// EGL context
    #[expect(dead_code, reason = "kept alive for GL operations")]
    egl_context: EGLContext,
    /// OpenGL ES context via glow
    gl: glow::Context,
    /// EGL extension: eglCreateImageKHR
    egl_create_image: EglCreateImageKhr,
    /// GL extension: glEGLImageTargetTexture2DOES
    gl_image_target_texture: GlEglImageTargetTexture2DOes,
    /// Render buffers (double buffering)
    buffers: [Option<RenderBuffer>; 2],
    /// Current back buffer index
    current_buffer: usize,
    /// Surface width
    width: u32,
    /// Surface height
    height: u32,
}

/// A render buffer with GBM BO, EGLImage, and OpenGL FBO
struct RenderBuffer {
    /// GBM buffer object
    bo: BufferObject<()>,
    /// EGLImage handle
    egl_image: *mut c_void,
    /// OpenGL texture (color attachment)
    texture: glow::Texture,
    /// OpenGL framebuffer object
    fbo: glow::Framebuffer,
}

impl EglState {
    /// Create EGL context using GBM device
    pub fn new(width: u32, height: u32) -> Result<Self> {
        tracing::info!(
            "Initializing GBM-based EGL for {}x{} rendering",
            width,
            height
        );

        // Open GPU device
        let gpu_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(GPU_PATH)
            .context("Failed to open GPU device")?;

        // Use DrmDeviceFd which is cloneable (required by smithay's EGLDisplay::new)
        let gpu_fd = DrmDeviceFd::new(OwnedFd::from(gpu_file).into());
        tracing::debug!("Opened GPU device: {}", GPU_PATH);

        // Create GBM device
        let gbm = GbmDevice::new(gpu_fd).context("Failed to create GBM device")?;
        tracing::debug!("GBM device created");

        // Create EGL display from GBM using Smithay's wrapper
        let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }.context(
            "Failed to create EGL display - check Mesa/etnaviv drivers and GBM_BACKENDS_PATH",
        )?;

        // Get raw EGL display handle for extension calls
        let egl_display_raw = egl_display.get_display_handle().handle.cast_mut();
        tracing::info!("EGL display created: {:?}", egl_display_raw);

        // Create EGL context
        let egl_context = EGLContext::new(&egl_display).context("Failed to create EGL context")?;
        tracing::info!("EGL context created");

        // Make context current (surfaceless)
        let _ = unsafe { egl_context.make_current() };

        // Load EGL extension: eglCreateImageKHR
        let egl_create_image: EglCreateImageKhr = unsafe {
            let proc = smithay::backend::egl::get_proc_address("eglCreateImageKHR");
            if proc.is_null() {
                anyhow::bail!("eglCreateImageKHR not available");
            }
            std::mem::transmute(proc)
        };
        tracing::debug!("Loaded eglCreateImageKHR");

        // Load GL extension: glEGLImageTargetTexture2DOES
        let gl_image_target_texture: GlEglImageTargetTexture2DOes = unsafe {
            let proc = smithay::backend::egl::get_proc_address("glEGLImageTargetTexture2DOES");
            if proc.is_null() {
                anyhow::bail!("glEGLImageTargetTexture2DOES not available");
            }
            std::mem::transmute(proc)
        };
        tracing::debug!("Loaded glEGLImageTargetTexture2DOES");

        // Create glow context for OpenGL ES calls
        let gl = unsafe {
            glow::Context::from_loader_function(|symbol| {
                smithay::backend::egl::get_proc_address(symbol)
            })
        };

        // Log OpenGL ES info
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        tracing::info!("OpenGL ES: {} ({})", version, renderer);

        Ok(Self {
            gbm,
            egl_display_raw,
            egl_display,
            egl_context,
            gl,
            egl_create_image,
            gl_image_target_texture,
            buffers: [None, None],
            current_buffer: 0,
            width,
            height,
        })
    }

    /// Allocate a render buffer (GBM BO + EGLImage + FBO)
    #[expect(clippy::cast_possible_wrap, reason = "GL constants fit in i32")]
    fn allocate_buffer(&mut self) -> Result<RenderBuffer> {
        use smithay::reexports::gbm::Format;

        tracing::debug!(
            "Allocating {}x{} GBM buffer with EGLImage",
            self.width,
            self.height
        );

        // Create GBM buffer object
        let bo = self
            .gbm
            .create_buffer_object::<()>(
                self.width,
                self.height,
                Format::Xrgb8888,
                BufferObjectFlags::RENDERING | BufferObjectFlags::LINEAR,
            )
            .context("Failed to create GBM buffer object")?;

        tracing::debug!(
            "GBM BO created: {}x{}, stride={}",
            bo.width(),
            bo.height(),
            bo.stride()
        );

        // Create EGLImage from GBM BO
        // EGL_NO_CONTEXT is used because the image is not bound to a specific context
        let attribs = [EGL_NONE];
        let egl_image = unsafe {
            (self.egl_create_image)(
                self.egl_display_raw,
                ptr::null_mut(), // EGL_NO_CONTEXT
                EGL_NATIVE_PIXMAP_KHR,
                bo.as_raw() as *mut c_void,
                attribs.as_ptr(),
            )
        };

        if egl_image == EGL_NO_IMAGE {
            anyhow::bail!("Failed to create EGLImage from GBM BO");
        }
        tracing::debug!("EGLImage created: {:?}", egl_image);

        // Create OpenGL texture
        let texture = unsafe {
            self.gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("Failed to create texture: {e}"))?
        };

        // Bind EGLImage to texture using GL_OES_EGL_image extension
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            (self.gl_image_target_texture)(GL_TEXTURE_2D, egl_image);

            // Set texture parameters
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
        }
        tracing::debug!("Texture bound to EGLImage");

        // Create OpenGL framebuffer object
        let fbo = unsafe {
            self.gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create framebuffer: {e}"))?
        };

        // Attach texture to framebuffer
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            // Check framebuffer completeness
            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Framebuffer incomplete: 0x{status:x}");
            }
        }

        tracing::debug!("Framebuffer created and complete with EGLImage-backed texture");

        Ok(RenderBuffer {
            bo,
            egl_image,
            texture,
            fbo,
        })
    }

    /// Begin a frame - bind the back buffer for rendering
    pub fn begin_frame(&mut self) -> Result<()> {
        // Ensure buffer is allocated
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(self.allocate_buffer()?);
        }

        // Get the FBO from the buffer
        let fbo = self.buffers[idx]
            .as_ref()
            .expect("BUG: buffer should exist after allocation")
            .fbo;

        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
            self.gl
                .viewport(0, 0, self.width as i32, self.height as i32);
        }
        Ok(())
    }

    /// Clear the screen with a color
    pub fn clear(&self, r: f32, g: f32, b: f32, a: f32) {
        unsafe {
            self.gl.clear_color(r, g, b, a);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// End frame and get DMA-BUF fd for the rendered buffer
    pub fn end_frame(&mut self) -> Result<DmaBufInfo> {
        // Ensure rendering is complete
        unsafe {
            self.gl.finish();
        }

        let idx = self.current_buffer;
        let buffer = self.buffers[idx]
            .as_ref()
            .expect("BUG: buffer should exist after begin_frame");

        // Export GBM BO as DMA-BUF
        let fd = buffer
            .bo
            .fd()
            .context("Failed to get DMA-BUF fd from GBM BO")?;

        let info = DmaBufInfo {
            fd,
            width: self.width,
            height: self.height,
            stride: buffer.bo.stride(),
            format: DrmFourcc::Xrgb8888,
            modifier: DrmModifier::Linear,
        };

        // Swap buffers for next frame
        self.current_buffer = 1 - self.current_buffer;

        Ok(info)
    }

    /// Resize buffers (deallocates existing buffers)
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }

        tracing::debug!(
            "Resizing from {}x{} to {}x{}",
            self.width,
            self.height,
            width,
            height
        );

        // Deallocate existing buffers
        for buffer in &mut self.buffers {
            if let Some(buf) = buffer.take() {
                unsafe {
                    self.gl.delete_framebuffer(buf.fbo);
                    self.gl.delete_texture(buf.texture);
                    // Note: EGLImage should be destroyed with eglDestroyImageKHR
                    // but we don't have that loaded - buffers will be cleaned up on exit
                    let _ = buf.egl_image;
                }
            }
        }

        self.width = width;
        self.height = height;
    }

    /// Get the glow OpenGL ES context
    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// Get current dimensions
    #[expect(dead_code)]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Information about a DMA-BUF export
pub struct DmaBufInfo {
    /// DMA-BUF file descriptor (owned)
    pub fd: OwnedFd,
    /// Buffer width
    pub width: u32,
    /// Buffer height
    pub height: u32,
    /// Buffer stride (bytes per row)
    pub stride: u32,
    /// Pixel format
    pub format: DrmFourcc,
    /// Buffer modifier
    pub modifier: DrmModifier,
}

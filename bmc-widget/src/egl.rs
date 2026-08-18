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

//! GBM/EGL rendering backend for DMA-BUF widget surfaces.
//!
//! Provides the common GPU infrastructure shared by all GPU-rendering widgets:
//! GPU device setup, EGL context creation, and DMA-BUF-exportable buffer
//! management. Widgets compose their own rendering pipeline on top — direct
//! FBO rendering (flip-clock) or two-FBO with staging/blit (wasm).

use std::ffi::c_void;
use std::fmt;
use std::fs::OpenOptions;
use std::os::fd::OwnedFd;
use std::ptr;

use anyhow::{Context, Result};
use drm_fourcc::{DrmFourcc, DrmModifier};
use glow::HasContext;
use smithay::{
    backend::{
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay, fence::EGLFence},
    },
    reexports::gbm::{
        AsRaw, BufferObject, BufferObjectFlags, Device as GbmDevice, Format as GbmFormat,
    },
};

/// Default GPU render node.
const GPU_PATH: &str = "/dev/dri/renderD128";

// EGL constants
const EGL_NATIVE_PIXMAP_KHR: u32 = 0x30B0;
const EGL_NONE: i32 = 0x3038;
const EGL_NO_IMAGE: *mut c_void = ptr::null_mut();
const EGL_CONTEXT_LOST: i32 = 0x300E;

// GL_OES_EGL_image extension
const GL_TEXTURE_2D: u32 = 0x0DE1;

// EGL/GL extension function pointer types
type EglGetError = unsafe extern "C" fn() -> i32;

type EglCreateImageKhr = unsafe extern "C" fn(
    dpy: *mut c_void,
    ctx: *mut c_void,
    target: u32,
    buffer: *mut c_void,
    attrib_list: *const i32,
) -> *mut c_void;

type EglDestroyImageKhr = unsafe extern "C" fn(dpy: *mut c_void, image: *mut c_void) -> i32;

type GlEglImageTargetTexture2DOes = unsafe extern "C" fn(target: u32, image: *mut c_void);

/// Whether an export buffer's FBO has a depth renderbuffer attached.
///
/// Required by widgets that use `GL_DEPTH_TEST` (e.g. 3D flip-clock); a
/// `DEPTH_COMPONENT16` renderbuffer costs ~1.3 MB per buffer at the
/// compositor's render size, so widgets that don't need depth opt out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Enabled,
    Disabled,
}

/// Pixel format used for DMA-BUF export buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Opaque RGB buffer. This is the default for widgets.
    Opaque,
    /// ARGB buffer preserving per-pixel alpha for composited overlays.
    Alpha,
}

impl ExportFormat {
    #[must_use]
    pub const fn drm_fourcc(self) -> DrmFourcc {
        match self {
            Self::Opaque => DrmFourcc::Xrgb8888,
            Self::Alpha => DrmFourcc::Argb8888,
        }
    }

    #[must_use]
    const fn gbm_format(self) -> GbmFormat {
        match self {
            Self::Opaque => GbmFormat::Xrgb8888,
            Self::Alpha => GbmFormat::Argb8888,
        }
    }
}

/// DMA-BUF export metadata for a rendered frame.
#[derive(Debug)]
pub struct DmaBufInfo {
    /// DMA-BUF file descriptor (owned — caller receives ownership).
    pub fd: OwnedFd,
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
    /// Buffer stride in bytes.
    pub stride: u32,
    /// Pixel format (DRM fourcc).
    pub format: DrmFourcc,
    /// Buffer modifier.
    pub modifier: DrmModifier,
}

/// Core EGL state — GPU device, EGL context, glow, extension function pointers.
///
/// Shared by all GPU widgets regardless of pipeline type. Widgets obtain an
/// `EglContext`, allocate [`ExportBuffer`]s for DMA-BUF output, and build
/// their own rendering pipeline (direct FBO, staging+blit, etc.) on top.
pub struct EglContext {
    /// GBM device for buffer allocation.
    gbm: GbmDevice<DrmDeviceFd>,
    /// Raw EGL display handle for extension calls.
    egl_display_raw: *mut c_void,
    /// EGL display (smithay wrapper, kept for lifetime).
    egl_display: EGLDisplay,
    /// EGL context (kept alive for GL operations).
    #[expect(dead_code, reason = "kept alive for GL operations")]
    context: EGLContext,
    /// OpenGL ES context via glow.
    gl: glow::Context,
    /// `eglGetError` for context-loss probing.
    egl_get_error: EglGetError,
    /// `eglCreateImageKHR` extension.
    egl_create_image: EglCreateImageKhr,
    /// `eglDestroyImageKHR` extension.
    egl_destroy_image: EglDestroyImageKhr,
    /// `glEGLImageTargetTexture2DOES` extension.
    gl_image_target_texture: GlEglImageTargetTexture2DOes,
}

impl fmt::Debug for EglContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EglContext")
            .field("egl_display_raw", &self.egl_display_raw)
            .finish_non_exhaustive()
    }
}

impl EglContext {
    /// Create a new EGL context backed by a GBM device.
    pub fn new() -> Result<Self> {
        tracing::info!("Initializing GBM-based EGL context");

        // Open GPU device
        let gpu_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(GPU_PATH)
            .context("Failed to open GPU device")?;

        let gpu_fd = DrmDeviceFd::new(OwnedFd::from(gpu_file).into());
        tracing::debug!("Opened GPU device: {GPU_PATH}");

        // Create GBM device
        let gbm = GbmDevice::new(gpu_fd).context("Failed to create GBM device")?;

        // Create EGL display from GBM
        let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }.context(
            "Failed to create EGL display — check Mesa/etnaviv drivers and GBM_BACKENDS_PATH",
        )?;

        let egl_display_raw = egl_display.get_display_handle().handle.cast_mut();
        tracing::info!("EGL display created: {egl_display_raw:?}");

        // Create EGL context
        let egl_context = EGLContext::new(&egl_display).context("Failed to create EGL context")?;
        tracing::info!("EGL context created");

        // Make context current (surfaceless)
        unsafe { egl_context.make_current() }
            .context("Failed to make EGL context current (surfaceless)")?;

        // Load EGL/GL extensions
        let egl_get_error: EglGetError = load_egl_proc("eglGetError")?;
        let egl_create_image: EglCreateImageKhr = load_egl_proc("eglCreateImageKHR")?;
        let egl_destroy_image: EglDestroyImageKhr = load_egl_proc("eglDestroyImageKHR")?;
        let gl_image_target_texture: GlEglImageTargetTexture2DOes =
            load_egl_proc("glEGLImageTargetTexture2DOES")?;

        // Create glow context
        let gl = unsafe {
            glow::Context::from_loader_function(|symbol| {
                smithay::backend::egl::get_proc_address(symbol)
            })
        };

        // Log GPU info
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        tracing::info!("OpenGL ES: {version} ({renderer})");

        Ok(Self {
            gbm,
            egl_display_raw,
            egl_display,
            context: egl_context,
            gl,
            egl_get_error,
            egl_create_image,
            egl_destroy_image,
            gl_image_target_texture,
        })
    }

    /// Get the glow OpenGL ES context.
    #[must_use]
    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// Return the EGL extension list reported by the display.
    #[must_use]
    pub fn egl_extensions(&self) -> &[String] {
        self.egl_display.extensions()
    }

    /// Submit pending GL commands and wait for completion with an EGL fence.
    pub fn wait_for_egl_fence(&self) -> Result<()> {
        let fence = EGLFence::create(&self.egl_display).context("Failed to create EGL fence")?;
        let completed = fence
            .client_wait(None, true)
            .context("Failed to wait for EGL fence")?;
        anyhow::ensure!(completed, "EGL fence wait returned before completion");
        Ok(())
    }

    /// Allocate an export buffer (GBM BO + EGLImage + GL texture + FBO).
    ///
    /// The returned [`ExportBuffer`] can be used as a render target and then
    /// exported as a DMA-BUF via [`Self::export_dmabuf`].
    pub fn allocate_export_buffer(
        &self,
        width: u32,
        height: u32,
        depth: Depth,
    ) -> Result<ExportBuffer> {
        self.allocate_export_buffer_with_format(width, height, depth, ExportFormat::Opaque)
    }

    /// Allocate an export buffer using an explicit DMA-BUF pixel format.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    #[expect(clippy::too_many_lines, reason = "linear GL resource setup")]
    pub fn allocate_export_buffer_with_format(
        &self,
        width: u32,
        height: u32,
        depth: Depth,
        format: ExportFormat,
    ) -> Result<ExportBuffer> {
        anyhow::ensure!(
            width > 0 && height > 0,
            "buffer dimensions must be non-zero"
        );

        tracing::debug!("Allocating {width}x{height} {format:?} GBM export buffer");

        // Prefer a Vivante-tiled buffer: the GC400 texture unit cannot sample
        // linear, so a linear export forces the compositor's driver to keep
        // a full-size tiled shadow copy per imported buffer.
        let bo = self
            .gbm
            .create_buffer_object_with_modifiers2::<()>(
                width,
                height,
                format.gbm_format(),
                [DrmModifier::Vivante_tiled].into_iter(),
                BufferObjectFlags::RENDERING,
            )
            .or_else(|err| {
                tracing::debug!("Tiled allocation unavailable ({err}); using linear");
                self.gbm.create_buffer_object::<()>(
                    width,
                    height,
                    format.gbm_format(),
                    BufferObjectFlags::RENDERING | BufferObjectFlags::LINEAR,
                )
            })
            .context("Failed to create GBM buffer object")?;

        // Legacy (non-modifier) allocations may report Invalid; they are
        // linear by construction.
        let modifier = bo.modifier();
        let modifier = if modifier == DrmModifier::Invalid {
            DrmModifier::Linear
        } else {
            modifier
        };

        tracing::debug!(
            "GBM BO created: {}x{}, stride={}, modifier={:?}",
            bo.width(),
            bo.height(),
            bo.stride(),
            modifier
        );

        // Create EGLImage from GBM BO
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

        // Create GL texture backed by the EGLImage
        let texture = unsafe {
            let tex = self
                .gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("Failed to create texture: {e}"))?;
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            (self.gl_image_target_texture)(GL_TEXTURE_2D, egl_image);

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
            tex
        };

        // Create FBO with color attachment and optional depth.
        #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
        let (fbo, depth_rb) = unsafe {
            let fbo = self
                .gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create framebuffer: {e}"))?;
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let depth_rb = if depth == Depth::Enabled {
                let rb = self
                    .gl
                    .create_renderbuffer()
                    .map_err(|e| anyhow::anyhow!("Failed to create depth renderbuffer: {e}"))?;
                self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
                self.gl.renderbuffer_storage(
                    glow::RENDERBUFFER,
                    glow::DEPTH_COMPONENT16,
                    width as i32,
                    height as i32,
                );
                self.gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(rb),
                );
                Some(rb)
            } else {
                None
            };

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Export framebuffer incomplete: 0x{status:x}");
            }
            (fbo, depth_rb)
        };

        let cached_stride = bo.stride();

        tracing::debug!("Export buffer allocated: {width}x{height}, stride={cached_stride}");

        Ok(ExportBuffer {
            bo,
            egl_image,
            texture,
            fbo,
            depth_rb,
            width,
            height,
            cached_fd: None,
            cached_stride,
            format,
            modifier,
        })
    }

    /// Allocate a per-widget staging render target.
    ///
    /// Returns a [`WidgetExportBuffer`] usable as a femtovg target: GL color
    /// texture + stencil RBO (+ optional depth RBO) attached to a complete
    /// FBO. Caller renders into the FBO, then blits to a separate
    /// [`ExportBuffer`] for DMA-BUF export.
    ///
    /// Must be released via [`Self::destroy_widget_export_buffer`] while this
    /// context is alive and current — GL deletion needs a current context.
    pub fn allocate_widget_export_buffer(
        &self,
        width: u32,
        height: u32,
        depth: Depth,
    ) -> Result<WidgetExportBuffer> {
        anyhow::ensure!(
            width > 0 && height > 0,
            "widget export buffer dimensions must be non-zero"
        );

        tracing::debug!("Allocating {width}x{height} widget export buffer (depth={depth:?})");

        let texture = self.make_staging_texture(width, height)?;
        let stencil_rbo = match self.make_renderbuffer(glow::STENCIL_INDEX8, width, height) {
            Ok(rbo) => rbo,
            Err(e) => {
                unsafe { self.gl.delete_texture(texture) };
                return Err(e.context("Failed to create stencil RBO"));
            }
        };
        let depth_rbo = if depth == Depth::Enabled {
            match self.make_renderbuffer(glow::DEPTH_COMPONENT16, width, height) {
                Ok(rbo) => Some(rbo),
                Err(e) => {
                    unsafe {
                        self.gl.delete_renderbuffer(stencil_rbo);
                        self.gl.delete_texture(texture);
                    }
                    return Err(e.context("Failed to create depth RBO"));
                }
            }
        } else {
            None
        };
        let fbo = match self.make_widget_export_fbo(texture, stencil_rbo, depth_rbo) {
            Ok(fbo) => fbo,
            Err(e) => {
                unsafe {
                    if let Some(rbo) = depth_rbo {
                        self.gl.delete_renderbuffer(rbo);
                    }
                    self.gl.delete_renderbuffer(stencil_rbo);
                    self.gl.delete_texture(texture);
                }
                return Err(e);
            }
        };

        Ok(WidgetExportBuffer {
            texture,
            fbo,
            stencil_rbo,
            depth_rbo,
            width,
            height,
        })
    }

    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    fn make_staging_texture(&self, width: u32, height: u32) -> Result<glow::Texture> {
        unsafe {
            let tex = self
                .gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("Failed to create staging texture: {e}"))?;
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                self.gl
                    .tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            Ok(tex)
        }
    }

    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    fn make_renderbuffer(
        &self,
        format: u32,
        width: u32,
        height: u32,
    ) -> Result<glow::Renderbuffer> {
        unsafe {
            let rbo = self
                .gl
                .create_renderbuffer()
                .map_err(|e| anyhow::anyhow!("create_renderbuffer failed: {e}"))?;
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
            self.gl
                .renderbuffer_storage(glow::RENDERBUFFER, format, width as i32, height as i32);
            Ok(rbo)
        }
    }

    /// Build the per-widget staging FBO, attach color + stencil (+ optional
    /// depth), and validate completeness. On failure (incomplete FBO) the
    /// freshly allocated framebuffer is deleted before returning; the
    /// caller still owns the texture and renderbuffers.
    fn make_widget_export_fbo(
        &self,
        texture: glow::Texture,
        stencil_rbo: glow::Renderbuffer,
        depth_rbo: Option<glow::Renderbuffer>,
    ) -> Result<glow::Framebuffer> {
        unsafe {
            let fbo = self
                .gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create staging FBO: {e}"))?;
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            self.gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(stencil_rbo),
            );
            if let Some(rbo) = depth_rbo {
                self.gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(rbo),
                );
            }

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                self.gl.delete_framebuffer(fbo);
                anyhow::bail!("Widget export framebuffer incomplete: 0x{status:x}");
            }
            Ok(fbo)
        }
    }

    /// Destroy a [`WidgetExportBuffer`], freeing all GL resources.
    ///
    /// Required: callers must invoke this while `self` is the current EGL
    /// context on this thread. Dropping a [`WidgetExportBuffer`] without
    /// going through this method leaks GL handles.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "takes ownership to consume the buffer"
    )]
    pub fn destroy_widget_export_buffer(&self, buf: WidgetExportBuffer) {
        unsafe {
            self.gl.delete_framebuffer(buf.fbo);
            self.gl.delete_renderbuffer(buf.stencil_rbo);
            if let Some(rbo) = buf.depth_rbo {
                self.gl.delete_renderbuffer(rbo);
            }
            self.gl.delete_texture(buf.texture);
        }
        tracing::debug!(
            "Destroyed widget export buffer ({}x{})",
            buf.width,
            buf.height
        );
    }

    /// Destroy an export buffer, freeing all GPU resources.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "takes ownership to consume the buffer"
    )]
    pub fn destroy_export_buffer(&self, buf: ExportBuffer) {
        unsafe {
            self.gl.delete_framebuffer(buf.fbo);
            if let Some(depth_rb) = buf.depth_rb {
                self.gl.delete_renderbuffer(depth_rb);
            }
            self.gl.delete_texture(buf.texture);
            (self.egl_destroy_image)(self.egl_display_raw, buf.egl_image);
        }
        tracing::debug!("Destroyed export buffer (EGLImage {:?})", buf.egl_image);
    }

    /// Export a rendered buffer as a DMA-BUF.
    ///
    /// On first call per buffer, obtains the DMA-BUF fd from the kernel and
    /// caches it. Subsequent calls duplicate the cached fd (cheap `dup` syscall
    /// instead of `drmPrimeHandleToFD`).
    pub fn export_dmabuf(buf: &mut ExportBuffer) -> Result<DmaBufInfo> {
        use std::os::fd::AsFd;

        // Cache fd on first use
        if buf.cached_fd.is_none() {
            let fd = buf
                .bo
                .fd()
                .context("Failed to get DMA-BUF fd from GBM BO")?;
            buf.cached_fd = Some(fd);
        }

        // Duplicate the cached fd
        let fd = buf
            .cached_fd
            .as_ref()
            .expect("BUG: cached_fd should exist after caching above")
            .as_fd()
            .try_clone_to_owned()
            .context("Failed to dup DMA-BUF fd")?;

        Ok(DmaBufInfo {
            fd,
            width: buf.width,
            height: buf.height,
            stride: buf.cached_stride,
            format: buf.format.drm_fourcc(),
            modifier: buf.modifier,
        })
    }

    /// Re-export of `smithay::backend::egl::get_proc_address` for GL loaders.
    ///
    /// Widgets that need to pass a GL function loader to external libraries
    /// (e.g. `bmc-wasm-runtime`) can use this without depending on smithay
    /// directly.
    #[must_use]
    pub fn get_proc_address(symbol: &str) -> *const c_void {
        // SAFETY: querying an EGL function pointer is safe; the returned pointer
        // is only dangerous if called with the wrong signature, which is the
        // caller's responsibility.
        unsafe { smithay::backend::egl::get_proc_address(symbol) }
    }

    /// Probe whether the EGL context has been lost.
    ///
    /// Reads the thread-local EGL error state. The host's main thread is the
    /// only thread that calls EGL, so the global last-error read is safe.
    #[must_use]
    pub fn is_context_lost(&self) -> bool {
        // SAFETY: eglGetError reads thread-local EGL state; no mutable aliasing.
        unsafe { (self.egl_get_error)() == EGL_CONTEXT_LOST }
    }
}

/// A DMA-BUF-exportable render buffer: GBM BO + EGLImage + GL texture + FBO.
///
/// Created via [`EglContext::allocate_export_buffer`]. Bind `fbo` as the GL
/// render target, render into it, then call [`EglContext::export_dmabuf`]
/// to get a DMA-BUF fd for Wayland buffer creation.
pub struct ExportBuffer {
    /// GBM buffer object.
    bo: BufferObject<()>,
    /// EGLImage handle (freed via `eglDestroyImageKHR` in
    /// [`EglContext::destroy_export_buffer`]).
    egl_image: *mut c_void,
    /// GL texture (color attachment, backed by the EGLImage).
    texture: glow::Texture,
    /// GL framebuffer object — bind this as the render target.
    pub fbo: glow::Framebuffer,
    /// GL depth renderbuffer (only allocated when [`Depth::Enabled`]).
    depth_rb: Option<glow::Renderbuffer>,
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
    /// Cached DMA-BUF fd (avoids kernel call per frame).
    cached_fd: Option<OwnedFd>,
    /// Cached stride in bytes.
    cached_stride: u32,
    /// Pixel format used when this buffer is advertised as DMA-BUF.
    format: ExportFormat,
    /// Layout the BO was actually allocated with (tiled or linear fallback).
    modifier: DrmModifier,
}

impl fmt::Debug for ExportBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExportBuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ExportBuffer {
    /// Get the raw GL framebuffer name (for `set_screen_target` etc.).
    #[must_use]
    pub fn fbo_id(&self) -> u32 {
        self.fbo.0.get()
    }
}

/// Per-widget staging render target: a GL color texture, stencil RBO, and FBO.
///
/// femtovg renders Y-flipped when targeting an FBO and needs an 8-bit stencil
/// for its painting algorithms. Each widget owns one of these against a shared
/// [`EglContext`]; the widget renders into `fbo` with femtovg, then blits with
/// Y-flip into a separate DMA-BUF-backed [`ExportBuffer`] for compositor
/// submission. Depth attachment is optional (matches [`ExportBuffer`] policy);
/// most femtovg widgets do not need it.
///
/// Must be released via [`EglContext::destroy_widget_export_buffer`] before
/// dropping — GL deletion requires the EGL context current on this thread,
/// which only the owning [`EglContext`] can guarantee.
pub struct WidgetExportBuffer {
    /// GL color texture (regular `glTexImage2D` storage, not EGLImage).
    texture: glow::Texture,
    fbo: glow::Framebuffer,
    /// Stencil renderbuffer (`STENCIL_INDEX8`). Always allocated — femtovg
    /// requires stencil for its painting algorithms.
    stencil_rbo: glow::Renderbuffer,
    /// Optional depth renderbuffer (`DEPTH_COMPONENT16`). Allocated only when
    /// constructed with [`Depth::Enabled`].
    depth_rbo: Option<glow::Renderbuffer>,
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
}

impl fmt::Debug for WidgetExportBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WidgetExportBuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("depth", &self.depth_rbo.is_some())
            .finish_non_exhaustive()
    }
}

impl WidgetExportBuffer {
    /// GL framebuffer handle (for `bind_framebuffer` etc.).
    #[must_use]
    pub fn fbo(&self) -> glow::Framebuffer {
        self.fbo
    }

    /// Raw GL framebuffer name (for femtovg `set_screen_target` etc.).
    #[must_use]
    pub fn fbo_id(&self) -> u32 {
        self.fbo.0.get()
    }

    /// GL color texture backing the staging FBO.
    #[must_use]
    pub fn texture(&self) -> glow::Texture {
        self.texture
    }
}

/// Compositor-release bookkeeping for the two export slots.
///
/// A slot submitted to the compositor is pinned until the compositor sends
/// `wl_buffer.release`; only an unpinned slot may be destroyed. Tracks one
/// availability bit per slot, paired with the slot's allocation state held by
/// [`DoubleBufferState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotReleaseState {
    available: [bool; 2],
}

impl SlotReleaseState {
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

    pub fn destroyable_slots(
        &self,
        allocated_slots: [bool; 2],
    ) -> impl Iterator<Item = usize> + '_ {
        (0..allocated_slots.len())
            .filter(move |&slot| allocated_slots[slot] && self.is_available(slot))
    }
}

impl Default for SlotReleaseState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct TwoSlotBufferCache<B> {
    buffers: [Option<B>; 2],
    release: SlotReleaseState,
}

impl<B> TwoSlotBufferCache<B> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffers: [None, None],
            release: SlotReleaseState::new(),
        }
    }

    #[must_use]
    pub fn cached_count(&self) -> usize {
        self.buffers
            .iter()
            .filter(|buffer| buffer.is_some())
            .count()
    }

    #[must_use]
    pub fn is_available(&self, slot: usize) -> bool {
        self.release.is_available(slot)
    }

    pub fn mark_presented(&mut self, slot: usize) {
        self.release.mark_presented(slot);
    }

    pub fn mark_released(&mut self, slot: usize) {
        self.release.mark_released(slot);
    }

    #[must_use]
    pub const fn release_state(&self) -> SlotReleaseState {
        self.release
    }

    pub fn reset_release_state(&mut self) {
        self.release = SlotReleaseState::new();
    }

    pub fn get_or_try_insert_with<E>(
        &mut self,
        slot: usize,
        make_buffer: impl FnOnce() -> Result<B, E>,
    ) -> Result<Option<&B>, E> {
        let Some(buffer) = self.buffers.get_mut(slot) else {
            return Ok(None);
        };
        if buffer.is_none() {
            *buffer = Some(make_buffer()?);
        }
        Ok(buffer.as_ref())
    }

    pub fn mark_released_matching(&mut self, mut matches: impl FnMut(&B) -> bool) -> bool {
        let Some(slot) = self
            .buffers
            .iter()
            .position(|buffer| buffer.as_ref().is_some_and(&mut matches))
        else {
            return false;
        };
        self.release.mark_released(slot);
        true
    }

    pub fn take_slot(&mut self, slot: usize) -> Option<B> {
        self.buffers.get_mut(slot)?.take()
    }

    pub fn take_all(&mut self) -> [Option<B>; 2] {
        std::mem::take(&mut self.buffers)
    }
}

impl<B> Default for TwoSlotBufferCache<B> {
    fn default() -> Self {
        Self::new()
    }
}

/// Double-buffered DMA-BUF export state.
///
/// Manages two lazily-allocated [`ExportBuffer`]s with ping-pong swap.
/// Widgets compose this with [`EglContext`] and their own rendering pipeline
/// (direct FBO, staging+blit, etc.).
pub struct DoubleBufferState {
    buffers: [Option<ExportBuffer>; 2],
    current_buffer: usize,
    width: u32,
    height: u32,
    depth: Depth,
    format: ExportFormat,
}

impl fmt::Debug for DoubleBufferState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DoubleBufferState")
            .field("current_buffer", &self.current_buffer)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("depth", &self.depth)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl DoubleBufferState {
    /// Create empty state at the given dimensions. Buffers are allocated lazily
    /// on the first call to [`Self::ensure_current`].
    #[must_use]
    pub fn new(width: u32, height: u32, depth: Depth) -> Self {
        Self::new_with_format(width, height, depth, ExportFormat::Opaque)
    }

    /// Create empty state at the given dimensions with an explicit export
    /// format. Buffers are allocated lazily on first use.
    #[must_use]
    pub fn new_with_format(width: u32, height: u32, depth: Depth, format: ExportFormat) -> Self {
        Self {
            buffers: [None, None],
            current_buffer: 0,
            width,
            height,
            depth,
            format,
        }
    }

    /// Buffer width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Buffer height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel format used by newly allocated export buffers.
    #[must_use]
    pub const fn export_format(&self) -> ExportFormat {
        self.format
    }

    /// Current back-buffer slot index.
    #[must_use]
    pub fn current_slot(&self) -> usize {
        self.current_buffer
    }

    /// Ensure the current back buffer is allocated, return a reference to it.
    pub fn ensure_current(&mut self, ctx: &EglContext) -> Result<&ExportBuffer> {
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(ctx.allocate_export_buffer_with_format(
                self.width,
                self.height,
                self.depth,
                self.format,
            )?);
        }
        Ok(self.buffers[idx]
            .as_ref()
            .expect("BUG: buffer should exist after allocation"))
    }

    /// Get a reference to the current back buffer (`None` if not yet allocated).
    #[must_use]
    pub fn current_ref(&self) -> Option<&ExportBuffer> {
        self.buffers[self.current_buffer].as_ref()
    }

    #[must_use]
    pub fn allocated_slots(&self) -> [bool; 2] {
        [self.buffers[0].is_some(), self.buffers[1].is_some()]
    }

    #[must_use]
    fn next_slot(current_buffer: usize) -> usize {
        debug_assert!(
            current_buffer < 2,
            "BUG: double-buffer slot index out of range"
        );
        1 - current_buffer
    }

    /// Export the current buffer as DMA-BUF and swap to the next buffer.
    ///
    /// Returns the DMA-BUF info and the slot index of the exported buffer
    /// (for use with [`crate::surface::WidgetSurface::submit_buffer`]).
    pub fn export_and_swap(&mut self) -> Result<(DmaBufInfo, usize)> {
        let slot = self.current_buffer;
        let buf = self.buffers[slot]
            .as_mut()
            .expect("BUG: buffer should exist after ensure_current");
        let info = EglContext::export_dmabuf(buf)?;
        self.current_buffer = Self::next_slot(slot);
        Ok((info, slot))
    }

    /// Resize -- deallocate all buffers so they are reallocated at the new size.
    ///
    /// No-op if dimensions are unchanged.
    pub fn resize(&mut self, ctx: &EglContext, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        tracing::debug!(
            "DoubleBufferState: resizing from {}x{} to {}x{}",
            self.width,
            self.height,
            width,
            height,
        );
        self.destroy_all(ctx);
        self.width = width;
        self.height = height;
    }

    /// Destroy all allocated buffers, freeing GPU resources.
    ///
    /// Low-level helper for owners that manage the associated [`EglContext`]
    /// themselves. Prefer [`DoubleBufferedEglState`] when possible so this
    /// cleanup happens automatically in `Drop`.
    pub fn destroy_all(&mut self, ctx: &EglContext) {
        for buffer in &mut self.buffers {
            if let Some(buf) = buffer.take() {
                ctx.destroy_export_buffer(buf);
            }
        }
    }

    pub fn destroy_slot(&mut self, ctx: &EglContext, slot: usize) -> bool {
        let Some(buffer) = self.buffers.get_mut(slot) else {
            return false;
        };
        if let Some(buf) = buffer.take() {
            ctx.destroy_export_buffer(buf);
            true
        } else {
            false
        }
    }
}

/// Owning EGL + double-buffer helper with automatic buffer cleanup.
///
/// This pairs [`EglContext`] with the internal double-buffer state so their
/// destruction order is always correct: allocated export buffers are destroyed
/// first, while the EGL/GL context is still alive, and only then is the
/// context dropped. Widgets with direct-FBO double-buffer pipelines can use
/// this instead of managing manual export-buffer cleanup in `Drop`.
pub struct DoubleBufferedEglState {
    ctx: EglContext,
    buffers: DoubleBufferState,
}

impl fmt::Debug for DoubleBufferedEglState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DoubleBufferedEglState")
            .field("ctx", &self.ctx)
            .field("buffers", &self.buffers)
            .finish()
    }
}

impl DoubleBufferedEglState {
    /// Create EGL context and empty double-buffer state at the given size.
    pub fn new(width: u32, height: u32, depth: Depth) -> Result<Self> {
        Ok(Self {
            ctx: EglContext::new()?,
            buffers: DoubleBufferState::new(width, height, depth),
        })
    }

    /// Get the glow OpenGL ES context.
    pub fn gl(&self) -> &glow::Context {
        self.ctx.gl()
    }

    /// Borrow the underlying [`EglContext`].
    ///
    /// Useful for callers that need to allocate sibling resources
    /// (e.g. [`WidgetExportBuffer`]) against the same context.
    #[must_use]
    pub fn ctx(&self) -> &EglContext {
        &self.ctx
    }

    /// Ensure the current back buffer is allocated, return a reference to it.
    pub fn ensure_current(&mut self) -> Result<&ExportBuffer> {
        self.buffers.ensure_current(&self.ctx)
    }

    /// Get a reference to the current back buffer (`None` if not yet allocated).
    #[must_use]
    pub fn current_ref(&self) -> Option<&ExportBuffer> {
        self.buffers.current_ref()
    }

    #[must_use]
    pub fn current_slot(&self) -> usize {
        self.buffers.current_slot()
    }

    #[must_use]
    pub fn allocated_slots(&self) -> [bool; 2] {
        self.buffers.allocated_slots()
    }

    /// Buffer width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.buffers.width()
    }

    /// Buffer height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.buffers.height()
    }

    /// Export the current buffer as DMA-BUF and swap to the next buffer.
    ///
    /// Returns the DMA-BUF info and the slot index of the exported buffer.
    pub fn export_and_swap(&mut self) -> Result<(DmaBufInfo, usize)> {
        self.buffers.export_and_swap()
    }

    /// Resize and drop any existing export buffers.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.buffers.resize(&self.ctx, width, height);
    }

    pub fn destroy_slot(&mut self, slot: usize) -> bool {
        self.buffers.destroy_slot(&self.ctx, slot)
    }

    /// Destroy all allocated export buffers, freeing the underlying CMA
    /// memory immediately. Buffers will be lazily reallocated on the
    /// next call to [`Self::ensure_current`] (and therefore on the next
    /// frame).
    ///
    /// Used by lifecycle-aware widgets to release render-target memory
    /// while the surface is in the `dormant` lifecycle state.
    pub fn destroy_buffers(&mut self) {
        self.buffers.destroy_all(&self.ctx);
    }
}

impl Drop for DoubleBufferedEglState {
    fn drop(&mut self) {
        self.buffers.destroy_all(&self.ctx);
    }
}

/// Resources for the Y-flip blit pass used by femtovg-rendering widgets.
///
/// femtovg renders Y-flipped when targeting an FBO; this program samples a
/// staging texture with flipped V and writes to a caller-supplied destination
/// FBO. Lives in [`SharedRenderScratch`] alongside the staging texture so the
/// host can share one program + VBO across all slots.
struct BlitResources {
    program: glow::Program,
    vbo: glow::Buffer,
    pos_loc: u32,
    uv_loc: u32,
    uv_scale_loc: glow::UniformLocation,
}

impl BlitResources {
    fn new(gl: &glow::Context) -> Result<Self> {
        let vert_src = r"#version 100
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
uniform vec2 u_uv_scale;
void main() {
    v_uv = a_uv * u_uv_scale;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
";
        let frag_src = r"#version 100
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}
";
        let program = unsafe {
            let vs = gl
                .create_shader(glow::VERTEX_SHADER)
                .map_err(|e| anyhow::anyhow!("Blit VS create: {e}"))?;
            gl.shader_source(vs, vert_src);
            gl.compile_shader(vs);
            if !gl.get_shader_compile_status(vs) {
                let log = gl.get_shader_info_log(vs);
                gl.delete_shader(vs);
                anyhow::bail!("Blit VS compile: {log}");
            }

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| {
                gl.delete_shader(vs);
                anyhow::anyhow!("Blit FS create: {e}")
            })?;
            gl.shader_source(fs, frag_src);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                let log = gl.get_shader_info_log(fs);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                anyhow::bail!("Blit FS compile: {log}");
            }

            let prog = gl.create_program().map_err(|e| {
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                anyhow::anyhow!("Blit program create: {e}")
            })?;
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            gl.link_program(prog);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(prog) {
                let log = gl.get_program_info_log(prog);
                gl.delete_program(prog);
                anyhow::bail!("Blit program link: {log}");
            }
            prog
        };

        #[rustfmt::skip]
        let vertices: [f32; 24] = [
            -1.0, -1.0,  0.0, 1.0,
             1.0, -1.0,  1.0, 1.0,
             1.0,  1.0,  1.0, 0.0,
            -1.0, -1.0,  0.0, 1.0,
             1.0,  1.0,  1.0, 0.0,
            -1.0,  1.0,  0.0, 0.0,
        ];

        let vbo = unsafe {
            let buf = gl.create_buffer().map_err(|e| {
                gl.delete_program(program);
                anyhow::anyhow!("Blit VBO create: {e}")
            })?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            let bytes: &[u8] = std::slice::from_raw_parts(
                vertices.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&vertices),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            buf
        };

        let pos_loc = unsafe {
            gl.get_attrib_location(program, "a_pos")
                .expect("BUG: a_pos not found in blit shader")
        };
        let uv_loc = unsafe {
            gl.get_attrib_location(program, "a_uv")
                .expect("BUG: a_uv not found in blit shader")
        };
        let uv_scale_loc = unsafe {
            gl.get_uniform_location(program, "u_uv_scale")
                .expect("BUG: u_uv_scale not found in blit shader")
        };

        Ok(Self {
            program,
            vbo,
            pos_loc,
            uv_loc,
            uv_scale_loc,
        })
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_buffer(self.vbo);
        }
    }
}

/// Resources for the offset panel-cache copy used by sliding system overlays.
///
/// Unlike [`BlitResources`] (which Y-flips a femtovg staging texture and fills
/// the whole target), this copies an already-upright source texture into a
/// destination FBO at a caller-supplied screen rectangle — a quad whose NDC
/// bounds are passed as a uniform so the panel can be translated vertically per
/// animation frame. No V-flip: the source was produced by a prior
/// [`BlitResources`] copy and is already in destination orientation.
struct OffsetBlitResources {
    program: glow::Program,
    vbo: glow::Buffer,
    pos_loc: u32,
    uv_loc: u32,
    rect_loc: glow::UniformLocation,
}

impl OffsetBlitResources {
    fn new(gl: &glow::Context) -> Result<Self> {
        // a_pos carries the unit quad in [0,1]; u_rect = (x0, y0, x1, y1) in NDC
        // remaps it to the destination rectangle, so the same VBO draws the
        // panel at any vertical offset.
        let vert_src = r"#version 100
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
uniform vec4 u_rect;
void main() {
    vec2 ndc = mix(u_rect.xy, u_rect.zw, a_pos);
    v_uv = a_uv;
    gl_Position = vec4(ndc, 0.0, 1.0);
}
";
        let frag_src = r"#version 100
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}
";
        let program = unsafe {
            let vs = gl
                .create_shader(glow::VERTEX_SHADER)
                .map_err(|e| anyhow::anyhow!("OffsetBlit VS create: {e}"))?;
            gl.shader_source(vs, vert_src);
            gl.compile_shader(vs);
            if !gl.get_shader_compile_status(vs) {
                let log = gl.get_shader_info_log(vs);
                gl.delete_shader(vs);
                anyhow::bail!("OffsetBlit VS compile: {log}");
            }
            let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| {
                gl.delete_shader(vs);
                anyhow::anyhow!("OffsetBlit FS create: {e}")
            })?;
            gl.shader_source(fs, frag_src);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                let log = gl.get_shader_info_log(fs);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                anyhow::bail!("OffsetBlit FS compile: {log}");
            }
            let prog = gl.create_program().map_err(|e| {
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                anyhow::anyhow!("OffsetBlit program create: {e}")
            })?;
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            gl.link_program(prog);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(prog) {
                let log = gl.get_program_info_log(prog);
                gl.delete_program(prog);
                anyhow::bail!("OffsetBlit program link: {log}");
            }
            prog
        };

        // Unit quad with a_uv == a_pos: an identity copy. The cached source was
        // produced by the same `blit_to` the normal overlay path uses to fill
        // the export buffer, so it already holds destination-oriented pixels —
        // sampling it straight keeps the panel upright.
        #[rustfmt::skip]
        let vertices: [f32; 24] = [
            0.0, 0.0,  0.0, 0.0,
            1.0, 0.0,  1.0, 0.0,
            1.0, 1.0,  1.0, 1.0,
            0.0, 0.0,  0.0, 0.0,
            1.0, 1.0,  1.0, 1.0,
            0.0, 1.0,  0.0, 1.0,
        ];

        let vbo = unsafe {
            let buf = gl.create_buffer().map_err(|e| {
                gl.delete_program(program);
                anyhow::anyhow!("OffsetBlit VBO create: {e}")
            })?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            let bytes: &[u8] = std::slice::from_raw_parts(
                vertices.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&vertices),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            buf
        };

        let pos_loc = unsafe {
            gl.get_attrib_location(program, "a_pos")
                .expect("BUG: a_pos not found in offset-blit shader")
        };
        let uv_loc = unsafe {
            gl.get_attrib_location(program, "a_uv")
                .expect("BUG: a_uv not found in offset-blit shader")
        };
        let rect_loc = unsafe {
            gl.get_uniform_location(program, "u_rect")
                .expect("BUG: u_rect not found in offset-blit shader")
        };

        Ok(Self {
            program,
            vbo,
            pos_loc,
            uv_loc,
            rect_loc,
        })
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_buffer(self.vbo);
        }
    }
}

#[must_use]
fn shared_scratch_uv_scale(max_width: u32, max_height: u32, w: u32, h: u32) -> (f32, f32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel dimensions converted to normalized GL texture coordinates"
    )]
    (w as f32 / max_width as f32, h as f32 / max_height as f32)
}

/// NDC rectangle `(x0, y0, x1, y1)` for a panel of `panel_h` pixels anchored at
/// the top of a `dest_h`-tall target, translated down by `offset_y` (negative =
/// off the top). The panel spans the full width.
///
/// This overlay's export buffer is scanned out Y-flipped, so pixel-y (0 = the
/// display top, growing downward) maps to NDC with +Y pointing *down*:
/// `ndc = 2 * y / h - 1`. The panel's top edge therefore takes the smaller NDC
/// y, pairing with the cache's top row (`a_pos.y = 0`, `a_uv.v = 0`).
#[must_use]
fn offset_panel_ndc_rect(dest_h: u32, panel_h: f32, offset_y: f32) -> [f32; 4] {
    #[expect(
        clippy::cast_precision_loss,
        reason = "target height in pixels converts to NDC without meaningful loss"
    )]
    let h = dest_h as f32;
    // Top edge at screen-y = offset_y, bottom edge at offset_y + panel_h.
    let top_px = offset_y;
    let bottom_px = offset_y + panel_h;
    let ndc_panel_top = 2.0 * top_px / h - 1.0;
    let ndc_panel_bottom = 2.0 * bottom_px / h - 1.0;
    [-1.0, ndc_panel_top, 1.0, ndc_panel_bottom]
}

/// Per-render-thread scratch resources for femtovg-rendering widgets.
///
/// femtovg renders into an FBO Y-flipped and needs an 8-bit stencil for its
/// painting algorithms; this scratch holds:
///
/// - a staging [`WidgetExportBuffer`] (color texture + stencil RBO + FBO),
///   sized to a caller-chosen maximum that bounds all consumers, and
/// - a blit program that copies the staging color texture to a destination
///   FBO with flipped V.
///
/// Construct once per render thread against a shared [`EglContext`]; in the
/// standalone-widget process there is one of each. In the multi-widget host
/// (BDK-469 Stage 5) the host owns one `SharedRenderScratch` and one
/// `EglContext`; each slot owns its own [`DoubleBufferState`] and reuses the
/// shared scratch every frame. Smaller widgets set viewport to their own
/// dimensions on [`Self::begin_frame`] and the blit reads `(0,0,w,h)` of the
/// staging texture; sizing the staging to the display maximum is the caller's
/// responsibility.
///
/// Single-threaded: callers must serialize their `begin_frame` →
/// `blit_to` cycles. Two slots cannot render concurrently against the same
/// scratch.
///
/// Release via [`Self::destroy`] while the [`EglContext`] is current on this
/// thread — GL deletion requires a current context.
pub struct SharedRenderScratch {
    staging: WidgetExportBuffer,
    blit: BlitResources,
    offset_blit: OffsetBlitResources,
    staging_fbo_id_at_construction: u32,
}

impl fmt::Debug for SharedRenderScratch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedRenderScratch")
            .field("staging", &self.staging)
            .finish_non_exhaustive()
    }
}

impl SharedRenderScratch {
    /// Allocate a staging FBO sized to `(max_width, max_height)` and the
    /// Y-flip blit program against `ctx`. Per-frame consumers may render at
    /// any size up to this maximum; [`Self::begin_frame`] sets viewport to
    /// the per-frame size.
    pub fn new(ctx: &EglContext, max_width: u32, max_height: u32) -> Result<Self> {
        let staging = ctx
            .allocate_widget_export_buffer(max_width, max_height, Depth::Disabled)
            .context("Failed to allocate SharedRenderScratch staging")?;
        let blit = match BlitResources::new(ctx.gl()) {
            Ok(blit) => blit,
            Err(e) => {
                ctx.destroy_widget_export_buffer(staging);
                return Err(e);
            }
        };
        match OffsetBlitResources::new(ctx.gl()) {
            Ok(offset_blit) => {
                let staging_fbo_id_at_construction = staging.fbo_id();
                Ok(Self {
                    staging,
                    blit,
                    offset_blit,
                    staging_fbo_id_at_construction,
                })
            }
            Err(e) => {
                blit.destroy(ctx.gl());
                ctx.destroy_widget_export_buffer(staging);
                Err(e)
            }
        }
    }

    /// The raw GL framebuffer id of the shared staging FBO.
    ///
    /// The id is stable for the host's lifetime: `SharedRenderScratch::new` allocates
    /// the FBO once and `begin_frame` only re-binds the existing handle (sets the
    /// viewport / clears attachments). The host's `FemtoVgRenderer` is constructed
    /// against this id at startup, so any future change that rebinds the staging FBO
    /// to a fresh id would silently corrupt every slot's render. The
    /// `debug_assert_eq!` in `begin_frame` catches regressions in debug builds.
    #[must_use]
    pub fn staging_fbo_id(&self) -> u32 {
        self.staging.fbo_id()
    }

    #[must_use]
    pub fn max_size(&self) -> (u32, u32) {
        (self.staging.width, self.staging.height)
    }

    #[must_use]
    pub fn supports_size(&self, w: u32, h: u32) -> bool {
        w <= self.staging.width && h <= self.staging.height
    }

    /// Bind the staging FBO, set viewport to `(w, h)`, and clear color +
    /// stencil. Returns the raw GL framebuffer name for femtovg's
    /// `set_screen_target`.
    ///
    /// `w` and `h` must be ≤ the maximum dimensions passed to [`Self::new`];
    /// callers exceeding the maximum render outside the staging and produce
    /// black borders or clipped output.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    #[must_use]
    pub fn begin_frame(&self, ctx: &EglContext, w: u32, h: u32) -> u32 {
        debug_assert_eq!(
            self.staging.fbo_id(),
            self.staging_fbo_id_at_construction,
            "BUG: SharedRenderScratch::begin_frame rebound the staging FBO to a fresh id; \
             the host's FemtoVgRenderer was constructed against the original id and will \
             silently write to the wrong target. If a future change legitimately needs to \
             re-create the FBO, update SharedHost::init to re-bake the renderer's target.",
        );
        let gl = ctx.gl();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.staging.fbo()));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
        }
        self.staging.fbo_id()
    }

    /// Blit the staging color texture into `dest_fbo` with Y-flip, viewport
    /// `(0, 0, w, h)`. Call after femtovg's `flush()` and before swapping the
    /// destination buffer out for export.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn blit_to(&self, ctx: &EglContext, dest_fbo: glow::Framebuffer, w: u32, h: u32) {
        let gl = ctx.gl();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dest_fbo));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);

            gl.use_program(Some(self.blit.program));
            let (u_scale, v_scale) =
                shared_scratch_uv_scale(self.staging.width, self.staging.height, w, h);
            gl.uniform_2_f32(Some(&self.blit.uv_scale_loc), u_scale, v_scale);

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.staging.texture()));

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.blit.vbo));
            gl.enable_vertex_attrib_array(self.blit.pos_loc);
            gl.vertex_attrib_pointer_f32(self.blit.pos_loc, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(self.blit.uv_loc);
            gl.vertex_attrib_pointer_f32(self.blit.uv_loc, 2, glow::FLOAT, false, 16, 8);

            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            gl.disable_vertex_attrib_array(self.blit.pos_loc);
            gl.disable_vertex_attrib_array(self.blit.uv_loc);
        }
    }

    /// Clear `dest_fbo` transparent and draw `src_tex` (an already-upright
    /// `panel_h`-tall panel) into it as a full-width band translated down by
    /// `offset_y` pixels. No femtovg, no layout: this is the per-frame
    /// blit-only slide present for a sliding system overlay. `src_tex` is
    /// sampled over its whole extent (UV `[0,1]`).
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn blit_texture_at_offset(
        &self,
        ctx: &EglContext,
        src_tex: glow::Texture,
        dest_fbo: glow::Framebuffer,
        dest_size: (u32, u32),
        panel_h: f32,
        offset_y: f32,
    ) {
        let (dest_w, dest_h) = dest_size;
        let gl = ctx.gl();
        let rect = offset_panel_ndc_rect(dest_h, panel_h, offset_y);
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dest_fbo));
            gl.viewport(0, 0, dest_w as i32, dest_h as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.use_program(Some(self.offset_blit.program));
            gl.uniform_4_f32(
                Some(&self.offset_blit.rect_loc),
                rect[0],
                rect[1],
                rect[2],
                rect[3],
            );

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(src_tex));

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.offset_blit.vbo));
            gl.enable_vertex_attrib_array(self.offset_blit.pos_loc);
            gl.vertex_attrib_pointer_f32(self.offset_blit.pos_loc, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(self.offset_blit.uv_loc);
            gl.vertex_attrib_pointer_f32(self.offset_blit.uv_loc, 2, glow::FLOAT, false, 16, 8);

            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            gl.disable_vertex_attrib_array(self.offset_blit.pos_loc);
            gl.disable_vertex_attrib_array(self.offset_blit.uv_loc);
        }
    }

    /// Release all GL resources. Callers must invoke this while `ctx` is
    /// current on this thread; dropping the value without calling `destroy`
    /// leaks the staging FBO, color texture, stencil RBO, blit programs, and
    /// blit VBOs.
    pub fn destroy(self, ctx: &EglContext) {
        self.blit.destroy(ctx.gl());
        self.offset_blit.destroy(ctx.gl());
        ctx.destroy_widget_export_buffer(self.staging);
    }
}

/// Load an EGL/GL extension function pointer by name.
fn load_egl_proc<T>(name: &str) -> Result<T> {
    // SAFETY: querying an EGL function pointer is safe; the returned pointer
    // is only dangerous if called with the wrong signature (handled below via
    // a size-checked transmute_copy with caller-specified T).
    let proc = unsafe { smithay::backend::egl::get_proc_address(name) };
    if proc.is_null() {
        anyhow::bail!("{name} not available");
    }
    // `transmute::<*const c_void, T>` would provide the size check directly,
    // but rustc rejects that for generic `T` (E0512). Keep `transmute_copy`
    // and restore the same robustness with a compile-time size assertion.
    const {
        assert!(std::mem::size_of::<T>() == std::mem::size_of::<*const c_void>());
    }
    // SAFETY: smithay returns a valid function pointer matching the EGL spec
    // for the given extension name; the caller ensures T matches the signature,
    // and the const assertion above enforces pointer-sized T.
    Ok(unsafe { std::mem::transmute_copy(&proc) })
}

#[cfg(test)]
mod tests {
    use super::{
        Depth, DoubleBufferState, EglContext, ExportFormat, SharedRenderScratch, SlotReleaseState,
        TwoSlotBufferCache, WidgetExportBuffer, offset_panel_ndc_rect, shared_scratch_uv_scale,
    };
    use drm_fourcc::DrmFourcc;

    #[test]
    fn release_state_blocks_presented_slot_until_release() {
        let mut state = SlotReleaseState::new();

        assert!(state.is_available(0));

        state.mark_presented(0);
        assert!(!state.is_available(0));
        assert_eq!(
            state.destroyable_slots([true, true]).collect::<Vec<_>>(),
            vec![1]
        );

        state.mark_released(0);
        assert!(state.is_available(0));
        assert_eq!(
            state.destroyable_slots([true, true]).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn release_state_ignores_stale_out_of_range_slot_ids() {
        let mut state = SlotReleaseState::new();

        state.mark_presented(2);
        state.mark_presented(1);

        assert!(state.is_available(0));
        assert!(!state.is_available(1));
    }

    #[test]
    fn two_slot_buffer_cache_reuses_buffers_and_marks_matching_release() {
        let mut cache = TwoSlotBufferCache::new();

        let first = cache
            .get_or_try_insert_with(0, || Ok::<_, std::convert::Infallible>(42_u32))
            .expect("BUG: infallible creation")
            .expect("BUG: slot 0 exists");
        assert_eq!(*first, 42);
        let second = cache
            .get_or_try_insert_with(0, || Ok::<_, std::convert::Infallible>(7_u32))
            .expect("BUG: infallible creation")
            .expect("BUG: slot 0 exists");
        assert_eq!(*second, 42);

        cache.mark_presented(0);
        assert!(!cache.is_available(0));
        assert!(cache.mark_released_matching(|value| *value == 42));

        assert!(cache.is_available(0));
        assert!(!cache.mark_released_matching(|value| *value == 99));
    }

    #[test]
    fn two_slot_buffer_cache_reports_invalid_slots_without_allocating() {
        let mut cache = TwoSlotBufferCache::new();

        let value = cache
            .get_or_try_insert_with(2, || Ok::<_, std::convert::Infallible>(42_u32))
            .expect("BUG: infallible creation");

        assert!(value.is_none());
        assert_eq!(cache.cached_count(), 0);
    }

    #[test]
    fn shared_scratch_uv_scale_samples_only_active_slot_region() {
        assert_eq!(shared_scratch_uv_scale(1280, 480, 640, 240), (0.5, 0.5));
        assert_eq!(shared_scratch_uv_scale(1280, 480, 1280, 480), (1.0, 1.0));
    }

    #[test]
    fn offset_panel_ndc_rect_settled_panel_fills_full_band() {
        // A full-height panel settled at offset 0 spans the whole NDC range.
        // In the Y-flipped buffer y0 is the panel's top edge (display top = -1)
        // and y1 its bottom edge (display bottom = +1).
        let [x0, y0, x1, y1] = offset_panel_ndc_rect(480, 480.0, 0.0);
        assert!((x0 + 1.0).abs() < 1e-6, "left edge at -1");
        assert!(
            (y0 + 1.0).abs() < 1e-6,
            "panel top edge at -1 (display top)"
        );
        assert!((x1 - 1.0).abs() < 1e-6, "right edge at 1");
        assert!(
            (y1 - 1.0).abs() < 1e-6,
            "panel bottom edge at 1 (display bottom)"
        );
    }

    #[test]
    fn offset_panel_ndc_rect_offscreen_pushes_band_above_top() {
        // Slid fully off the top of the display. The buffer is Y-flipped, so the
        // display's top edge is NDC y = -1; the whole band sits at or beyond it.
        let [_, y0, _, y1] = offset_panel_ndc_rect(480, 480.0, -480.0);
        assert!(
            y1 <= -1.0,
            "panel bottom edge at or beyond the display top (NDC -1)"
        );
        assert!(
            y0 < y1,
            "panel top edge pushed further off-screen than its bottom"
        );
    }

    #[test]
    #[ignore = "EglContext::new() opens /dev/dri/renderD128; the CI sandbox surfaceless EGL has no DRM render node"]
    fn two_widget_export_buffers_share_one_egl_context() {
        let ctx = EglContext::new().expect("BUG: EGL context creation should succeed in test env");

        let a: WidgetExportBuffer = ctx
            .allocate_widget_export_buffer(640, 480, Depth::Disabled)
            .expect("BUG: first WidgetExportBuffer should allocate");
        let b: WidgetExportBuffer = ctx
            .allocate_widget_export_buffer(320, 240, Depth::Disabled)
            .expect("BUG: second WidgetExportBuffer should allocate");

        assert_eq!(a.width, 640);
        assert_eq!(a.height, 480);
        assert_eq!(b.width, 320);
        assert_eq!(b.height, 240);
        assert_ne!(a.fbo_id(), b.fbo_id(), "FBOs must be distinct GL names");

        ctx.destroy_widget_export_buffer(a);
        ctx.destroy_widget_export_buffer(b);
    }

    #[test]
    fn double_buffer_state_starts_empty_on_slot_zero() {
        let state = DoubleBufferState::new(640, 480, Depth::Disabled);

        assert_eq!(state.width(), 640);
        assert_eq!(state.height(), 480);
        assert_eq!(state.current_slot(), 0);
        assert_eq!(state.current_buffer, 0);
        assert!(state.buffers.iter().all(Option::is_none));
        assert!(state.current_ref().is_none());
    }

    #[test]
    fn double_buffer_state_keeps_alpha_export_format_explicit() {
        let opaque = DoubleBufferState::new(640, 480, Depth::Disabled);
        let alpha =
            DoubleBufferState::new_with_format(640, 480, Depth::Disabled, ExportFormat::Alpha);

        assert_eq!(opaque.export_format().drm_fourcc(), DrmFourcc::Xrgb8888);
        assert_eq!(alpha.export_format().drm_fourcc(), DrmFourcc::Argb8888);
    }

    #[test]
    fn next_slot_ping_pongs_between_two_slots() {
        assert_eq!(DoubleBufferState::next_slot(0), 1);
        assert_eq!(DoubleBufferState::next_slot(1), 0);
    }

    #[test]
    #[ignore = "EglContext::new() opens /dev/dri/renderD128; the CI sandbox surfaceless EGL has no DRM render node"]
    fn shared_scratch_supports_two_independent_double_buffers() {
        use glow::HasContext;

        let ctx = EglContext::new().expect("BUG: EGL context creation should succeed in test env");
        let scratch = SharedRenderScratch::new(&ctx, 480, 480)
            .expect("BUG: SharedRenderScratch should allocate at display max");

        let mut slot_a = DoubleBufferState::new(320, 240, Depth::Disabled);
        let mut slot_b = DoubleBufferState::new(480, 480, Depth::Disabled);

        // Each slot allocates its own export buffer against the shared ctx;
        // their FBOs are distinct GL names.
        let fbo_a_id;
        let fbo_b_id;
        let fbo_a_handle;
        let fbo_b_handle;
        {
            let buf_a = slot_a
                .ensure_current(&ctx)
                .expect("BUG: slot A export buffer should allocate");
            fbo_a_id = buf_a.fbo_id();
            fbo_a_handle = buf_a.fbo;
        }
        {
            let buf_b = slot_b
                .ensure_current(&ctx)
                .expect("BUG: slot B export buffer should allocate");
            fbo_b_id = buf_b.fbo_id();
            fbo_b_handle = buf_b.fbo;
        }
        assert_ne!(fbo_a_id, fbo_b_id, "slot FBOs must be distinct GL names");

        // Render slot A: clear staging to red, blit to slot A's export FBO.
        // The clear values used here override the default clear set by
        // `begin_frame`; the order is intentional.
        let _ = scratch.begin_frame(&ctx, 320, 240);
        unsafe {
            ctx.gl().clear_color(1.0, 0.0, 0.0, 1.0);
            ctx.gl().clear(glow::COLOR_BUFFER_BIT);
        }
        scratch.blit_to(&ctx, fbo_a_handle, 320, 240);

        // Render slot B: clear staging to green, blit to slot B's export FBO.
        let _ = scratch.begin_frame(&ctx, 480, 480);
        unsafe {
            ctx.gl().clear_color(0.0, 1.0, 0.0, 1.0);
            ctx.gl().clear(glow::COLOR_BUFFER_BIT);
        }
        scratch.blit_to(&ctx, fbo_b_handle, 480, 480);

        unsafe {
            let err = ctx.gl().get_error();
            assert_eq!(
                err,
                glow::NO_ERROR,
                "GL error after cross-slot blit: 0x{err:x}"
            );
        }

        // Read one pixel back from each export FBO. The color attachment is
        // the EGLImage-backed texture (== the DMA-BUF the compositor will
        // sample), so `glReadPixels` returns the exact bytes the compositor
        // would see. Buffer format is XRGB8888 with stride-padded rows; we
        // only need one pixel from the centre so stride does not matter.
        #[expect(
            clippy::integer_division,
            reason = "center pixel via floor-division is intentional"
        )]
        let read_centre_rgba = |fbo: glow::Framebuffer, w: i32, h: i32| -> [u8; 4] {
            let mut px = [0_u8; 4];
            unsafe {
                ctx.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                ctx.gl().read_pixels(
                    w / 2,
                    h / 2,
                    1,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut px)),
                );
                let err = ctx.gl().get_error();
                assert_eq!(err, glow::NO_ERROR, "glReadPixels error: 0x{err:x}");
            }
            px
        };

        let px_a = read_centre_rgba(fbo_a_handle, 320, 240);
        let px_b = read_centre_rgba(fbo_b_handle, 480, 480);

        // Slot A should be (red) ≈ 255,0,0; slot B (green) ≈ 0,255,0. Allow
        // 1 LSB tolerance for sampler/format conversion. Crucially the two
        // pixels must differ, proving cross-slot independence.
        assert!(
            px_a[0] > 250 && px_a[1] < 5,
            "slot A centre pixel should be red, got {px_a:?}"
        );
        assert!(
            px_b[1] > 250 && px_b[0] < 5,
            "slot B centre pixel should be green, got {px_b:?}"
        );
        assert_ne!(px_a, px_b, "cross-slot blits must produce distinct pixels");

        // Each slot exports its own DMA-BUF independently.
        let (info_a, _) = slot_a.export_and_swap().expect("BUG: slot A export");
        let (info_b, _) = slot_b.export_and_swap().expect("BUG: slot B export");
        assert_eq!(info_a.width, 320);
        assert_eq!(info_b.width, 480);

        slot_a.destroy_all(&ctx);
        slot_b.destroy_all(&ctx);
        scratch.destroy(&ctx);
    }
}

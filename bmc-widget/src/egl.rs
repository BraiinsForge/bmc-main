// Copyright (C) 2026  Braiins Systems s.r.o.
//
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
        egl::{EGLContext, EGLDisplay},
    },
    reexports::gbm::{AsRaw, BufferObject, BufferObjectFlags, Device as GbmDevice},
};

/// Default GPU render node.
const GPU_PATH: &str = "/dev/dri/renderD128";

// EGL constants
const EGL_NATIVE_PIXMAP_KHR: u32 = 0x30B0;
const EGL_NONE: i32 = 0x3038;
const EGL_NO_IMAGE: *mut c_void = ptr::null_mut();

// GL_OES_EGL_image extension
const GL_TEXTURE_2D: u32 = 0x0DE1;

// EGL/GL extension function pointer types
type EglCreateImageKhr = unsafe extern "C" fn(
    dpy: *mut c_void,
    ctx: *mut c_void,
    target: u32,
    buffer: *mut c_void,
    attrib_list: *const i32,
) -> *mut c_void;

type EglDestroyImageKhr = unsafe extern "C" fn(dpy: *mut c_void, image: *mut c_void) -> i32;

type GlEglImageTargetTexture2DOes = unsafe extern "C" fn(target: u32, image: *mut c_void);

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
    #[expect(dead_code, reason = "kept alive for context lifetime")]
    egl_display: EGLDisplay,
    /// EGL context (kept alive for GL operations).
    #[expect(dead_code, reason = "kept alive for GL operations")]
    context: EGLContext,
    /// OpenGL ES context via glow.
    gl: glow::Context,
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

    /// Allocate an export buffer (GBM BO + EGLImage + GL texture + FBO).
    ///
    /// The returned [`ExportBuffer`] can be used as a render target and then
    /// exported as a DMA-BUF via [`Self::export_dmabuf`].
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    #[expect(clippy::too_many_lines, reason = "linear GL resource setup")]
    pub fn allocate_export_buffer(&self, width: u32, height: u32) -> Result<ExportBuffer> {
        use smithay::reexports::gbm::Format;

        anyhow::ensure!(
            width > 0 && height > 0,
            "buffer dimensions must be non-zero"
        );

        tracing::debug!("Allocating {width}x{height} GBM export buffer");

        // Create GBM buffer object
        let bo = self
            .gbm
            .create_buffer_object::<()>(
                width,
                height,
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

        // Create FBO with color + depth attachments.
        // Depth is needed by widgets that use GL_DEPTH_TEST (e.g. 3D flip-clock).
        #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
        let fbo = unsafe {
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

            let depth_rb = self
                .gl
                .create_renderbuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create depth renderbuffer: {e}"))?;
            self.gl
                .bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rb));
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
                Some(depth_rb),
            );

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Export framebuffer incomplete: 0x{status:x}");
            }
            fbo
        };

        let cached_stride = bo.stride();

        tracing::debug!("Export buffer allocated: {width}x{height}, stride={cached_stride}");

        Ok(ExportBuffer {
            bo,
            egl_image,
            texture,
            fbo,
            width,
            height,
            cached_fd: None,
            cached_stride,
        })
    }

    /// Destroy an export buffer, freeing all GPU resources.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "takes ownership to consume the buffer"
    )]
    pub fn destroy_export_buffer(&self, buf: ExportBuffer) {
        unsafe {
            self.gl.delete_framebuffer(buf.fbo);
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
            format: DrmFourcc::Xrgb8888,
            modifier: DrmModifier::Linear,
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
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
    /// Cached DMA-BUF fd (avoids kernel call per frame).
    cached_fd: Option<OwnedFd>,
    /// Cached stride in bytes.
    cached_stride: u32,
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

/// Double-buffered DMA-BUF export state.
///
/// Manages two lazily-allocated [`ExportBuffer`]s with ping-pong swap.
/// Widgets compose this with [`EglContext`] and their own rendering pipeline
/// (direct FBO, staging+blit, etc.).
struct DoubleBufferState {
    buffers: [Option<ExportBuffer>; 2],
    current_buffer: usize,
    width: u32,
    height: u32,
}

impl fmt::Debug for DoubleBufferState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DoubleBufferState")
            .field("current_buffer", &self.current_buffer)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl DoubleBufferState {
    /// Create empty state at the given dimensions. Buffers are allocated lazily
    /// on the first call to [`Self::ensure_current`].
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            buffers: [None, None],
            current_buffer: 0,
            width,
            height,
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

    /// Ensure the current back buffer is allocated, return a reference to it.
    pub fn ensure_current(&mut self, ctx: &EglContext) -> Result<&ExportBuffer> {
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(ctx.allocate_export_buffer(self.width, self.height)?);
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
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            ctx: EglContext::new()?,
            buffers: DoubleBufferState::new(width, height),
        })
    }

    /// Get the glow OpenGL ES context.
    pub fn gl(&self) -> &glow::Context {
        self.ctx.gl()
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
}

impl Drop for DoubleBufferedEglState {
    fn drop(&mut self) {
        self.buffers.destroy_all(&self.ctx);
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
    use super::DoubleBufferState;

    #[test]
    fn double_buffer_state_starts_empty_on_slot_zero() {
        let state = DoubleBufferState::new(640, 480);

        assert_eq!(state.width(), 640);
        assert_eq!(state.height(), 480);
        assert_eq!(state.current_buffer, 0);
        assert!(state.buffers.iter().all(Option::is_none));
        assert!(state.current_ref().is_none());
    }

    #[test]
    fn next_slot_ping_pongs_between_two_slots() {
        assert_eq!(DoubleBufferState::next_slot(0), 1);
        assert_eq!(DoubleBufferState::next_slot(1), 0);
    }
}

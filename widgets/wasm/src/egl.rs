// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! EGL context management for OpenGL ES rendering via GBM
//!
//! This module uses GBM (Generic Buffer Manager) to create EGL context
//! and render to buffers that can be exported as DMA-BUF for Wayland.

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
///
/// Uses a two-FBO pipeline to work around FemtoVG's Y-flip on FBO targets:
/// - **Staging FBO**: regular GL texture + stencil — FemtoVG renders here
/// - **Export FBO**: EGLImage-backed texture — exported as DMA-BUF
///
/// After FemtoVG flushes, `blit_to_export()` copies the staging texture to the
/// export FBO with flipped V coordinates, correcting the Y inversion.
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
    /// Export buffers (EGLImage-backed, for DMA-BUF)
    buffers: [Option<RenderBuffer>; 2],
    /// Current back buffer index
    current_buffer: usize,
    /// Staging FBO where FemtoVG renders (regular GL texture + stencil)
    staging: Option<StagingBuffer>,
    /// Blit shader program (fullscreen quad with Y-flip)
    blit_program: Option<BlitResources>,
    /// Surface width
    width: u32,
    /// Surface height
    height: u32,
}

/// Staging buffer for FemtoVG rendering (regular GL texture, not EGLImage)
struct StagingBuffer {
    /// Color texture
    texture: glow::Texture,
    /// Framebuffer object
    fbo: glow::Framebuffer,
    /// Stencil renderbuffer (required by FemtoVG)
    stencil_rbo: glow::Renderbuffer,
}

/// Resources for the Y-flip blit pass
struct BlitResources {
    /// Shader program
    program: glow::Program,
    /// Vertex buffer (fullscreen quad with flipped UVs)
    vbo: glow::Buffer,
    /// Cached attribute location: a_pos
    pos_loc: u32,
    /// Cached attribute location: a_uv
    uv_loc: u32,
}

/// Export render buffer with GBM BO + EGLImage-backed FBO (no stencil needed)
struct RenderBuffer {
    /// GBM buffer object
    bo: BufferObject<()>,
    /// EGLImage handle
    #[expect(dead_code, reason = "kept alive for texture binding")]
    egl_image: *mut c_void,
    /// OpenGL texture (color attachment, backed by EGLImage)
    texture: glow::Texture,
    /// OpenGL framebuffer object
    fbo: glow::Framebuffer,
    /// Cached DMA-BUF fd (avoids kernel call per frame)
    cached_fd: Option<OwnedFd>,
    /// Cached stride
    cached_stride: u32,
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
            staging: None,
            blit_program: None,
            width,
            height,
        })
    }

    /// Allocate an export render buffer (GBM BO + EGLImage + FBO, no stencil)
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
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
                .map_err(|e| anyhow::anyhow!("Failed to create texture: {}", e))?
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

        // Create export FBO (color-only, no stencil needed)
        let fbo = unsafe {
            self.gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create framebuffer: {}", e))?
        };

        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Export framebuffer incomplete: 0x{:x}", status);
            }
        }

        tracing::debug!("Export FBO created with EGLImage texture");

        // Cache stride now (cheap struct access)
        let cached_stride = bo.stride();

        Ok(RenderBuffer {
            bo,
            egl_image,
            texture,
            fbo,
            cached_fd: None, // Lazily cached on first end_frame
            cached_stride,
        })
    }

    /// Begin a frame — allocate resources if needed, bind the staging FBO.
    ///
    /// Returns the raw GL framebuffer name (u32) of the **staging** FBO so the
    /// renderer can tell FemtoVG which FBO to target via `set_screen_target`.
    /// After FemtoVG flushes, call `blit_to_export()` to copy with Y-flip.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn begin_frame(&mut self) -> Result<u32> {
        // Ensure export buffer exists
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(self.allocate_buffer()?);
        }

        // Ensure staging FBO exists
        if self.staging.is_none() {
            self.staging = Some(self.allocate_staging()?);
        }

        // Ensure blit resources exist
        if self.blit_program.is_none() {
            self.blit_program = Some(self.create_blit_resources()?);
        }

        let staging = self
            .staging
            .as_ref()
            .expect("BUG: staging should exist after allocation");

        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(staging.fbo));
            self.gl
                .viewport(0, 0, self.width as i32, self.height as i32);
            self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
        }

        // Return staging FBO name for FemtoVG
        Ok(staging.fbo.0.get())
    }

    /// Allocate the staging FBO (regular GL texture + stencil for FemtoVG)
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    fn allocate_staging(&self) -> Result<StagingBuffer> {
        tracing::debug!("Allocating staging FBO {}x{}", self.width, self.height);

        let texture = unsafe {
            let tex = self
                .gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("Failed to create staging texture: {}", e))?;
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                self.width as i32,
                self.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
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

        let stencil_rbo = unsafe {
            let rbo = self
                .gl
                .create_renderbuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create stencil RBO: {}", e))?;
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
            self.gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::STENCIL_INDEX8,
                self.width as i32,
                self.height as i32,
            );
            rbo
        };

        let fbo = unsafe {
            let fbo = self
                .gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create staging FBO: {}", e))?;
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

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Staging framebuffer incomplete: 0x{:x}", status);
            }
            fbo
        };

        tracing::debug!("Staging FBO created with texture + stencil");
        Ok(StagingBuffer {
            texture,
            fbo,
            stencil_rbo,
        })
    }

    /// Create the blit shader and fullscreen quad VBO with flipped UVs
    fn create_blit_resources(&self) -> Result<BlitResources> {
        let vert_src = r"#version 100
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
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
            let vs = self
                .gl
                .create_shader(glow::VERTEX_SHADER)
                .map_err(|e| anyhow::anyhow!("Blit VS create: {}", e))?;
            self.gl.shader_source(vs, vert_src);
            self.gl.compile_shader(vs);
            if !self.gl.get_shader_compile_status(vs) {
                let log = self.gl.get_shader_info_log(vs);
                anyhow::bail!("Blit VS compile: {}", log);
            }

            let fs = self
                .gl
                .create_shader(glow::FRAGMENT_SHADER)
                .map_err(|e| anyhow::anyhow!("Blit FS create: {}", e))?;
            self.gl.shader_source(fs, frag_src);
            self.gl.compile_shader(fs);
            if !self.gl.get_shader_compile_status(fs) {
                let log = self.gl.get_shader_info_log(fs);
                anyhow::bail!("Blit FS compile: {}", log);
            }

            let prog = self
                .gl
                .create_program()
                .map_err(|e| anyhow::anyhow!("Blit program create: {}", e))?;
            self.gl.attach_shader(prog, vs);
            self.gl.attach_shader(prog, fs);
            self.gl.link_program(prog);
            self.gl.delete_shader(vs);
            self.gl.delete_shader(fs);
            if !self.gl.get_program_link_status(prog) {
                let log = self.gl.get_program_info_log(prog);
                anyhow::bail!("Blit program link: {}", log);
            }
            prog
        };

        // Fullscreen quad: position (clip space) + UV (flipped V: 1→0 instead of 0→1)
        #[rustfmt::skip]
        let vertices: [f32; 24] = [
            // pos x, pos y, u, v (V is flipped: top of quad samples bottom of texture)
            -1.0, -1.0,  0.0, 1.0,  // bottom-left  → sample top of texture
             1.0, -1.0,  1.0, 1.0,  // bottom-right → sample top-right
             1.0,  1.0,  1.0, 0.0,  // top-right    → sample bottom-right
            -1.0, -1.0,  0.0, 1.0,  // bottom-left
             1.0,  1.0,  1.0, 0.0,  // top-right
            -1.0,  1.0,  0.0, 0.0,  // top-left     → sample bottom-left
        ];

        let vbo = unsafe {
            let buf = self
                .gl
                .create_buffer()
                .map_err(|e| anyhow::anyhow!("Blit VBO create: {}", e))?;
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            let bytes: &[u8] = std::slice::from_raw_parts(
                vertices.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&vertices),
            );
            self.gl
                .buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            buf
        };

        // Cache attribute locations (avoid string lookups per frame)
        let pos_loc = unsafe {
            self.gl
                .get_attrib_location(program, "a_pos")
                .expect("BUG: a_pos not found in blit shader")
        };
        let uv_loc = unsafe {
            self.gl
                .get_attrib_location(program, "a_uv")
                .expect("BUG: a_uv not found in blit shader")
        };

        tracing::debug!("Blit resources created (Y-flip shader + fullscreen quad)");
        Ok(BlitResources {
            program,
            vbo,
            pos_loc,
            uv_loc,
        })
    }

    /// Blit the staging FBO to the export FBO with Y-flip.
    ///
    /// Call this after FemtoVG `flush()` and before `end_frame()`.
    #[expect(clippy::cast_possible_wrap, reason = "GL dimensions fit in i32")]
    pub fn blit_to_export(&self) -> Result<()> {
        let staging = self
            .staging
            .as_ref()
            .context("BUG: staging not allocated")?;
        let blit = self
            .blit_program
            .as_ref()
            .context("BUG: blit resources not allocated")?;
        let export = self.buffers[self.current_buffer]
            .as_ref()
            .context("BUG: export buffer not allocated")?;

        unsafe {
            // Bind export FBO as render target
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(export.fbo));
            self.gl
                .viewport(0, 0, self.width as i32, self.height as i32);

            // Use blit shader
            self.gl.use_program(Some(blit.program));

            // Bind staging texture as source (texture unit 0, uniform defaults to 0)
            self.gl.active_texture(glow::TEXTURE0);
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(staging.texture));

            // Bind VBO and set attributes (using cached locations)
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(blit.vbo));
            self.gl.enable_vertex_attrib_array(blit.pos_loc);
            self.gl
                .vertex_attrib_pointer_f32(blit.pos_loc, 2, glow::FLOAT, false, 16, 0);
            self.gl.enable_vertex_attrib_array(blit.uv_loc);
            self.gl
                .vertex_attrib_pointer_f32(blit.uv_loc, 2, glow::FLOAT, false, 16, 8);

            // Disable stencil test (export FBO has no stencil)
            self.gl.disable(glow::STENCIL_TEST);

            // Draw fullscreen quad
            self.gl.draw_arrays(glow::TRIANGLES, 0, 6);

            // Clean up
            self.gl.disable_vertex_attrib_array(blit.pos_loc);
            self.gl.disable_vertex_attrib_array(blit.uv_loc);
        }

        Ok(())
    }

    /// End frame and get DMA-BUF fd for the rendered buffer
    pub fn end_frame(&mut self) -> Result<DmaBufInfo> {
        use std::os::fd::AsFd;

        // Ensure rendering is complete
        unsafe {
            self.gl.finish();
        }

        let idx = self.current_buffer;
        let buffer = self.buffers[idx]
            .as_mut()
            .expect("BUG: buffer should exist after begin_frame");

        // Cache DMA-BUF fd on first use (avoids drmPrimeHandleToFD per frame)
        if buffer.cached_fd.is_none() {
            let fd = buffer
                .bo
                .fd()
                .context("Failed to get DMA-BUF fd from GBM BO")?;
            buffer.cached_fd = Some(fd);
        }

        // Duplicate the cached fd (dup syscall is cheaper than drmPrimeHandleToFD)
        let fd = buffer
            .cached_fd
            .as_ref()
            .expect("BUG: cached_fd should exist")
            .as_fd()
            .try_clone_to_owned()
            .context("Failed to dup DMA-BUF fd")?;

        let info = DmaBufInfo {
            fd,
            width: self.width,
            height: self.height,
            stride: buffer.cached_stride,
            format: DrmFourcc::Xrgb8888,
            modifier: DrmModifier::Linear,
        };

        // Double buffering: alternate between buffers to avoid stalls
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

        for buffer in &mut self.buffers {
            if let Some(buf) = buffer.take() {
                unsafe {
                    self.gl.delete_framebuffer(buf.fbo);
                    self.gl.delete_texture(buf.texture);
                }
            }
        }

        if let Some(staging) = self.staging.take() {
            unsafe {
                self.gl.delete_framebuffer(staging.fbo);
                self.gl.delete_renderbuffer(staging.stencil_rbo);
                self.gl.delete_texture(staging.texture);
            }
        }

        self.width = width;
        self.height = height;
    }
}

/// Information about a DMA-BUF export
pub struct DmaBufInfo {
    /// DMA-BUF file descriptor (owned)
    pub fd: std::os::fd::OwnedFd,
    /// Buffer width in pixels
    pub width: u32,
    /// Buffer height in pixels
    pub height: u32,
    /// Buffer stride in bytes
    pub stride: u32,
    /// Pixel format (DRM fourcc)
    pub format: DrmFourcc,
    /// Buffer modifier
    pub modifier: DrmModifier,
}

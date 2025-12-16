// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL/OpenGL ES rendering for split GPU/display architecture
//!
//! This module handles GPU-accelerated rendering on systems where the GPU
//! and display controller are separate devices (common on embedded SoCs like STM32MP1).
//!
//! Architecture:
//! - GPU device (etnaviv on renderD128): EGL context, OpenGL ES rendering
//! - Display device (stm32-ltdc on card1): DRM/KMS scanout only
//!
//! Buffer allocation strategy (standard approach for split GPU/display):
//! The display controller (stm32-ltdc) is a simple scanout engine that can only
//! display buffers it allocated itself. The GPU (etnaviv) can import foreign buffers.
//! Therefore we allocate on display and import to GPU:
//!
//! The flow is:
//! 1. Allocate DUMB buffer on DISPLAY device (CMA-backed, scanout-capable)
//! 2. Export dumb buffer as DMA-BUF (PRIME export)
//! 3. Import DMA-BUF into GPU's EGL context as render target
//! 4. GPU renders with OpenGL ES into the buffer
//! 5. Create framebuffer handle from dumb buffer on display device
//! 6. Page flip on display device (buffer is already there, no import needed)

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::{Buffer as AllocatorBuffer, Fourcc, dmabuf::Dmabuf},
        drm::{DrmDevice, DrmDeviceFd, DrmSurface, PlaneConfig, PlaneState},
        egl::{EGLContext, EGLDisplay},
        renderer::{
            Bind, Color32F, Frame, ImportMemWl, Renderer, Texture,
            gles::{GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType},
        },
    },
    reexports::{
        drm::{
            buffer::Buffer as DrmBuffer,
            control::{
                Device as ControlDevice, Mode, connector, crtc, dumbbuffer::DumbBuffer, framebuffer,
            },
        },
        gbm::Device as GbmDevice,
        wayland_server::protocol::wl_buffer::WlBuffer,
    },
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
};
use std::{fs::OpenOptions, os::unix::io::OwnedFd, path::Path};

/// Custom rotation shader - rotates texture around its center
const ROTATION_SHADER: &str = r#"
//_DEFINES
precision mediump float;

uniform float rotation_angle;

varying vec2 v_coords;
uniform sampler2D tex;

void main() {
    // Get center-relative coordinates (-0.5 to 0.5)
    vec2 centered = v_coords - vec2(0.5, 0.5);

    // Rotate around center
    float cos_a = cos(rotation_angle);
    float sin_a = sin(rotation_angle);
    vec2 rotated = vec2(
        centered.x * cos_a - centered.y * sin_a,
        centered.x * sin_a + centered.y * cos_a
    );

    // Back to texture coordinates (0 to 1)
    vec2 tex_coords = rotated + vec2(0.5, 0.5);

    // Sample with transparency for out-of-bounds
    if (tex_coords.x < 0.0 || tex_coords.x > 1.0 || tex_coords.y < 0.0 || tex_coords.y > 1.0) {
        gl_FragColor = vec4(0.0, 0.0, 0.0, 0.0);
    } else {
        gl_FragColor = texture2D(tex, tex_coords);
    }
}
"#;

/// Display dimensions (after 90° rotation from 600x1280 portrait panel)
pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 480;

/// EGL render state for split GPU/display architecture
#[expect(
    missing_debug_implementations,
    reason = "contains non-Debug smithay types"
)]
pub struct EglRenderState {
    // GPU side (rendering)
    /// GBM device for GPU (kept alive for EGL display lifetime)
    #[expect(dead_code, reason = "must be kept alive for EGL display lifetime")]
    gpu_gbm: GbmDevice<DrmDeviceFd>,
    /// EGL display (created from GPU's GBM)
    #[expect(dead_code, reason = "must be kept alive for renderer lifetime")]
    egl_display: EGLDisplay,
    /// OpenGL ES renderer
    renderer: GlesRenderer,

    // Display side (scanout only)
    /// DRM device for display output
    display_drm: DrmDevice,
    /// DRM surface for mode setting
    display_surface: DrmSurface,
    /// Primary plane handle
    primary_plane: smithay::reexports::drm::control::plane::Handle,

    // Buffer management (double buffering)
    /// Render buffers (allocated on GPU, imported to display via DMA-BUF)
    buffers: [Option<RenderBuffer>; 2],
    /// Which buffer is currently being displayed (0 or 1)
    current_slot: usize,

    // State
    /// Display width
    width: u32,
    /// Display height
    height: u32,
    /// Frame counter
    frame_count: u32,
    /// Whether a page flip is pending
    flip_pending: bool,

    // Shaders
    /// Custom rotation shader for arbitrary angle rotation
    rotation_shader: Option<GlesTexProgram>,
}

/// A render buffer with its associated handles
struct RenderBuffer {
    /// Dumb buffer allocated on display device (CMA-backed, scanout-capable)
    #[expect(dead_code, reason = "must be kept alive for framebuffer lifetime")]
    dumb_buffer: DumbBuffer,
    /// DMA-BUF export of the buffer (used for GPU rendering)
    dmabuf: Dmabuf,
    /// Framebuffer handle on display device (for scanout)
    fb: framebuffer::Handle,
}

impl EglRenderState {
    /// Create a new EGL render state with split GPU/display devices
    ///
    /// # Arguments
    /// * `gpu_path` - Path to GPU device (e.g., /dev/dri/renderD128 or /dev/dri/card0)
    /// * `display_path` - Path to display device (e.g., /dev/dri/card1)
    ///
    /// # Errors
    /// Returns error if device initialization, EGL setup, or display configuration fails
    pub fn new(gpu_path: &Path, display_path: &Path) -> Result<Self> {
        tracing::info!("Initializing EGL render state (split GPU/display)");
        tracing::info!("  GPU device: {:?}", gpu_path);
        tracing::info!("  Display device: {:?}", display_path);

        // === Initialize GPU device (for EGL/OpenGL rendering) ===
        let gpu_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(gpu_path)
            .context("Failed to open GPU device")?;

        let gpu_fd = DrmDeviceFd::new(OwnedFd::from(gpu_file).into());
        let gpu_gbm = GbmDevice::new(gpu_fd.clone()).context("Failed to create GPU GBM device")?;

        tracing::info!("GPU GBM device created");

        // Create EGL display from GPU's GBM device
        let egl_display = unsafe { EGLDisplay::new(gpu_gbm.clone()) }
            .context("Failed to create EGL display - check Mesa/etnaviv drivers")?;

        tracing::info!("EGL display created");

        // Create EGL context
        let egl_context = EGLContext::new(&egl_display).context("Failed to create EGL context")?;

        tracing::info!("EGL context created");

        // Create OpenGL ES renderer
        let mut renderer =
            unsafe { GlesRenderer::new(egl_context) }.context("Failed to create GLES renderer")?;

        tracing::info!("GLES renderer created");

        // Compile custom rotation shader
        let rotation_shader = match renderer.compile_custom_texture_shader(
            ROTATION_SHADER,
            &[UniformName::new("rotation_angle", UniformType::_1f)],
        ) {
            Ok(shader) => {
                tracing::info!("Rotation shader compiled successfully");
                Some(shader)
            }
            Err(e) => {
                tracing::warn!("Failed to compile rotation shader: {:?}", e);
                None
            }
        };

        tracing::info!("GPU GBM device ready for buffer allocation");

        // === Initialize display device (for scanout only) ===
        let display_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(display_path)
            .context("Failed to open display device")?;

        let display_fd = DrmDeviceFd::new(OwnedFd::from(display_file).into());
        let (mut display_drm, _notifier) =
            DrmDevice::new(display_fd, false).context("Failed to create display DRM device")?;

        tracing::info!("Display DRM device created");

        // Find connected display and configure mode
        let (connector, crtc, mode) = Self::find_display_config(&display_drm)?;

        tracing::info!(
            "Display configured: {}x{} @ {}Hz",
            mode.size().0,
            mode.size().1,
            mode.vrefresh()
        );

        // Create DRM surface for the display
        let display_surface = display_drm
            .create_surface(crtc, mode, &[connector])
            .context("Failed to create DRM surface")?;

        // Get primary plane
        let planes = display_surface.planes();
        let primary_plane = planes
            .primary
            .first()
            .context("No primary plane available")?
            .handle;

        tracing::info!("Using primary plane: {:?}", primary_plane);

        // The panel reports 600x1280 but only 480x1280 is visible
        // Use the visible dimensions for buffer allocation
        let mode_width = u32::from(mode.size().0);
        let mode_height = u32::from(mode.size().1);

        // Clamp width to visible area (480 pixels visible, panel reports 600)
        let width = if mode_width == 600 { 480 } else { mode_width };
        let height = mode_height;

        tracing::info!(
            "Buffer dimensions: {}x{} (mode reported {}x{})",
            width,
            height,
            mode_width,
            mode_height
        );

        Ok(Self {
            gpu_gbm,
            egl_display,
            renderer,
            display_drm,
            display_surface,
            primary_plane,
            buffers: [None, None],
            current_slot: 0,
            width,
            height,
            frame_count: 0,
            flip_pending: false,
            rotation_shader,
        })
    }

    /// Find a connected display and return connector, CRTC, and mode
    fn find_display_config(drm: &DrmDevice) -> Result<(connector::Handle, crtc::Handle, Mode)> {
        let resources = drm
            .resource_handles()
            .context("Failed to get DRM resources")?;

        for conn_handle in resources.connectors() {
            let conn = drm
                .get_connector(*conn_handle, true)
                .context("Failed to get connector info")?;

            if conn.state() != connector::State::Connected {
                continue;
            }

            tracing::info!(
                "Found connected display: {:?} ({:?})",
                conn_handle,
                conn.interface()
            );

            // Find preferred mode or first available
            let mode = conn
                .modes()
                .iter()
                .find(|m| {
                    m.mode_type()
                        .contains(smithay::reexports::drm::control::ModeTypeFlags::PREFERRED)
                })
                .or_else(|| conn.modes().first())
                .copied()
                .context("No display mode available")?;

            // Find CRTC
            let encoder = conn
                .current_encoder()
                .or_else(|| conn.encoders().first().copied())
                .context("No encoder for connector")?;

            let encoder_info = drm.get_encoder(encoder)?;
            let crtc = encoder_info
                .crtc()
                .or_else(|| resources.crtcs().first().copied())
                .context("No CRTC available")?;

            return Ok((*conn_handle, crtc, mode));
        }

        anyhow::bail!("No connected display found")
    }

    /// Allocate a new render buffer using the standard split GPU/display approach:
    /// 1. Create dumb buffer on display device (CMA-backed, scanout-capable)
    /// 2. PRIME export as DMA-BUF
    /// 3. Import into GPU for rendering
    fn allocate_buffer(&mut self) -> Result<RenderBuffer> {
        tracing::debug!(
            "Allocating {}x{} dumb buffer on display device (standard approach)",
            self.width,
            self.height
        );

        // Step 1: Create dumb buffer on display device
        // stm32-ltdc uses CMA (Contiguous Memory Allocator) for dumb buffers
        let dumb_buffer = self
            .display_drm
            .create_dumb_buffer(
                (self.width, self.height),
                smithay::reexports::drm::buffer::DrmFourcc::Xrgb8888,
                32, // bits per pixel
            )
            .context("Failed to create dumb buffer on display device")?;

        tracing::debug!(
            "Dumb buffer created on display: size={}x{}, pitch={}, handle={:?}",
            dumb_buffer.size().0,
            dumb_buffer.size().1,
            dumb_buffer.pitch(),
            dumb_buffer.handle()
        );

        // Step 2: PRIME export the dumb buffer as DMA-BUF
        let dmabuf_fd = self
            .display_drm
            .buffer_to_prime_fd(dumb_buffer.handle(), 0)
            .context("Failed to PRIME export dumb buffer as DMA-BUF")?;

        tracing::debug!("Dumb buffer PRIME exported as DMA-BUF: fd={:?}", dmabuf_fd);

        // Step 3: Build Dmabuf descriptor for GPU import
        #[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
        let size = (self.width as i32, self.height as i32);
        let format_code = Fourcc::Xrgb8888;
        let modifier = smithay::reexports::drm::buffer::DrmModifier::Linear;

        let mut builder = Dmabuf::builder(
            size,
            format_code,
            modifier,
            smithay::backend::allocator::dmabuf::DmabufFlags::empty(),
        );
        builder.add_plane(dmabuf_fd, 0, 0, dumb_buffer.pitch());
        let dmabuf = builder
            .build()
            .context("Failed to build DMA-BUF descriptor")?;

        tracing::debug!(
            "DMA-BUF descriptor built for GPU import: {} planes, format {:?}",
            dmabuf.num_planes(),
            dmabuf.format()
        );

        // Step 4: Create framebuffer handle on display device
        // The buffer is already on the display device, so this just creates a handle
        let fb = self
            .display_drm
            .add_framebuffer(&dumb_buffer, 24, 32)
            .context("Failed to create framebuffer from dumb buffer")?;

        tracing::debug!("Framebuffer created on display: {:?}", fb);

        Ok(RenderBuffer {
            dumb_buffer,
            dmabuf,
            fb,
        })
    }

    /// Ensure buffer at given slot is allocated, return its dmabuf and fb
    fn ensure_buffer(&mut self, slot: usize) -> Result<(Dmabuf, framebuffer::Handle)> {
        if self.buffers[slot].is_none() {
            self.buffers[slot] = Some(self.allocate_buffer()?);
        }
        let buf = self.buffers[slot]
            .as_ref()
            .expect("BUG: buffer should exist");
        Ok((buf.dmabuf.clone(), buf.fb))
    }

    /// Render a frame
    ///
    /// Renders to the back buffer and queues a page flip.
    /// If `client_buffer` is provided, it will be rendered; otherwise just clears.
    pub fn render_frame(&mut self, client_buffer: Option<&WlBuffer>) -> Result<()> {
        if self.flip_pending {
            return Ok(());
        }

        // Get the back buffer slot
        let back_slot = 1 - self.current_slot;

        // Ensure buffer is allocated and get its handles
        let (mut dmabuf, fb) = self.ensure_buffer(back_slot)?;

        // Import client texture BEFORE starting the frame (to avoid borrow conflicts)
        let client_texture = if let Some(buffer) = client_buffer {
            match self.renderer.import_shm_buffer(buffer, None, &[]) {
                Ok(texture) => Some(texture),
                Err(e) => {
                    tracing::warn!("Failed to import SHM buffer: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        // Bind the DMA-BUF as render target
        let mut framebuffer = self
            .renderer
            .bind(&mut dmabuf)
            .context("Failed to bind render target")?;

        // Begin rendering
        #[expect(
            clippy::cast_possible_wrap,
            reason = "display dimensions are always small enough to fit in i32"
        )]
        let output_size = Size::from((self.width as i32, self.height as i32));
        // No rotation for now - keep it simple
        let mut frame = self
            .renderer
            .render(&mut framebuffer, output_size, Transform::Normal)
            .context("Failed to begin render frame")?;

        // Clear to dark background
        let clear_color = Color32F::new(0.05, 0.05, 0.1, 1.0);
        frame
            .clear(clear_color, &[Rectangle::from_size(output_size)])
            .context("Failed to clear frame")?;

        // Render client texture if we have one
        if let Some(ref texture) = client_texture {
            // Get texture dimensions (widget renders in landscape: 1280x480)
            let tex_size = texture.size();

            // Source rectangle (full texture) - needs f64 for Buffer coordinates
            let src: Rectangle<f64, BufferCoord> = Rectangle::from_loc_and_size(
                (0.0, 0.0),
                (f64::from(tex_size.w), f64::from(tex_size.h)),
            );

            // Animation time
            let time = f64::from(self.frame_count) * 0.016;

            // Rotation speed (radians per second) - full rotation every ~2 seconds
            let rotation_speed = 3.0;
            #[expect(clippy::cast_possible_truncation, reason = "angle fits in f32")]
            let rotation_angle = (time * rotation_speed) as f32;

            // After Transform::_270, texture 1280x480 becomes 480x1280 in buffer
            // Place full size at origin
            let dst_w = tex_size.h; // 480 -> width after rotation
            let dst_h = tex_size.w; // 1280 -> height after rotation
            let dst = Rectangle::from_loc_and_size((0, 0), (dst_w, dst_h));

            tracing::info!(
                "Rotate: frame={}, angle={:.2} rad, dst={}x{}",
                self.frame_count,
                rotation_angle,
                dst_w,
                dst_h
            );

            // Render with custom rotation shader if available, otherwise fallback
            let (shader, uniforms): (Option<&GlesTexProgram>, Vec<Uniform<'_>>) =
                if let Some(ref shader) = self.rotation_shader {
                    (
                        Some(shader),
                        vec![Uniform::new("rotation_angle", rotation_angle)],
                    )
                } else {
                    (None, vec![])
                };

            // Render the texture with 90° CCW base rotation + custom shader rotation
            if let Err(e) = frame.render_texture_from_to(
                texture,
                src,             // src (full texture)
                dst,             // dst (rotated dimensions)
                &[dst],          // damage
                &[],             // opaque_regions
                Transform::_270, // 90° CCW base rotation for display orientation
                1.0,             // alpha
                shader,          // custom rotation shader
                &uniforms,       // rotation angle uniform
            ) {
                tracing::warn!("Failed to render texture: {:?}", e);
            }
        }

        // Finish rendering
        let _sync = frame.finish().context("Failed to finish frame")?;
        drop(framebuffer);

        // Queue page flip
        self.queue_page_flip(fb)?;

        // Swap buffer slots
        self.current_slot = back_slot;
        self.frame_count = self.frame_count.wrapping_add(1);

        Ok(())
    }

    /// Queue a page flip on the display device
    fn queue_page_flip(&mut self, fb: framebuffer::Handle) -> Result<()> {
        let src_size: Size<f64, BufferCoord> =
            Size::from((f64::from(self.width), f64::from(self.height)));
        let src_rect = Rectangle::from_size(src_size);

        #[expect(
            clippy::cast_possible_wrap,
            reason = "display dimensions are always small enough to fit in i32"
        )]
        let dst_size: Size<i32, Physical> = Size::from((self.width as i32, self.height as i32));
        let dst_rect = Rectangle::from_size(dst_size);

        let plane_config = PlaneConfig {
            src: src_rect,
            dst: dst_rect,
            transform: Transform::Normal,
            alpha: 1.0,
            damage_clips: None,
            fb,
            fence: None,
        };

        let plane_state = PlaneState {
            handle: self.primary_plane,
            config: Some(plane_config),
        };

        if self.frame_count == 0 {
            // Initial commit
            self.display_surface
                .commit([plane_state].into_iter(), true)
                .context("Failed to commit initial frame")?;
            tracing::info!("Initial frame committed");
        } else {
            // Page flip
            self.display_surface
                .page_flip([plane_state].into_iter(), true)
                .context("Failed to page flip")?;
        }

        self.flip_pending = true;

        Ok(())
    }

    /// Called when vblank/page flip complete event is received
    pub fn on_vblank(&mut self) {
        self.flip_pending = false;
    }

    /// Check if a page flip is pending
    #[must_use]
    pub fn is_flip_pending(&self) -> bool {
        self.flip_pending
    }

    /// Get display dimensions
    #[must_use]
    pub fn display_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Get the display DRM device (for event loop integration)
    #[must_use]
    pub fn display_drm(&self) -> &DrmDevice {
        &self.display_drm
    }

    /// Get frame count
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }
}

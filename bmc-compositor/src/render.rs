// Copyright (C) 2025  Braiins Systems s.r.o.

//! Simple software rendering using DRM dumb buffers
//!
//! This module provides a basic rendering backend that uses DRM dumb buffers
//! for software rendering. This is simpler than OpenGL and works on any DRM device.

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        drm::{DrmDevice, DrmSurface, PlaneConfig, PlaneState},
    },
    reexports::{
        drm::{
            buffer::Buffer,
            control::{Device as ControlDevice, dumbbuffer::DumbBuffer, framebuffer},
        },
        wayland_server::protocol::wl_buffer::WlBuffer,
    },
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
    wayland::shm,
};

use crate::drm_backend::DeviceState;

/// Simple framebuffer for software rendering
#[expect(
    missing_debug_implementations,
    reason = "contains non-Debug DumbBuffer type"
)]
pub struct SoftwareFramebuffer {
    /// The dumb buffer
    buffer: DumbBuffer,
    /// Framebuffer handle for scanout
    fb: framebuffer::Handle,
    /// Buffer dimensions
    width: u32,
    height: u32,
    /// Bytes per pixel (4 for XRGB8888)
    bpp: u32,
    /// Pitch (bytes per row)
    pitch: u32,
}

impl SoftwareFramebuffer {
    /// Create a new dumb buffer framebuffer
    pub fn new(drm: &DrmDevice, width: u32, height: u32) -> Result<Self> {
        // Create dumb buffer (XRGB8888 format, 32 bits per pixel)
        let buffer = drm
            .create_dumb_buffer((width, height), drm_fourcc::DrmFourcc::Xrgb8888, 32)
            .context("Failed to create dumb buffer")?;

        // Create framebuffer from the dumb buffer
        let fb = drm
            .add_framebuffer(&buffer, 24, 32)
            .context("Failed to create framebuffer")?;

        let pitch = buffer.pitch();

        Ok(Self {
            buffer,
            fb,
            width,
            height,
            bpp: 4,
            pitch,
        })
    }

    /// Get the framebuffer handle for scanout
    pub fn framebuffer(&self) -> framebuffer::Handle {
        self.fb
    }

    /// Get buffer dimensions
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Map the buffer for CPU access and fill with a solid color
    #[allow(dead_code)]
    pub fn fill_color(&mut self, drm: &DrmDevice, r: u8, g: u8, b: u8) -> Result<()> {
        let mut mapping = drm
            .map_dumb_buffer(&mut self.buffer)
            .context("Failed to map dumb buffer")?;

        let pixel = u32::from(b) | (u32::from(g) << 8) | (u32::from(r) << 16);

        // Fill the buffer with the color
        for y in 0..self.height {
            for x in 0..self.width {
                let offset = (y * self.pitch + x * self.bpp) as usize;
                if offset + 4 <= mapping.len() {
                    mapping[offset..offset + 4].copy_from_slice(&pixel.to_le_bytes());
                }
            }
        }

        Ok(())
    }

    /// Map the buffer for CPU access and draw a test pattern
    #[allow(dead_code)]
    pub fn draw_test_pattern(&mut self, drm: &DrmDevice, frame: u32) -> Result<()> {
        let mut mapping = drm
            .map_dumb_buffer(&mut self.buffer)
            .context("Failed to map dumb buffer")?;

        // Draw colored bars that move with frame count
        for y in 0..self.height {
            for x in 0..self.width {
                // Create moving color bars
                let bar_width = self.width / 8;
                let bar_index = ((x + frame) / bar_width) % 8;

                let (r, g, b): (u8, u8, u8) = match bar_index {
                    0 => (255, 0, 0),     // Red
                    1 => (0, 255, 0),     // Green
                    2 => (0, 0, 255),     // Blue
                    3 => (255, 255, 0),   // Yellow
                    4 => (255, 0, 255),   // Magenta
                    5 => (0, 255, 255),   // Cyan
                    6 => (255, 255, 255), // White
                    _ => (128, 128, 128), // Gray
                };

                let pixel = u32::from(b) | (u32::from(g) << 8) | (u32::from(r) << 16);
                let offset = (y * self.pitch + x * self.bpp) as usize;
                if offset + 4 <= mapping.len() {
                    mapping[offset..offset + 4].copy_from_slice(&pixel.to_le_bytes());
                }
            }
        }

        Ok(())
    }

    /// Clear the buffer to black
    pub fn clear(&mut self, drm: &DrmDevice) -> Result<()> {
        let mut mapping = drm
            .map_dumb_buffer(&mut self.buffer)
            .context("Failed to map dumb buffer")?;

        // Fill with black
        mapping.fill(0);

        Ok(())
    }

    /// Copy a client's SHM buffer to this framebuffer at a given offset with rotation
    pub fn draw_buffer_at(
        &mut self,
        drm: &DrmDevice,
        buffer: &WlBuffer,
        offset_x: i32,
        offset_y: i32,
        rotate_90: bool,
    ) -> Result<bool> {
        // Map our framebuffer for writing
        let mut mapping = drm
            .map_dumb_buffer(&mut self.buffer)
            .context("Failed to map dumb buffer")?;

        // Access the SHM buffer contents
        let result = shm::with_buffer_contents(&buffer, |ptr, len, data| {
            let src_width = data.width as u32;
            let src_height = data.height as u32;
            let src_stride = data.stride as u32;
            let src_offset = data.offset as usize;

            // Convert shm format to fourcc
            let format = shm::shm_format_to_fourcc(data.format);

            tracing::debug!(
                "Drawing surface: {}x{}, stride={}, format: {:?}",
                src_width,
                src_height,
                src_stride,
                format
            );

            // Copy pixels, handling format conversion
            // Cast to i32 for offset math (display dimensions are always small enough)
            let dst_width = self.width as i32;
            let dst_height = self.height as i32;

            // Handle different pixel formats
            let bytes_per_pixel: u32 = match format {
                Some(Fourcc::Argb8888) | Some(Fourcc::Xrgb8888) => 4,
                Some(Fourcc::Rgb888) => 3,
                _ => {
                    tracing::warn!("Unsupported format: {:?}", format);
                    return;
                }
            };

            // Create a slice from the raw pointer
            let src_data = unsafe { std::slice::from_raw_parts(ptr, len) };

            // Copy each pixel with offset and optional rotation
            for src_y in 0..src_height {
                for src_x in 0..src_width {
                    // Calculate destination position with offset and optional 90° rotation
                    let (dst_x, dst_y) = if rotate_90 {
                        // Rotate 90° counter-clockwise: (x, y) -> (y, width - 1 - x)
                        let rotated_x = src_y as i32;
                        let rotated_y = (src_width - 1 - src_x) as i32;
                        (rotated_x + offset_x, rotated_y + offset_y)
                    } else {
                        (src_x as i32 + offset_x, src_y as i32 + offset_y)
                    };

                    // Skip if outside destination bounds
                    if dst_x < 0 || dst_y < 0 || dst_x >= dst_width || dst_y >= dst_height {
                        continue;
                    }

                    let src_idx =
                        src_offset + (src_y * src_stride + src_x * bytes_per_pixel) as usize;
                    let dst_idx = (dst_y as u32 * self.pitch + dst_x as u32 * self.bpp) as usize;

                    if src_idx + bytes_per_pixel as usize <= src_data.len()
                        && dst_idx + 4 <= mapping.len()
                    {
                        match format {
                            Some(Fourcc::Argb8888) | Some(Fourcc::Xrgb8888) => {
                                // Direct copy for matching formats
                                mapping[dst_idx..dst_idx + 4]
                                    .copy_from_slice(&src_data[src_idx..src_idx + 4]);
                            }
                            Some(Fourcc::Rgb888) => {
                                // Convert RGB888 to XRGB8888
                                mapping[dst_idx] = src_data[src_idx]; // B
                                mapping[dst_idx + 1] = src_data[src_idx + 1]; // G
                                mapping[dst_idx + 2] = src_data[src_idx + 2]; // R
                                mapping[dst_idx + 3] = 0xFF; // X
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        match result {
            Ok(()) => Ok(true),
            Err(e) => {
                tracing::warn!("Failed to access buffer contents: {:?}", e);
                Ok(false)
            }
        }
    }
}

/// Render state for software rendering
#[expect(
    missing_debug_implementations,
    reason = "contains non-Debug smithay types"
)]
pub struct RenderState {
    /// DRM surface for scanout
    surface: DrmSurface,
    /// Double-buffered framebuffers
    framebuffers: [SoftwareFramebuffer; 2],
    /// Current front buffer index
    current_buffer: usize,
    /// Frame counter for animation
    frame_count: u32,
    /// Primary plane handle
    primary_plane: smithay::reexports::drm::control::plane::Handle,
    /// Display width
    width: i32,
    /// Display height
    height: i32,
    /// Whether a page flip is pending (waiting for vblank)
    flip_pending: bool,
    /// Animation position (x offset)
    animation_x: f32,
    /// Animation direction (1.0 = right, -1.0 = left)
    animation_dir: f32,
}

impl RenderState {
    /// Create a new render state
    pub fn new(device: &mut DeviceState) -> Result<Self> {
        let connector = device.connector.context("No connector configured")?;
        let crtc = device.crtc.context("No CRTC configured")?;
        let mode = device.mode.context("No mode configured")?;

        // Create DRM surface
        let surface = device
            .drm
            .create_surface(crtc, mode, &[connector])
            .context("Failed to create DRM surface")?;

        tracing::info!("DRM surface created: {}x{}", mode.size().0, mode.size().1);

        // Get the primary plane
        let planes = surface.planes();
        let primary_plane = planes
            .primary
            .iter()
            .next()
            .context("No primary plane available")?
            .handle;

        tracing::info!("Using primary plane: {:?}", primary_plane);

        // Create double-buffered framebuffers
        let width = u32::from(mode.size().0);
        let height = u32::from(mode.size().1);

        let fb0 = SoftwareFramebuffer::new(&device.drm, width, height)
            .context("Failed to create framebuffer 0")?;
        let fb1 = SoftwareFramebuffer::new(&device.drm, width, height)
            .context("Failed to create framebuffer 1")?;

        tracing::info!("Created double-buffered framebuffers: {}x{}", width, height);

        #[expect(clippy::cast_possible_wrap, reason = "display dimensions are small")]
        Ok(Self {
            surface,
            framebuffers: [fb0, fb1],
            current_buffer: 0,
            frame_count: 0,
            primary_plane,
            width: width as i32,
            height: height as i32,
            flip_pending: false,
            animation_x: 0.0,
            animation_dir: 1.0,
        })
    }

    /// Render a frame and flip to display
    pub fn render_frame(&mut self, drm: &DrmDevice, buffer: Option<&WlBuffer>) -> Result<()> {
        // Skip if a flip is already pending
        if self.flip_pending {
            return Ok(());
        }

        // Get the back buffer (opposite of current)
        let back_buffer = 1 - self.current_buffer;

        // Clear to black first
        self.framebuffers[back_buffer].clear(drm)?;

        // Update animation - bounce left to right (after 90° CCW rotation, X becomes Y)
        let animation_speed = 10.0; // pixels per frame (faster)
        let max_offset = 400.0; // maximum offset

        self.animation_x += animation_speed * self.animation_dir;
        if self.animation_x >= max_offset {
            self.animation_x = max_offset;
            self.animation_dir = -1.0;
        } else if self.animation_x <= 0.0 {
            self.animation_x = 0.0;
            self.animation_dir = 1.0;
        }

        // After 90° CCW rotation: src(x,y) -> dst(y, width-1-x)
        // To move left-right on screen, offset Y (which becomes dst X after rotation)
        let offset_x = 0;
        let offset_y = self.animation_x as i32;

        // Draw the buffer if available (with 90° rotation for portrait panel)
        let rotate_90 = true;
        if let Some(buffer) = buffer {
            match self.framebuffers[back_buffer]
                .draw_buffer_at(drm, buffer, offset_x, offset_y, rotate_90)
            {
                Ok(true) => {
                    tracing::trace!("Buffer drawn successfully at x={}", offset_x);
                }
                Ok(false) => {
                    tracing::trace!("Buffer could not be drawn");
                }
                Err(e) => {
                    tracing::warn!("Failed to draw buffer: {}", e);
                }
            }
        }

        // Page flip to show the back buffer
        let fb = self.framebuffers[back_buffer].framebuffer();

        // Create plane config for the framebuffer
        // Source rectangle is in buffer coordinates (f64)
        let src_size: Size<f64, BufferCoord> =
            Size::from((f64::from(self.width), f64::from(self.height)));
        let src_rect = Rectangle::from_size(src_size);

        // Destination rectangle is in physical coordinates (i32)
        let dst_size: Size<i32, Physical> = Size::from((self.width, self.height));
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

        // Use commit for initial frame, page_flip for subsequent
        if self.frame_count == 0 {
            self.surface
                .commit([plane_state].into_iter(), true)
                .context("Failed to commit initial frame")?;
            tracing::info!("Initial frame committed successfully");
            self.flip_pending = true;
        } else {
            match self.surface.page_flip([plane_state].into_iter(), true) {
                Ok(()) => {
                    self.flip_pending = true;
                }
                Err(e) => {
                    // Log detailed error info
                    tracing::debug!("Page flip error: {:?}", e);
                    return Err(anyhow::anyhow!("Failed to page flip: {}", e));
                }
            }
        }

        // Swap buffers
        self.current_buffer = back_buffer;
        self.frame_count = self.frame_count.wrapping_add(1);

        Ok(())
    }

    /// Called when a vblank event is received (page flip completed)
    pub fn on_vblank(&mut self) {
        self.flip_pending = false;
    }

    /// Check if a flip is pending
    pub fn is_flip_pending(&self) -> bool {
        self.flip_pending
    }
}

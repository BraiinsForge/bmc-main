// Copyright (C) 2025  Braiins Systems s.r.o.

//! Double-buffer management for split GPU/display architecture.

use anyhow::{Context, Result};
use smithay::{
    backend::allocator::{Fourcc, dmabuf::Dmabuf},
    reexports::drm::{
        buffer::{Buffer as DrmBuffer, DrmFourcc},
        control::{Device as ControlDevice, dumbbuffer::DumbBuffer, framebuffer},
    },
};

use super::drm_output::DrmOutput;

/// Pixel format of a scanout/render buffer allocated by the pool.
#[derive(Clone, Copy)]
pub enum ScanoutFormat {
    Xrgb8888,
    Rgb565,
}

impl ScanoutFormat {
    fn drm_fourcc(self) -> DrmFourcc {
        match self {
            Self::Xrgb8888 => DrmFourcc::Xrgb8888,
            Self::Rgb565 => DrmFourcc::Rgb565,
        }
    }

    fn alloc_fourcc(self) -> Fourcc {
        match self {
            Self::Xrgb8888 => Fourcc::Xrgb8888,
            Self::Rgb565 => Fourcc::Rgb565,
        }
    }

    fn bpp(self) -> u32 {
        match self {
            Self::Xrgb8888 => 32,
            Self::Rgb565 => 16,
        }
    }

    fn depth(self) -> u32 {
        match self {
            Self::Xrgb8888 => 24,
            Self::Rgb565 => 16,
        }
    }
}

pub struct RenderBuffer {
    // Kept alive for framebuffer lifetime
    _dumb_buffer: DumbBuffer,
    pub dmabuf: Dmabuf,
    pub fb: framebuffer::Handle,
}

pub struct BufferPool {
    buffers: [Option<RenderBuffer>; 2],
    current_slot: usize,
    width: u32,
    height: u32,
    format: ScanoutFormat,
}

impl BufferPool {
    pub fn new(width: u32, height: u32, format: ScanoutFormat) -> Self {
        Self {
            buffers: [None, None],
            current_slot: 0,
            width,
            height,
            format,
        }
    }

    pub fn back_buffer(&mut self, output: &DrmOutput) -> Result<&mut RenderBuffer> {
        let slot = 1 - self.current_slot;
        if self.buffers[slot].is_none() {
            self.buffers[slot] = Some(Self::allocate_buffer(
                output,
                self.width,
                self.height,
                self.format,
            )?);
        }
        Ok(self.buffers[slot]
            .as_mut()
            .expect("BUG: buffer should exist"))
    }

    pub fn swap(&mut self) {
        self.current_slot = 1 - self.current_slot;
    }

    fn allocate_buffer(
        output: &DrmOutput,
        width: u32,
        height: u32,
        format: ScanoutFormat,
    ) -> Result<RenderBuffer> {
        tracing::debug!(
            "Allocating {}x{} dumb buffer on display device",
            width,
            height
        );

        let dumb_buffer = output
            .drm()
            .create_dumb_buffer((width, height), format.drm_fourcc(), format.bpp())
            .context("Failed to create dumb buffer")?;

        tracing::debug!(
            "Dumb buffer created: size={}x{}, pitch={}, handle={:?}",
            dumb_buffer.size().0,
            dumb_buffer.size().1,
            dumb_buffer.pitch(),
            dumb_buffer.handle()
        );

        let dmabuf_fd = output
            .drm()
            .buffer_to_prime_fd(dumb_buffer.handle(), 0)
            .context("Failed to PRIME export dumb buffer")?;

        #[expect(clippy::cast_possible_wrap)]
        let size = (width as i32, height as i32);
        let modifier = smithay::reexports::drm::buffer::DrmModifier::Linear;

        let mut builder = Dmabuf::builder(
            size,
            format.alloc_fourcc(),
            modifier,
            smithay::backend::allocator::dmabuf::DmabufFlags::empty(),
        );
        builder.add_plane(dmabuf_fd, 0, 0, dumb_buffer.pitch());
        let dmabuf = builder
            .build()
            .context("Failed to build DMA-BUF descriptor")?;

        let fb = output
            .drm()
            .add_framebuffer(&dumb_buffer, format.depth(), format.bpp())
            .context("Failed to create framebuffer")?;

        tracing::debug!("Framebuffer created: {:?}", fb);

        Ok(RenderBuffer {
            _dumb_buffer: dumb_buffer,
            dmabuf,
            fb,
        })
    }
}

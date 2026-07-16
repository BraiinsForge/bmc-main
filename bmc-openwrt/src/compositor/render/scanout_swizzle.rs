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

//! GPU output pass that rewrites the composited RGB frame as BGR565 bytes.
//!
//! The ST7365P panel on BMM products expects red and blue swapped. The plane
//! advertises only `RG16`/`XR24`, so the swap cannot be expressed through the
//! fourcc and must be produced in the pixels. This pass samples the natural-RGB
//! XRGB8888 intermediate (left untouched for the capture path) and writes a
//! `.bgr`-swizzled `RG16` scanout buffer that is page-flipped instead.

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::{
            Bind, Frame as RendererFrame, ImportDma, Renderer,
            gles::{GlesRenderer, GlesTexProgram},
        },
    },
    reexports::drm::control::framebuffer,
    utils::{Buffer as BufferCoord, Rectangle, Size, Transform},
};

use super::buffer_pool::ScanoutFormat;
use super::{BufferPool, DrmOutput};

const SWIZZLE_SHADER: &str = include_str!("scanout_swizzle.frag");

pub struct ScanoutSwizzler {
    buffers: BufferPool,
    program: GlesTexProgram,
    width: u32,
    height: u32,
}

impl ScanoutSwizzler {
    pub fn new(renderer: &mut GlesRenderer, width: u32, height: u32) -> Result<Self> {
        let program = renderer
            .compile_custom_texture_shader(SWIZZLE_SHADER, &[])
            .context("Failed to compile BGR565 swizzle shader")?;
        Ok(Self {
            buffers: BufferPool::new(width, height, ScanoutFormat::Rgb565),
            program,
            width,
            height,
        })
    }

    /// Sample the composited XRGB8888 intermediate and write a BGR565 scanout
    /// buffer, returning its framebuffer handle for page-flip.
    pub fn present(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &DrmOutput,
        intermediate: &Dmabuf,
    ) -> Result<framebuffer::Handle> {
        let texture = renderer
            .import_dmabuf(intermediate, None)
            .context("Failed to import composited buffer as swizzle source")?;

        let scanout = self.buffers.back_buffer(output)?;
        let fb = scanout.fb;

        let mut framebuffer = renderer
            .bind(&mut scanout.dmabuf)
            .context("Failed to bind BGR565 scanout target")?;

        #[expect(clippy::cast_possible_wrap)]
        let size = Size::from((self.width as i32, self.height as i32));
        let dst = Rectangle::from_size(size);
        let src: Rectangle<f64, BufferCoord> =
            Rectangle::from_size(Size::from((f64::from(self.width), f64::from(self.height))));

        let mut frame = renderer
            .render(&mut framebuffer, size, Transform::Normal)
            .context("Failed to begin swizzle frame")?;
        frame
            .render_texture_from_to(
                &texture,
                src,
                dst,
                &[dst],
                &[],
                Transform::Normal,
                1.0,
                Some(&self.program),
                &[],
            )
            .context("Failed to render BGR565 swizzle pass")?;
        let _sync = frame.finish().context("Failed to finish swizzle frame")?;
        drop(framebuffer);

        self.buffers.swap();
        Ok(fb)
    }
}

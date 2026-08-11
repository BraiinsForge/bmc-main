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

use std::fmt::Write as _;
use std::ptr::NonNull;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

#[must_use]
pub(super) fn compiled_empty_svg() -> Vec<u8> {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&0_u16.to_le_bytes());
    data
}

#[must_use]
pub(super) fn one_px_png(rgba: [u8; 4]) -> Vec<u8> {
    let image: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba(rgba));
    let mut data = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut data, ImageFormat::Png)
        .expect("BUG: PNG fixture must encode");
    data.into_inner()
}

#[must_use]
pub(super) fn wat_string_literal(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 4);
    for byte in bytes {
        write!(output, "\\{byte:02x}").expect("BUG: write to String cannot fail");
    }
    output
}

pub(super) fn renderer_ptr(renderer: &mut FemtoVgRenderer) -> NonNull<dyn Renderer> {
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null")
}

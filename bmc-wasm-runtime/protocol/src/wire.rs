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

//! Little-endian primitives shared by the wire codecs. Readers advance `*pos`
//! and return `None` on truncated input; writers append at `*pos` into a buffer
//! the caller has already sized to the struct's `SIZE`.

use crate::colors::Color;

pub(crate) fn read_u16(data: &[u8], pos: &mut usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(*pos..*pos + 2)?.try_into().ok()?;
    *pos += 2;
    Some(u16::from_le_bytes(bytes))
}

pub(crate) fn read_u32(data: &[u8], pos: &mut usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(*pos..*pos + 4)?.try_into().ok()?;
    *pos += 4;
    Some(u32::from_le_bytes(bytes))
}

pub(crate) fn read_f32(data: &[u8], pos: &mut usize) -> Option<f32> {
    let bytes: [u8; 4] = data.get(*pos..*pos + 4)?.try_into().ok()?;
    *pos += 4;
    Some(f32::from_le_bytes(bytes))
}

pub(crate) fn read_color(data: &[u8], pos: &mut usize) -> Option<Color> {
    Some(Color::from_raw(read_u32(data, pos)?))
}

pub(crate) fn write_u16(buf: &mut [u8], pos: &mut usize, v: u16) {
    buf[*pos..*pos + 2].copy_from_slice(&v.to_le_bytes());
    *pos += 2;
}

pub(crate) fn write_u32(buf: &mut [u8], pos: &mut usize, v: u32) {
    buf[*pos..*pos + 4].copy_from_slice(&v.to_le_bytes());
    *pos += 4;
}

pub(crate) fn write_f32(buf: &mut [u8], pos: &mut usize, v: f32) {
    buf[*pos..*pos + 4].copy_from_slice(&v.to_le_bytes());
    *pos += 4;
}

pub(crate) fn write_color(buf: &mut [u8], pos: &mut usize, c: Color) {
    write_u32(buf, pos, c.to_u32());
}

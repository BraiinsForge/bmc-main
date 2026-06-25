// Copyright (C) 2026  Braiins Systems s.r.o.

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

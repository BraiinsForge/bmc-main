// Copyright (C) 2026  Braiins Systems s.r.o.

//! Little-endian primitive readers shared by the wire decoders. Each advances
//! `*pos` past the bytes it consumes and returns `None` on truncated input.

use crate::colors::Color;

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

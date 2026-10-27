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

//! SVG binary format constants shared between the proc macro and host runtime.
//!
//! Binary format (emitted by `include_svg!`, parsed by host `SvgRegistry`):
//!
//! ```text
//! [viewbox_w: f32][viewbox_h: f32][path_count: u16]
//!   for each path:
//!     [flags: u8]              // bit 0: has_fill, bit 1: has_stroke,
//!                              // bit 2: even-odd fill, bit 3: has_id
//!     [fill_color: u32]        // RGBA, present if has_fill
//!     [stroke_color: u32]      // RGBA, present if has_stroke
//!     [stroke_width: f32]      // present if has_stroke
//!     [id_len: u16][id_bytes]  // UTF-8, present if has_id
//!     [op_count: u16]
//!       for each op: [type: u8][data...]
//! ```

/// Path operation: move to (x, y) — 2×f32
pub const SVG_OP_MOVE_TO: u8 = 0x00;

/// Path operation: line to (x, y) — 2×f32
pub const SVG_OP_LINE_TO: u8 = 0x01;

/// Path operation: quadratic bezier (cx, cy, x, y) — 4×f32
pub const SVG_OP_QUAD_TO: u8 = 0x02;

/// Path operation: cubic bezier (cx1, cy1, cx2, cy2, x, y) — 6×f32
pub const SVG_OP_CUBIC_TO: u8 = 0x03;

/// Path operation: close path — no data
pub const SVG_OP_CLOSE: u8 = 0x04;

/// Flag bit: path has a fill color
pub const SVG_FLAG_HAS_FILL: u8 = 0x01;

/// Flag bit: path has a stroke
pub const SVG_FLAG_HAS_STROKE: u8 = 0x02;

/// Flag bit: path uses even-odd fill rule (instead of default non-zero)
pub const SVG_FLAG_EVENODD: u8 = 0x04;

/// Flag bit: path carries an `id` string (`[id_len: u16][id_bytes]`
/// follow the colour / stroke-width block). Widgets address paths
/// by id when calling `Draw::svg(...).fill(id, color)` to override
/// per-path fills.
pub const SVG_FLAG_HAS_ID: u8 = 0x08;

/// The viewBox a compiled icon opens with, when both sides are positive and finite;
/// `None` for data too short or sized so it cannot be drawn.
#[must_use]
pub fn svg_viewbox(data: &[u8]) -> Option<(f32, f32)> {
    let side = |offset: usize| {
        let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
        let side = f32::from_le_bytes(bytes);
        (side.is_normal() && side > 0.0).then_some(side)
    };
    Some((side(0)?, side(4)?))
}

/// The length of the compiled icon `data` opens with, walked field by field as the host's parser reads it;
/// `None` when the data ends inside it, an id is not UTF-8, or an op is one the parser does not know.
#[must_use]
pub fn compiled_svg_len(data: &[u8]) -> Option<usize> {
    let mut cursor = Cursor { data, at: 0 };
    cursor.take(8)?; // viewbox_w, viewbox_h
    for _ in 0..cursor.u16()? {
        let flags = cursor.u8()?;
        if flags & SVG_FLAG_HAS_FILL != 0 {
            cursor.take(4)?;
        }
        if flags & SVG_FLAG_HAS_STROKE != 0 {
            cursor.take(8)?;
        }
        if flags & SVG_FLAG_HAS_ID != 0 {
            let len = cursor.u16()?;
            core::str::from_utf8(cursor.take(usize::from(len))?).ok()?;
        }
        for _ in 0..cursor.u16()? {
            let floats = match cursor.u8()? {
                SVG_OP_MOVE_TO | SVG_OP_LINE_TO => 2,
                SVG_OP_QUAD_TO => 4,
                SVG_OP_CUBIC_TO => 6,
                SVG_OP_CLOSE => 0,
                _ => return None,
            };
            cursor.take(floats * 4)?;
        }
    }
    Some(cursor.at)
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let bytes = self.data.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
}

// ── Built-in icon IDs ───────────────────────────────────────────────
// Reserved range `SVG_RESERVED_MIN..=0xFFFF` for host-bundled SVGs.
// User-registered SVGs get IDs `1..SVG_RESERVED_MIN`; the host registry
// must refuse new user allocations once it reaches `SVG_RESERVED_MIN`.

use crate::ids::SvgId;

/// First SVG ID reserved for host-bundled (builtin + dev) icons.
///
/// User-icon allocators must stop before this value. `0xFE00..=0xFEFF`
/// is dev/testbed icons (see [`ICON_DEV_CAMERA`] etc.) and `0xFF00..=0xFFFF`
/// is builtin icons (see [`ICON_BUILTIN_BASE`] etc.).
pub const SVG_RESERVED_MIN: u16 = 0xFE00;

/// `const`-context lift of a known-non-zero builtin ID.
/// Panics at compile time if a literal is mistyped as zero.
const fn builtin(raw: u16) -> SvgId {
    match SvgId::from_wire(raw) {
        Some(id) => id,
        None => panic!("BUG: builtin SVG id must be non-zero"),
    }
}

/// Base ID for built-in icons.
pub const ICON_BUILTIN_BASE: SvgId = builtin(0xFF00);

/// Close (X) icon — used by modal close button.
pub const ICON_CLOSE: SvgId = builtin(0xFF01);

/// Error icon (filled circle with diagonal line) — notification error state.
pub const ICON_ERROR: SvgId = builtin(0xFF10);

/// Warning icon (filled triangle with exclamation) — notification warning state.
pub const ICON_WARNING: SvgId = builtin(0xFF11);

/// Success icon (filled circle with checkmark) — notification success state.
pub const ICON_SUCCESS: SvgId = builtin(0xFF12);

/// Info icon (filled circle with "i") — notification info state.
pub const ICON_INFO: SvgId = builtin(0xFF13);

/// Meter / gauge icon — fuel budget indicator.
pub const ICON_METER: SvgId = builtin(0xFF14);

/// Minus / subtract icon — NumberInput decrement stepper.
pub const ICON_MINUS: SvgId = builtin(0xFF15);

/// Plus / add icon — NumberInput increment stepper.
pub const ICON_PLUS: SvgId = builtin(0xFF16);

/// Warning icon (filled triangle) — NumberInput warning state.
pub const ICON_WARN_ALT: SvgId = builtin(0xFF17);

/// Error icon (filled circle with "!") — NumberInput error state.
pub const ICON_WARN_FILLED: SvgId = builtin(0xFF18);

// ── Dev / testbed-only icons ───────────────────────────────────────
// Used in the recording panel event log. Range 0xFE00..=0xFEFF.

/// Camera icon — capture event.
pub const ICON_DEV_CAMERA: SvgId = builtin(0xFE01);

/// Cursor / pointer icon — click and drag events.
pub const ICON_DEV_CURSOR: SvgId = builtin(0xFE02);

/// Scroll icon — scroll events.
pub const ICON_DEV_SCROLL: SvgId = builtin(0xFE03);

/// Download arrow icon — inbound network data (fetch, ws message, socket data, etc.).
pub const ICON_DEV_DOWNLOAD: SvgId = builtin(0xFE04);

/// Upload arrow icon — outbound / connection open (ws open, socket connected).
pub const ICON_DEV_UPLOAD: SvgId = builtin(0xFE05);

/// Broken link icon — disconnect / close (ws close, socket closed, ssdp/mdns removed).
pub const ICON_DEV_UNLINK: SvgId = builtin(0xFE06);

#[cfg(test)]
mod tests {
    use super::{
        SVG_FLAG_HAS_FILL, SVG_FLAG_HAS_ID, SVG_OP_CLOSE, SVG_OP_MOVE_TO, compiled_svg_len,
        svg_viewbox,
    };

    fn header(width: f32, height: f32) -> Vec<u8> {
        [width.to_le_bytes(), height.to_le_bytes()].concat()
    }

    /// One filled path with an id, moving to a point and closing.
    fn icon() -> Vec<u8> {
        [
            header(24.0, 24.0),
            1_u16.to_le_bytes().to_vec(),
            vec![SVG_FLAG_HAS_FILL | SVG_FLAG_HAS_ID],
            0xff00_00ff_u32.to_le_bytes().to_vec(),
            2_u16.to_le_bytes().to_vec(),
            b"cn".to_vec(),
            2_u16.to_le_bytes().to_vec(),
            vec![SVG_OP_MOVE_TO],
            [1.0_f32.to_le_bytes(), 2.0_f32.to_le_bytes()].concat(),
            vec![SVG_OP_CLOSE],
        ]
        .concat()
    }

    #[test]
    fn a_whole_icon_is_walked_to_its_end() {
        let icon = icon();
        assert_eq!(compiled_svg_len(&icon), Some(icon.len()));
    }

    #[test]
    fn an_icon_cut_short_or_with_an_unknown_op_has_no_length() {
        let icon = icon();
        assert_eq!(
            compiled_svg_len(&icon[..icon.len() - 1]),
            None,
            "missing its last op"
        );
        assert_eq!(
            compiled_svg_len(&icon[..icon.len() - 4]),
            None,
            "cut inside an op's points"
        );

        let mut unknown = icon.clone();
        let close = unknown.len() - 1;
        unknown[close] = 0x7f;
        assert_eq!(
            compiled_svg_len(&unknown),
            None,
            "an op the parser does not know"
        );

        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 516\n";
        assert_ne!(
            compiled_svg_len(pointer),
            Some(pointer.len()),
            "a git-lfs pointer"
        );
    }

    #[test]
    fn bytes_past_the_icon_are_not_part_of_it() {
        let mut icon = icon();
        let len = icon.len();
        icon.extend_from_slice(b"trailing");
        assert_eq!(compiled_svg_len(&icon), Some(len));
    }

    #[test]
    fn the_viewbox_is_read_off_the_header() {
        assert_eq!(svg_viewbox(&header(24.0, 18.0)), Some((24.0, 18.0)));
    }

    #[test]
    fn a_header_that_cannot_be_drawn_has_no_viewbox() {
        assert_eq!(svg_viewbox(&header(0.0, 18.0)), None);
        assert_eq!(svg_viewbox(&header(24.0, f32::NAN)), None);
        assert_eq!(svg_viewbox(&header(-1.0, 18.0)), None);
        assert_eq!(svg_viewbox(b"<svg"), None, "too short for a header");
    }
}

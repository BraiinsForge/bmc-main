// Copyright (C) 2026  Braiins Systems s.r.o.

//! SVG binary format constants shared between the proc macro and host runtime.
//!
//! Binary format (emitted by `include_svg!`, parsed by host `SvgRegistry`):
//!
//! ```text
//! [viewbox_w: f32][viewbox_h: f32][path_count: u16]
//!   for each path:
//!     [flags: u8]              // bit 0: has_fill, bit 1: has_stroke, bit 2: even-odd fill
//!     [fill_color: u32]        // RGBA, present if has_fill
//!     [stroke_color: u32]      // RGBA, present if has_stroke
//!     [stroke_width: f32]      // present if has_stroke
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

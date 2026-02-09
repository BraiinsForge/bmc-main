// Copyright (C) 2026  Braiins Systems s.r.o.

//! Icon registry — parses compact binary icon data into FemtoVG paths.
//!
//! Icons are registered once (on first use from WASM) and persist for the
//! runtime lifetime. Each registered icon gets an opaque `u16` ID.

use std::collections::HashMap;
use std::fmt;

use bmc_wasm_protocol::{
    ICON_FLAG_HAS_FILL, ICON_FLAG_HAS_STROKE, ICON_OP_CLOSE, ICON_OP_CUBIC_TO, ICON_OP_LINE_TO,
    ICON_OP_MOVE_TO, ICON_OP_QUAD_TO,
};
use femtovg::{Paint, Path};

use super::text::to_femtovg_color;

/// A single parsed path from an SVG icon.
// Path doesn't impl Debug, so manual impl would be noisy — skip it.
#[expect(missing_debug_implementations)]
pub struct IconPath {
    pub path: Path,
    pub fill_color: Option<u32>,
    pub stroke_color: Option<u32>,
    pub stroke_width: f32,
}

/// A fully parsed icon ready for rendering.
#[expect(missing_debug_implementations)]
pub struct RegisteredIcon {
    pub paths: Vec<IconPath>,
    pub viewbox_w: f32,
    pub viewbox_h: f32,
}

/// Registry mapping opaque IDs to parsed FemtoVG icon data.
pub struct IconRegistry {
    icons: HashMap<u16, RegisteredIcon>,
    next_id: u16,
}

impl fmt::Debug for IconRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IconRegistry")
            .field("count", &self.icons.len())
            .field("next_id", &self.next_id)
            .finish()
    }
}

impl IconRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            icons: HashMap::new(),
            next_id: 1,
        }
    }

    /// Parse binary icon data and register it, returning the assigned ID.
    pub fn register(&mut self, data: &[u8]) -> u16 {
        let id = self.next_id;
        self.next_id += 1;

        match parse_icon(data) {
            Ok(icon) => {
                self.icons.insert(id, icon);
            }
            Err(e) => {
                tracing::error!("failed to parse icon data: {e}");
            }
        }
        id
    }

    /// Parse binary icon data and register it with an explicit ID.
    pub fn register_with_id(&mut self, id: u16, data: &[u8]) {
        match parse_icon(data) {
            Ok(icon) => {
                self.icons.insert(id, icon);
            }
            Err(e) => {
                tracing::error!("failed to parse icon data for id 0x{id:04X}: {e}");
            }
        }
    }

    /// Register all built-in icons from compiled data.
    pub fn register_builtins(&mut self) {
        for &(id, data) in super::builtin_icons::BUILTIN_ICON_DATA {
            self.register_with_id(id, data);
        }
    }

    #[must_use]
    pub fn get(&self, id: u16) -> Option<&RegisteredIcon> {
        self.icons.get(&id)
    }
}

/// Render a registered icon onto the canvas.
///
/// `color == 0` (TRANSPARENT) → use original SVG colors.
/// `color != 0` → tint all fills/strokes with the given color.
pub fn draw_icon(
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    icon: &RegisteredIcon,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    if icon.viewbox_w <= 0.0 || icon.viewbox_h <= 0.0 {
        return;
    }

    let scale_x = w / icon.viewbox_w;
    let scale_y = h / icon.viewbox_h;

    canvas.save();
    canvas.translate(x, y);
    canvas.scale(scale_x, scale_y);

    let tint = if color != 0 {
        Some(to_femtovg_color(color))
    } else {
        None
    };

    for icon_path in &icon.paths {
        if let Some(fill_color) = icon_path.fill_color {
            let paint_color = tint.unwrap_or_else(|| to_femtovg_color(fill_color));
            canvas.fill_path(&icon_path.path, &Paint::color(paint_color));
        }
        if let Some(stroke_color) = icon_path.stroke_color {
            let paint_color = tint.unwrap_or_else(|| to_femtovg_color(stroke_color));
            let mut paint = Paint::color(paint_color);
            paint.set_line_width(icon_path.stroke_width);
            canvas.stroke_path(&icon_path.path, &paint);
        }
    }

    canvas.restore();
}

// ── Binary format parsing ───────────────────────────────────────────

fn parse_icon(data: &[u8]) -> anyhow::Result<RegisteredIcon> {
    let mut r = IconReader { data, pos: 0 };

    let viewbox_w = r.read_f32()?;
    let viewbox_h = r.read_f32()?;
    let path_count = r.read_u16()?;

    let mut paths = Vec::with_capacity(path_count as usize);
    for _ in 0..path_count {
        paths.push(read_icon_path(&mut r)?);
    }

    Ok(RegisteredIcon {
        paths,
        viewbox_w,
        viewbox_h,
    })
}

fn read_icon_path(r: &mut IconReader<'_>) -> anyhow::Result<IconPath> {
    let flags = r.read_u8()?;
    let has_fill = flags & ICON_FLAG_HAS_FILL != 0;
    let has_stroke = flags & ICON_FLAG_HAS_STROKE != 0;

    let fill_color = if has_fill { Some(r.read_u32()?) } else { None };

    let stroke_color;
    let stroke_width;
    if has_stroke {
        stroke_color = Some(r.read_u32()?);
        stroke_width = r.read_f32()?;
    } else {
        stroke_color = None;
        stroke_width = 0.0;
    }

    let op_count = r.read_u16()?;
    let mut path = Path::new();

    for _ in 0..op_count {
        let op = r.read_u8()?;
        match op {
            ICON_OP_MOVE_TO => {
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                path.move_to(x, y);
            }
            ICON_OP_LINE_TO => {
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                path.line_to(x, y);
            }
            ICON_OP_QUAD_TO => {
                let cx = r.read_f32()?;
                let cy = r.read_f32()?;
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                path.quad_to(cx, cy, x, y);
            }
            ICON_OP_CUBIC_TO => {
                let cx1 = r.read_f32()?;
                let cy1 = r.read_f32()?;
                let cx2 = r.read_f32()?;
                let cy2 = r.read_f32()?;
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                path.bezier_to(cx1, cy1, cx2, cy2, x, y);
            }
            ICON_OP_CLOSE => {
                path.close();
            }
            _ => anyhow::bail!("unknown icon path op: 0x{op:02x}"),
        }
    }

    Ok(IconPath {
        path,
        fill_color,
        stroke_color,
        stroke_width,
    })
}

struct IconReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl IconReader<'_> {
    fn read_u8(&mut self) -> anyhow::Result<u8> {
        if self.pos >= self.data.len() {
            anyhow::bail!("unexpected end of icon data");
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u16(&mut self) -> anyhow::Result<u16> {
        if self.pos + 2 > self.data.len() {
            anyhow::bail!("unexpected end of icon data");
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn read_u32(&mut self) -> anyhow::Result<u32> {
        if self.pos + 4 > self.data.len() {
            anyhow::bail!("unexpected end of icon data");
        }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn read_f32(&mut self) -> anyhow::Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }
}

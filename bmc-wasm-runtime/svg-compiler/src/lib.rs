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

//! Shared SVG-to-binary icon compiler.
//!
//! Used by both the `include_svg!` proc macro (compile-time)
//! and the host runtime `build.rs` (for built-in icons).

// ── Svg binary format ──────────────────────────────────────────────
//
// [viewbox_w: f32][viewbox_h: f32][path_count: u16]
//   for each path:
//     [flags: u8]              bit 0: has_fill, bit 1: has_stroke,
//                              bit 2: even-odd fill, bit 3: has_id
//     [fill_color: u32]        RGBA, present if has_fill
//     [stroke_color: u32]      RGBA, present if has_stroke
//     [stroke_width: f32]      present if has_stroke
//     [id_len: u16][id_bytes]  UTF-8, present if has_id
//     [op_count: u16]
//       0x00 MoveTo  [x: f32][y: f32]
//       0x01 LineTo  [x: f32][y: f32]
//       0x02 QuadTo  [cx: f32][cy: f32][x: f32][y: f32]
//       0x03 CubicTo [cx1: f32][cy1: f32][cx2: f32][cy2: f32][x: f32][y: f32]
//       0x04 Close

const OP_MOVE_TO: u8 = 0x00;
const OP_LINE_TO: u8 = 0x01;
const OP_QUAD_TO: u8 = 0x02;
const OP_CUBIC_TO: u8 = 0x03;
const OP_CLOSE: u8 = 0x04;

const FLAG_HAS_FILL: u8 = 0x01;
const FLAG_HAS_STROKE: u8 = 0x02;
const FLAG_EVENODD: u8 = 0x04;
const FLAG_HAS_ID: u8 = 0x08;

/// Compile an SVG string into compact binary path data.
///
/// Panics on parse errors (intended for build-time use).
#[must_use]
pub fn compile_svg(svg: &str) -> Vec<u8> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
        .unwrap_or_else(|e| panic!("SVG parse error: {e}"));

    let size = tree.size();
    let mut paths = Vec::new();
    collect_paths(tree.root(), &mut paths);

    let mut buf = Vec::new();
    buf.extend_from_slice(&size.width().to_le_bytes());
    buf.extend_from_slice(&size.height().to_le_bytes());
    let path_count = u16::try_from(paths.len()).unwrap_or_else(|_| panic!("too many paths in SVG"));
    buf.extend_from_slice(&path_count.to_le_bytes());

    for info in &paths {
        write_path(&mut buf, info);
    }

    buf
}

struct PathInfo {
    fill_color: Option<u32>,
    stroke_color: Option<u32>,
    stroke_width: f32,
    is_evenodd: bool,
    /// `id` attribute from the source SVG path, or empty if absent.
    /// Widgets address paths by id when calling `Draw::svg(...).fill(id, color)`,
    /// so preserving meaningful ids (per the project's svgo config)
    /// feeds the fill-by-id colorize pipeline.
    id: String,
    ops: Vec<PathOp>,
}

enum PathOp {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

fn collect_paths(group: &usvg::Group, out: &mut Vec<PathInfo>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect_paths(g, out),
            usvg::Node::Path(path) => {
                let fill = path.fill();
                let fill_color = fill.and_then(|f| paint_to_rgba(f.paint(), f.opacity().get()));
                let stroke_color = path
                    .stroke()
                    .and_then(|s| paint_to_rgba(s.paint(), s.opacity().get()));
                let stroke_width = path.stroke().map_or(0.0, |s| s.width().get());

                // Skip paths with no visible paint
                if fill_color.is_none() && stroke_color.is_none() {
                    continue;
                }

                let is_evenodd = fill.is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd);

                // Transform path data to root coordinates
                let abs_ts = path.abs_transform();
                let data = path
                    .data()
                    .clone()
                    .transform(abs_ts)
                    .unwrap_or_else(|| path.data().clone());

                let mut ops = Vec::new();
                for seg in data.segments() {
                    use usvg::tiny_skia_path::PathSegment;
                    match seg {
                        PathSegment::MoveTo(p) => ops.push(PathOp::MoveTo(p.x, p.y)),
                        PathSegment::LineTo(p) => ops.push(PathOp::LineTo(p.x, p.y)),
                        PathSegment::QuadTo(c, p) => {
                            ops.push(PathOp::QuadTo(c.x, c.y, p.x, p.y));
                        }
                        PathSegment::CubicTo(c1, c2, p) => {
                            ops.push(PathOp::CubicTo(c1.x, c1.y, c2.x, c2.y, p.x, p.y));
                        }
                        PathSegment::Close => ops.push(PathOp::Close),
                    }
                }

                out.push(PathInfo {
                    fill_color,
                    stroke_color,
                    stroke_width,
                    is_evenodd,
                    id: path.id().to_owned(),
                    ops,
                });
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
}

fn paint_to_rgba(paint: &usvg::Paint, opacity: f32) -> Option<u32> {
    match paint {
        usvg::Paint::Color(c) => {
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let a = (opacity * 255.0) as u8;
            Some(
                (u32::from(c.red) << 24)
                    | (u32::from(c.green) << 16)
                    | (u32::from(c.blue) << 8)
                    | u32::from(a),
            )
        }
        // Gradients and patterns are not supported — skip
        usvg::Paint::LinearGradient(_)
        | usvg::Paint::RadialGradient(_)
        | usvg::Paint::Pattern(_) => None,
    }
}

fn write_path(buf: &mut Vec<u8>, info: &PathInfo) {
    let mut flags = 0_u8;
    if info.fill_color.is_some() {
        flags |= FLAG_HAS_FILL;
    }
    if info.stroke_color.is_some() {
        flags |= FLAG_HAS_STROKE;
    }
    if info.is_evenodd {
        flags |= FLAG_EVENODD;
    }
    if !info.id.is_empty() {
        flags |= FLAG_HAS_ID;
    }
    buf.push(flags);

    if let Some(color) = info.fill_color {
        buf.extend_from_slice(&color.to_le_bytes());
    }
    if let Some(color) = info.stroke_color {
        buf.extend_from_slice(&color.to_le_bytes());
        buf.extend_from_slice(&info.stroke_width.to_le_bytes());
    }
    if !info.id.is_empty() {
        let id_len = u16::try_from(info.id.len())
            .unwrap_or_else(|_| panic!("path id `{}` exceeds u16::MAX bytes", info.id));
        buf.extend_from_slice(&id_len.to_le_bytes());
        buf.extend_from_slice(info.id.as_bytes());
    }

    let op_count =
        u16::try_from(info.ops.len()).unwrap_or_else(|_| panic!("too many ops in SVG path"));
    buf.extend_from_slice(&op_count.to_le_bytes());
    for op in &info.ops {
        match op {
            PathOp::MoveTo(x, y) => {
                buf.push(OP_MOVE_TO);
                buf.extend_from_slice(&x.to_le_bytes());
                buf.extend_from_slice(&y.to_le_bytes());
            }
            PathOp::LineTo(x, y) => {
                buf.push(OP_LINE_TO);
                buf.extend_from_slice(&x.to_le_bytes());
                buf.extend_from_slice(&y.to_le_bytes());
            }
            PathOp::QuadTo(cx, cy, x, y) => {
                buf.push(OP_QUAD_TO);
                buf.extend_from_slice(&cx.to_le_bytes());
                buf.extend_from_slice(&cy.to_le_bytes());
                buf.extend_from_slice(&x.to_le_bytes());
                buf.extend_from_slice(&y.to_le_bytes());
            }
            PathOp::CubicTo(cx1, cy1, cx2, cy2, x, y) => {
                buf.push(OP_CUBIC_TO);
                buf.extend_from_slice(&cx1.to_le_bytes());
                buf.extend_from_slice(&cy1.to_le_bytes());
                buf.extend_from_slice(&cx2.to_le_bytes());
                buf.extend_from_slice(&cy2.to_le_bytes());
                buf.extend_from_slice(&x.to_le_bytes());
                buf.extend_from_slice(&y.to_le_bytes());
            }
            PathOp::Close => buf.push(OP_CLOSE),
        }
    }
}

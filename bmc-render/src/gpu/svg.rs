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

//! Svg registry — parses compact binary icon data into FemtoVG paths.
//!
//! Each registered icon gets an opaque `u16` ID. Dormancy drops parsed paths
//! while preserving the tag and ID reservation for restoration after wake.

use std::collections::HashMap;
use std::fmt;

use bmc_ui::{
    Color, SVG_FLAG_EVENODD, SVG_FLAG_HAS_FILL, SVG_FLAG_HAS_ID, SVG_FLAG_HAS_STROKE, SVG_OP_CLOSE,
    SVG_OP_CUBIC_TO, SVG_OP_LINE_TO, SVG_OP_MOVE_TO, SVG_OP_QUAD_TO, SVG_RESERVED_MIN, SvgId,
};
use femtovg::{FillRule, Paint, Path};

use super::text::to_femtovg_color;
use crate::interaction::Rect;
use crate::renderer::reservations::{RegisterError, TagReservations};
use crate::renderer::{AssetSuspendResult, AssetTagState, StrokeReach, SvgContent};

/// A single parsed path from an SVG icon.
// Path doesn't impl Debug, so manual impl would be noisy — skip it.
#[expect(missing_debug_implementations)]
pub struct IconPath {
    pub path: Path,
    pub fill_color: Option<u32>,
    pub stroke_color: Option<u32>,
    pub stroke_width: f32,
    pub is_evenodd: bool,
}

/// A fully parsed icon ready for rendering.
#[expect(missing_debug_implementations)]
pub struct RegisteredSvg {
    pub paths: Vec<IconPath>,
    /// Map of SVG path `id` attribute → index into `paths`.
    /// Built at registration from paths that ship with a non-empty id,
    /// used at draw time to resolve `Draw::svg(...).fill(id, color)` overrides.
    ///
    /// Paths without an id (most internal `<path>` elements)
    /// don't appear here and are unreachable via per-id fills.
    pub paths_by_id: HashMap<String, usize>,
    pub viewbox_w: f32,
    pub viewbox_h: f32,
    /// `None` when nothing paints, the viewBox is empty or a value is NaN,
    /// which leaves the declared box as the only bound.
    pub content: Option<SvgContent>,
}

/// Registry mapping tag reservations and opaque IDs to parsed FemtoVG icon data.
///
/// Built-in icons sit beside the reservations under fixed IDs
/// from the host-reserved range, which the reservations never issue.
pub struct SvgRegistry {
    reservations: TagReservations<SvgId, RegisteredSvg>,
    builtins: HashMap<SvgId, RegisteredSvg>,
}

impl fmt::Debug for SvgRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SvgRegistry")
            .field("reservations", &self.reservations)
            .field("builtins", &self.builtins.len())
            .finish()
    }
}

fn report_exhausted() {
    tracing::error!(
        "user icon registry exhausted (reserved range starts at 0x{SVG_RESERVED_MIN:04X})",
    );
}

impl SvgRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            reservations: TagReservations::new(SVG_RESERVED_MIN),
            builtins: HashMap::new(),
        }
    }

    pub fn reserve(&mut self, tag: &str) -> Option<SvgId> {
        let id = self.reservations.reserve(tag, ());
        if id.is_none() {
            report_exhausted();
        }
        id
    }

    /// Register binary icon data under `tag`.
    ///
    /// Returns `None` once user-icon allocation reaches `SVG_RESERVED_MIN`,
    /// to avoid colliding with builtin/dev icon IDs.
    pub fn register(&mut self, tag: &str, data: &[u8]) -> Option<SvgId> {
        match self
            .reservations
            .register_with(tag, (), |()| parse_svg(data))
        {
            Ok(id) => Some(id),
            Err(RegisterError::Exhausted) => {
                report_exhausted();
                None
            }
            Err(RegisterError::Load(e)) => {
                tracing::error!("failed to parse icon data ({tag}): {e}");
                None
            }
        }
    }

    /// Parse binary icon data and register it with an explicit ID.
    pub fn register_with_id(&mut self, id: SvgId, data: &[u8]) {
        assert!(
            id.to_wire() >= SVG_RESERVED_MIN,
            "BUG: explicit SVG ID must belong to the host-reserved range"
        );
        match parse_svg(data) {
            Ok(icon) => {
                self.builtins.insert(id, icon);
            }
            Err(e) => {
                tracing::error!(
                    "failed to parse icon data for id 0x{:04X}: {e}",
                    id.to_wire()
                );
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
    pub fn get(&self, id: SvgId) -> Option<&RegisteredSvg> {
        self.reservations
            .payload(id)
            .or_else(|| self.builtins.get(&id))
    }

    #[must_use]
    pub fn tag_state(&self, tag: &str) -> AssetTagState<SvgId> {
        self.reservations.tag_state(tag)
    }

    pub fn suspend_exact(&mut self, tag: &str) -> AssetSuspendResult<SvgId> {
        self.reservations.suspend(tag).result()
    }

    #[must_use]
    pub fn resident_path_bytes(&self) -> u64 {
        self.reservations
            .payloads()
            .chain(self.builtins.values())
            .flat_map(|icon| &icon.paths)
            .map(|path| u64::try_from(path.path.size()).unwrap_or(u64::MAX))
            .sum()
    }

    /// Evict every tag matching `prefix` at segment boundaries.
    /// The tag is either exactly `prefix` or a descendant under it.
    /// Returns the number of tags removed.
    pub fn evict_prefix(&mut self, prefix: &str) -> usize {
        self.reservations.evict_prefix(prefix).len()
    }
}

/// Render a registered icon onto the canvas.
///
/// Override precedence (per path):
/// 1. `fills` override matching the path's `id` wins,
///    recolouring the fill paint only;
/// 2. otherwise `color != TRANSPARENT` tints every fill and stroke
///    with that colour, keeping each path's own alpha (scaled by the tint's),
///    so paths of differing opacity keep the icon's shape;
/// 3. otherwise the path's stored SVG colours are used.
///
/// Whichever wins is then scaled by `brightness`, as [`Color::brightness`] would.
///
/// `fills` is small (typically 1–4 entries per draw) so a linear
/// lookup per path is faster than hashing in practice and keeps
/// the guest from caring about hash collisions.
#[expect(clippy::too_many_arguments)]
pub fn draw_svg(
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    icon: &RegisteredSvg,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
    anti_alias: bool,
    fills: &[(String, Color)],
    brightness: f32,
) {
    if icon.viewbox_w <= 0.0 || icon.viewbox_h <= 0.0 {
        return;
    }

    let scale_x = w / icon.viewbox_w;
    let scale_y = h / icon.viewbox_h;

    canvas.save();
    canvas.translate(x, y);
    canvas.scale(scale_x, scale_y);

    let dim = |c: femtovg::Color| {
        femtovg::Color::rgbaf(c.r * brightness, c.g * brightness, c.b * brightness, c.a)
    };
    let tinted = |tint: femtovg::Color, own: femtovg::Color| {
        femtovg::Color::rgbaf(tint.r, tint.g, tint.b, tint.a * own.a)
    };
    let tint = if color == bmc_ui::TRANSPARENT {
        None
    } else {
        Some(to_femtovg_color(color.to_u32()))
    };

    // Resolve overrides on demand so `fills.is_empty()` (the common
    // hot-path case for hands / centre stack) avoids a per-draw
    // Vec<Option<…>> allocation. `fills` is typically 0–4 entries.
    let override_for = |idx: usize| -> Option<femtovg::Color> {
        fills.iter().find_map(|(id, override_color)| {
            icon.paths_by_id
                .get(id)
                .filter(|&&i| i == idx)
                .map(|_| to_femtovg_color(override_color.to_u32()))
        })
    };

    for (idx, icon_path) in icon.paths.iter().enumerate() {
        let path_override = override_for(idx);
        if let Some(fill_color) = icon_path.fill_color {
            let own = to_femtovg_color(fill_color);
            let paint_color = path_override
                .or_else(|| tint.map(|t| tinted(t, own)))
                .unwrap_or(own);
            let mut paint = Paint::color(dim(paint_color));
            paint.set_anti_alias(anti_alias);
            if icon_path.is_evenodd {
                paint.set_fill_rule(FillRule::EvenOdd);
            }
            canvas.fill_path(&icon_path.path, &paint);
        }
        if let Some(stroke_color) = icon_path.stroke_color {
            let own = to_femtovg_color(stroke_color);
            let paint_color = path_override
                .or_else(|| tint.map(|t| tinted(t, own)))
                .unwrap_or(own);
            let mut paint = Paint::color(dim(paint_color));
            paint.set_anti_alias(anti_alias);
            paint.set_line_width(icon_path.stroke_width);
            canvas.stroke_path(&icon_path.path, &paint);
        }
    }

    canvas.restore();
}

// ── Binary format parsing ───────────────────────────────────────────

fn parse_svg(data: &[u8]) -> anyhow::Result<RegisteredSvg> {
    let mut r = IconReader { data, pos: 0 };

    let viewbox_w = r.read_f32()?;
    let viewbox_h = r.read_f32()?;
    let path_count = r.read_u16()?;

    let mut paths = Vec::with_capacity(path_count as usize);
    let mut paths_by_id = HashMap::new();
    let mut content = ContentBuilder::default();
    for idx in 0..path_count {
        let (path, id) = read_icon_path(&mut r, &mut content)?;
        if let Some(id) = id {
            paths_by_id.insert(id, idx as usize);
        }
        paths.push(path);
    }

    Ok(RegisteredSvg {
        paths,
        paths_by_id,
        viewbox_w,
        viewbox_h,
        content: content.finish(viewbox_w, viewbox_h),
    })
}

/// Bounding box of path points.
#[derive(Clone, Copy, Default)]
enum PointBounds {
    #[default]
    Empty,
    Bounded(Rect),
    /// A NaN point, which `f32::min` would drop rather than bound.
    Poisoned,
}

impl PointBounds {
    fn include(&mut self, x: f32, y: f32) {
        let point = if x.is_nan() || y.is_nan() {
            Self::Poisoned
        } else {
            Self::Bounded(Rect::new(x, y, 0.0, 0.0))
        };
        self.union(point);
    }

    fn union(&mut self, other: PointBounds) {
        *self = match (*self, other) {
            (Self::Poisoned, _) | (_, Self::Poisoned) => Self::Poisoned,
            (Self::Empty, bounds) | (bounds, Self::Empty) => bounds,
            (Self::Bounded(mut rect), Self::Bounded(more)) => {
                rect.union(more);
                Self::Bounded(rect)
            }
        };
    }
}

/// [`SvgContent`] accumulated over the paths that paint.
#[derive(Default)]
struct ContentBuilder {
    points: PointBounds,
    /// Set by a NaN stroke width, which leaves the stroke's reach unbounded.
    poisoned: bool,
    stroke: Option<StrokeReach>,
    fills: bool,
}

impl ContentBuilder {
    /// Grow by one path: its points, and how its paint reaches past them.
    fn add_path(&mut self, points: PointBounds, fill: bool, stroke_width: Option<f32>) {
        if fill || stroke_width.is_some() {
            self.points.union(points);
        }
        self.fills |= fill;
        if let Some(width) = stroke_width {
            self.poisoned |= width.is_nan();
            let reach = StrokeReach {
                half_width: width.abs() / 2.0,
            };
            self.stroke = Some(self.stroke.map_or(reach, |widest| widest.widen(reach)));
        }
    }

    fn finish(self, viewbox_w: f32, viewbox_h: f32) -> Option<SvgContent> {
        let PointBounds::Bounded(bounds) = self.points else {
            return None;
        };
        if self.poisoned || !(viewbox_w > 0.0 && viewbox_h > 0.0) {
            return None;
        }
        Some(SvgContent {
            bounds,
            viewbox_w,
            viewbox_h,
            stroke: self.stroke,
            fills: self.fills,
        })
    }
}

impl StrokeReach {
    fn widen(self, other: Self) -> Self {
        Self {
            half_width: self.half_width.max(other.half_width),
        }
    }
}

/// Parse one path, growing `content` by the region it paints.
fn read_icon_path(
    r: &mut IconReader<'_>,
    content: &mut ContentBuilder,
) -> anyhow::Result<(IconPath, Option<String>)> {
    let flags = r.read_u8()?;
    let has_fill = flags & SVG_FLAG_HAS_FILL != 0;
    let has_stroke = flags & SVG_FLAG_HAS_STROKE != 0;
    let is_evenodd = flags & SVG_FLAG_EVENODD != 0;
    let has_id = flags & SVG_FLAG_HAS_ID != 0;

    let fill_color = if has_fill { Some(r.read_u32()?) } else { None };

    let (stroke_color, stroke_width) = if has_stroke {
        (Some(r.read_u32()?), r.read_f32()?)
    } else {
        (None, 0.0)
    };

    let id = if has_id {
        let id_len = r.read_u16()? as usize;
        let bytes = r.read_bytes(id_len)?;
        Some(
            std::str::from_utf8(bytes)
                .map_err(|e| anyhow::anyhow!("path id is not valid UTF-8: {e}"))?
                .to_owned(),
        )
    } else {
        None
    };

    let (path, points) = read_path_ops(r)?;
    content.add_path(points, has_fill, has_stroke.then_some(stroke_width));

    Ok((
        IconPath {
            path,
            fill_color,
            stroke_color,
            stroke_width,
            is_evenodd,
        },
        id,
    ))
}

/// Parse a path's ops into femtovg's path and the bounds of their points.
///
/// Control points count too: a Bézier never leaves the hull of its own.
fn read_path_ops(r: &mut IconReader<'_>) -> anyhow::Result<(Path, PointBounds)> {
    let op_count = r.read_u16()?;
    let mut path = Path::new();
    let mut points = PointBounds::default();

    for _ in 0..op_count {
        let op = r.read_u8()?;
        match op {
            SVG_OP_MOVE_TO => {
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                points.include(x, y);
                path.move_to(x, y);
            }
            SVG_OP_LINE_TO => {
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                points.include(x, y);
                path.line_to(x, y);
            }
            SVG_OP_QUAD_TO => {
                let cx = r.read_f32()?;
                let cy = r.read_f32()?;
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                points.include(cx, cy);
                points.include(x, y);
                path.quad_to(cx, cy, x, y);
            }
            SVG_OP_CUBIC_TO => {
                let cx1 = r.read_f32()?;
                let cy1 = r.read_f32()?;
                let cx2 = r.read_f32()?;
                let cy2 = r.read_f32()?;
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                points.include(cx1, cy1);
                points.include(cx2, cy2);
                points.include(x, y);
                path.bezier_to(cx1, cy1, cx2, cy2, x, y);
            }
            SVG_OP_CLOSE => {
                path.close();
            }
            _ => anyhow::bail!("unknown icon path op: 0x{op:02x}"),
        }
    }

    Ok((path, points))
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

    fn read_bytes(&mut self, n: usize) -> anyhow::Result<&[u8]> {
        if self.pos + n > self.data.len() {
            anyhow::bail!("unexpected end of icon data");
        }
        let v = &self.data[self.pos..self.pos + n];
        self.pos += n;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::{AssetSuspendResult, AssetTagState};

    /// Smallest valid icon binary: viewbox 100×100, zero paths.
    fn minimal_icon() -> Vec<u8> {
        let mut buf = Vec::with_capacity(10);
        buf.extend_from_slice(&100.0_f32.to_le_bytes());
        buf.extend_from_slice(&100.0_f32.to_le_bytes());
        buf.extend_from_slice(&0_u16.to_le_bytes());
        buf
    }

    fn line_icon() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&100.0_f32.to_le_bytes());
        buf.extend_from_slice(&100.0_f32.to_le_bytes());
        buf.extend_from_slice(&1_u16.to_le_bytes());
        buf.push(0);
        buf.extend_from_slice(&2_u16.to_le_bytes());
        buf.push(SVG_OP_MOVE_TO);
        buf.extend_from_slice(&0.0_f32.to_le_bytes());
        buf.extend_from_slice(&0.0_f32.to_le_bytes());
        buf.push(SVG_OP_LINE_TO);
        buf.extend_from_slice(&100.0_f32.to_le_bytes());
        buf.extend_from_slice(&100.0_f32.to_le_bytes());
        buf
    }

    #[test]
    fn suspend_preserves_tag_and_restores_same_id() {
        let mut reg = SvgRegistry::new();
        let data = line_icon();

        let id = reg
            .register("widget:icon", &data)
            .expect("BUG: register should succeed");
        assert_eq!(reg.tag_state("widget:icon"), AssetTagState::Resident(id));
        let resident_path_bytes = reg.resident_path_bytes();
        assert!(resident_path_bytes > 0);

        assert_eq!(
            reg.suspend_exact("widget:icon"),
            AssetSuspendResult::Suspended(id)
        );
        assert_eq!(reg.tag_state("widget:icon"), AssetTagState::Suspended(id));
        assert_eq!(reg.resident_path_bytes(), 0);
        assert!(reg.get(id).is_none());

        assert_eq!(reg.register("widget:icon", &data), Some(id));
        assert_eq!(reg.tag_state("widget:icon"), AssetTagState::Resident(id));
        assert_eq!(reg.resident_path_bytes(), resident_path_bytes);

        let next_id = reg
            .register("widget:next", &data)
            .expect("BUG: register next icon should succeed");
        assert_eq!(next_id.to_wire(), id.to_wire() + 1);
    }

    #[test]
    fn failed_restore_keeps_svg_reservation() {
        let mut reg = SvgRegistry::new();
        let data = minimal_icon();

        let id = reg
            .register("widget:icon", &data)
            .expect("BUG: register should succeed");
        assert_eq!(
            reg.suspend_exact("widget:icon"),
            AssetSuspendResult::Suspended(id)
        );

        assert_eq!(reg.register("widget:icon", &[]), None);
        assert_eq!(reg.tag_state("widget:icon"), AssetTagState::Suspended(id));

        let next_id = reg
            .register("widget:next", &data)
            .expect("BUG: register next icon should succeed");
        assert_eq!(next_id.to_wire(), id.to_wire() + 1);
    }

    #[test]
    fn exact_reservation_and_suspension_preserve_svg_id() {
        let mut registry = SvgRegistry::new();
        let id = registry
            .reserve("widget:icon")
            .expect("BUG: SVG reservation should succeed");

        assert_eq!(
            registry.tag_state("widget:icon"),
            AssetTagState::Suspended(id)
        );
        assert_eq!(
            registry.suspend_exact("widget:icon"),
            AssetSuspendResult::AlreadySuspended(id)
        );
        assert_eq!(registry.register("widget:icon", &minimal_icon()), Some(id));
        assert_eq!(
            registry.suspend_exact("widget:icon"),
            AssetSuspendResult::Suspended(id)
        );
        assert_eq!(
            registry.suspend_exact("missing"),
            AssetSuspendResult::Unknown
        );
    }

    #[test]
    fn purge_removes_suspended_svg_reservation() {
        let mut reg = SvgRegistry::new();
        let data = minimal_icon();

        let id = reg
            .register("widget:icon", &data)
            .expect("BUG: register should succeed");
        assert_eq!(
            reg.suspend_exact("widget:icon"),
            AssetSuspendResult::Suspended(id)
        );

        assert_eq!(reg.evict_prefix("widget:icon"), 1);
        assert_eq!(reg.tag_state("widget:icon"), AssetTagState::Unknown);

        let new_id = reg
            .register("widget:icon", &data)
            .expect("BUG: re-register should succeed");
        assert_eq!(new_id, id);
    }

    #[test]
    fn recycled_svg_ids_stay_below_the_reserved_range() {
        let mut registry = SvgRegistry::new();
        let reserved_min = 3;
        registry.reservations = TagReservations::new(reserved_min);
        let data = minimal_icon();
        let first = registry
            .register("first", &data)
            .expect("BUG: first registration should succeed");
        let second = registry
            .register("second", &data)
            .expect("BUG: second registration should succeed");

        assert_eq!(registry.register("exhausted", &data), None);
        assert_eq!(registry.evict_prefix("first"), 1);
        assert_eq!(registry.register("replacement", &data), Some(first));
        assert!(first.to_wire() < reserved_min);
        assert!(second.to_wire() < reserved_min);
    }

    #[test]
    fn evict_removes_tag_and_id() {
        let mut reg = SvgRegistry::new();
        let data = minimal_icon();

        let id = reg
            .register("crate::icon", &data)
            .expect("BUG: register should succeed");
        assert!(reg.get(id).is_some());

        assert_eq!(reg.evict_prefix("crate::icon"), 1);
        assert!(reg.get(id).is_none());
        // Idempotent: second evict is a no-op.
        assert_eq!(reg.evict_prefix("crate::icon"), 0);
    }

    #[test]
    fn evict_prefix_only_touches_matching_tags() {
        let mut reg = SvgRegistry::new();
        let data = minimal_icon();

        let id_a1 = reg.register("a:1", &data).expect("BUG: register a:1");
        let id_a2 = reg.register("a:2", &data).expect("BUG: register a:2");
        let id_b1 = reg.register("b:1", &data).expect("BUG: register b:1");

        assert_eq!(reg.evict_prefix("a"), 2);
        assert!(reg.get(id_a1).is_none());
        assert!(reg.get(id_a2).is_none());
        assert!(reg.get(id_b1).is_some());
    }

    #[test]
    fn evict_prefix_respects_segment_boundaries() {
        let mut reg = SvgRegistry::new();
        let data = minimal_icon();

        let id_foo = reg.register("foo", &data).expect("BUG: register foo");
        let id_foobar = reg.register("foobar", &data).expect("BUG: register foobar");
        let id_foo_child = reg
            .register("foo:child", &data)
            .expect("BUG: register foo:child");

        // `foo` matches itself and `foo:child` but not the sibling `foobar`.
        assert_eq!(reg.evict_prefix("foo"), 2);
        assert!(reg.get(id_foo).is_none());
        assert!(reg.get(id_foo_child).is_none());
        assert!(reg.get(id_foobar).is_some());
    }

    #[test]
    fn evict_does_not_touch_register_with_id_entries() {
        let mut reg = SvgRegistry::new();
        let data = minimal_icon();
        let builtin = SvgId::from_wire(SVG_RESERVED_MIN).expect("BUG: reserved id ctor");

        reg.register_with_id(builtin, &data);
        let user = reg
            .register("user::foo", &data)
            .expect("BUG: user register");

        assert!(reg.get(builtin).is_some());
        assert!(reg.get(user).is_some());

        assert_eq!(reg.evict_prefix("user::foo"), 1);
        assert!(reg.get(user).is_none());
        // Built-in icons have no `by_tag` entry, so prefix sweeps don't reach them.
        assert!(reg.get(builtin).is_some());
    }

    #[test]
    #[should_panic(expected = "BUG: explicit SVG ID must belong to the host-reserved range")]
    fn register_with_id_rejects_user_range() {
        let mut reg = SvgRegistry::new();
        let user = SvgId::from_wire(SVG_RESERVED_MIN - 1).expect("BUG: user ID must be nonzero");

        reg.register_with_id(user, &minimal_icon());
    }

    /// An icon of one path through `points`, painted as `paint` asks.
    fn polyline_icon(viewbox: (f32, f32), paint: PathPaint, points: &[(f32, f32)]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&viewbox.0.to_le_bytes());
        buf.extend_from_slice(&viewbox.1.to_le_bytes());
        buf.extend_from_slice(&1_u16.to_le_bytes());
        match paint {
            PathPaint::Nothing => buf.push(0),
            PathPaint::Fill => {
                buf.push(SVG_FLAG_HAS_FILL);
                buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
            }
            PathPaint::Stroke(width) => {
                buf.push(SVG_FLAG_HAS_STROKE);
                buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
                buf.extend_from_slice(&width.to_le_bytes());
            }
        }
        let op_count = u16::try_from(points.len()).expect("BUG: fixture fits a u16 op count");
        buf.extend_from_slice(&op_count.to_le_bytes());
        for (i, (x, y)) in points.iter().enumerate() {
            buf.push(if i == 0 {
                SVG_OP_MOVE_TO
            } else {
                SVG_OP_LINE_TO
            });
            buf.extend_from_slice(&x.to_le_bytes());
            buf.extend_from_slice(&y.to_le_bytes());
        }
        buf
    }

    #[derive(Clone, Copy)]
    enum PathPaint {
        Nothing,
        Fill,
        Stroke(f32),
    }

    fn content(data: &[u8]) -> Option<SvgContent> {
        parse_svg(data).expect("BUG: fixture icon parses").content
    }

    #[test]
    fn a_letterboxed_icon_reports_the_sliver_its_paths_cover() {
        let data = polyline_icon(
            (398.0, 398.0),
            PathPaint::Fill,
            &[(197.0, 0.0), (201.0, 0.0), (201.0, 199.0), (197.0, 199.0)],
        );
        let content = content(&data).expect("BUG: a filled path paints");
        let b = content.bounds;
        assert_eq!((b.x, b.y, b.w, b.h), (197.0, 0.0, 4.0, 199.0));
        assert!(
            content.fills && content.stroke.is_none(),
            "it fills and nothing strokes"
        );
    }

    #[test]
    fn a_stroke_records_its_half_width() {
        let data = polyline_icon(
            (100.0, 100.0),
            PathPaint::Stroke(2.0),
            &[(50.0, 10.0), (50.0, 90.0)],
        );
        let stroke = content(&data)
            .expect("BUG: a stroked path paints")
            .stroke
            .expect("BUG: the fixture strokes");
        assert!(
            (stroke.half_width - 1.0).abs() < f32::EPSILON,
            "got {}",
            stroke.half_width
        );
    }

    #[test]
    fn an_icon_painting_nothing_has_no_content() {
        assert!(content(&minimal_icon()).is_none());
        let unpainted = polyline_icon(
            (100.0, 100.0),
            PathPaint::Nothing,
            &[(0.0, 0.0), (1.0, 1.0)],
        );
        assert!(content(&unpainted).is_none());
    }

    #[test]
    fn a_nan_point_or_stroke_width_drops_the_content() {
        let point = polyline_icon(
            (100.0, 100.0),
            PathPaint::Fill,
            &[(10.0, 10.0), (f32::NAN, 20.0), (20.0, 20.0)],
        );
        assert!(content(&point).is_none());
        let width = polyline_icon(
            (100.0, 100.0),
            PathPaint::Stroke(f32::NAN),
            &[(10.0, 10.0), (20.0, 20.0)],
        );
        assert!(content(&width).is_none());
    }

    #[test]
    fn an_empty_viewbox_has_no_content() {
        let data = polyline_icon((0.0, 100.0), PathPaint::Fill, &[(0.0, 0.0), (0.0, 1.0)]);
        assert!(content(&data).is_none());
    }
}

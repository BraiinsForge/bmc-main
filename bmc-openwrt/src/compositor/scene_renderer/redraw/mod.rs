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

//! What a frame redraws, planned from what changed on the output.

use bmc_platform::{DisplayTransform, FrameRedraw};
use smithay::{
    reexports::wayland_server::backend::ObjectId,
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
};

use super::{CombinedSceneGrid, RenderableItem, RowOrder, texture_damage_rect};
use crate::compositor::output_damage::{OutputDamage, WidgetDamage};

/// What a frame redraws.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Redraw {
    Everything,
    /// Output rectangles that do not overlap.
    Only(Vec<Rectangle<i32, Physical>>),
}

/// What a frame draws apart from the pixels inside widget buffers.
///
/// Two frames with the same layout differ only where their widgets' buffers changed.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameLayout {
    widgets: Vec<LaidOutWidget>,
    grids: Vec<LaidOutGrid>,
}

#[derive(Clone, Debug, PartialEq)]
struct LaidOutWidget {
    instance_id: bmc::compositor::InstanceId,
    dst: Rectangle<i32, Physical>,
    alpha: f32,
    row_order: RowOrder,
}

#[derive(Clone, Debug, PartialEq)]
struct LaidOutGrid {
    x_offset: i32,
    alpha: f32,
}

impl FrameLayout {
    /// The layout of a frame drawing `items` over `grids`.
    ///
    /// The widgets are sorted, as their draw order follows the order their buffers last
    /// committed in, and widgets that do not overlap draw the same pixels in any order.
    pub(super) fn of(items: &[RenderableItem], grids: &[CombinedSceneGrid]) -> Self {
        let mut layout = Self {
            widgets: items
                .iter()
                .map(|item| LaidOutWidget {
                    instance_id: item.instance_id.clone(),
                    dst: item.dst,
                    alpha: item.alpha,
                    row_order: item.row_order,
                })
                .collect(),
            grids: grids
                .iter()
                .map(|grid| LaidOutGrid {
                    x_offset: grid.x_offset,
                    alpha: grid.alpha,
                })
                .collect(),
        };
        layout.widgets.sort_by(|a, b| {
            (&a.instance_id, a.dst.loc.x, a.dst.loc.y).cmp(&(
                &b.instance_id,
                b.dst.loc.x,
                b.dst.loc.y,
            ))
        });
        layout
    }
}

/// A flipped frame as the next one needs it: its layout, and where it differed from the frame before.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LastFrame {
    layout: FrameLayout,
    change: FrameChange,
}

/// Where a frame differed from the frame before it.
#[derive(Clone, Debug, PartialEq)]
enum FrameChange {
    Unknown,
    Known(Vec<Rectangle<i32, Physical>>),
}

/// What a frame draws besides widgets at rest, any of which redraws the whole output.
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one frame, each forcing a whole redraw"
)]
pub(super) struct FrameExtras {
    pub(super) transition: bool,
    pub(super) drag: bool,
    pub(super) layers: bool,
    pub(super) logo: bool,
    pub(super) probe: bool,
}

impl FrameExtras {
    fn any(self) -> bool {
        self.transition || self.drag || self.layers || self.logo || self.probe
    }
}

/// What a frame redraws, and what it leaves the next frame once flipped.
///
/// The back buffer holds the frame before last, so a partial redraw has to cover
/// what both the last frame and this one changed.
/// A frame's change is known only when it shares the last frame's layout:
/// anything else, from a logo that went away to a widget that moved or resized,
/// changes pixels no widget damage describes.
/// A frame with extras leaves the next frame nothing to build on, as does a board
/// that redraws every frame whole.
pub(super) fn plan_frame(
    frame_redraw: FrameRedraw,
    extras: FrameExtras,
    damage: &OutputDamage,
    last: Option<LastFrame>,
    layout: impl FnOnce() -> FrameLayout,
    widget_change: impl FnOnce() -> Vec<Rectangle<i32, Physical>>,
) -> (Redraw, Option<LastFrame>) {
    if frame_redraw == FrameRedraw::Whole || extras.any() {
        return (Redraw::Everything, None);
    }
    let layout = layout();
    let same_layout = last.as_ref().is_some_and(|last| last.layout == layout);
    let change = if same_layout && !damage.is_full() {
        FrameChange::Known(widget_change())
    } else {
        FrameChange::Unknown
    };
    let redraw = match (&change, last.map(|last| last.change)) {
        (FrameChange::Known(change), Some(FrameChange::Known(last_change))) => {
            Redraw::Only(disjoint(change.iter().chain(&last_change)))
        }
        _ => Redraw::Everything,
    };
    (redraw, Some(LastFrame { layout, change }))
}

/// Every overlap of a rectangle in `areas` with one in `within`.
pub(super) fn intersect_all(
    areas: &[Rectangle<i32, Physical>],
    within: &[Rectangle<i32, Physical>],
) -> Vec<Rectangle<i32, Physical>> {
    areas
        .iter()
        .flat_map(|area| within.iter().filter_map(|w| area.intersection(*w)))
        .collect()
}

/// The area `rects` cover, as rectangles that do not overlap,
/// since smithay fills every damage rectangle it is given, overlaps twice.
fn disjoint<'a>(
    rects: impl IntoIterator<Item = &'a Rectangle<i32, Physical>>,
) -> Vec<Rectangle<i32, Physical>> {
    let mut covered: Vec<Rectangle<i32, Physical>> = Vec::new();
    for rect in rects {
        let uncovered =
            Rectangle::subtract_rects_many_in_place(vec![*rect], covered.iter().copied());
        covered.extend(uncovered);
    }
    covered
}

/// The damage of a draw into `dst`, relative to its origin as smithay takes it:
/// the part of `redraw` inside `dst`.
pub(super) fn damage_within(
    dst: Rectangle<i32, Physical>,
    redraw: &Redraw,
) -> Vec<Rectangle<i32, Physical>> {
    match redraw {
        Redraw::Only(redraw) => local_to(dst, &intersect_all(&[dst], redraw)),
        Redraw::Everything => vec![texture_damage_rect(dst)],
    }
}

/// `rects` relative to `dst`'s origin, as smithay takes a draw's damage.
fn local_to(
    dst: Rectangle<i32, Physical>,
    rects: &[Rectangle<i32, Physical>],
) -> Vec<Rectangle<i32, Physical>> {
    rects
        .iter()
        .map(|rect| Rectangle::new(rect.loc - dst.loc, rect.size))
        .collect()
}

/// A widget buffer's damage on the output, through the transform its texture is drawn with.
fn buffer_damage_on_output(
    region: Rectangle<i32, BufferCoord>,
    tex_size: Size<i32, BufferCoord>,
    transform: Transform,
    dst: Rectangle<i32, Physical>,
) -> Option<Rectangle<i32, Physical>> {
    let region = clip_to_buffer(region, tex_size)?;
    // smithay's `to_logical` lands a rect where `render_texture_from_to` draws it
    // under every transform but a plain quarter turn, where it picks the opposite one.
    let mapping = match transform {
        Transform::_90 | Transform::_270 => transform.invert(),
        Transform::Normal
        | Transform::_180
        | Transform::Flipped
        | Transform::Flipped90
        | Transform::Flipped180
        | Transform::Flipped270 => transform,
    };
    let local = region.to_logical(1, mapping, &tex_size);
    Rectangle::new(
        (dst.loc.x + local.loc.x, dst.loc.y + local.loc.y).into(),
        (local.size.w, local.size.h).into(),
    )
    .intersection(dst)
}

/// The part of client damage that lies on its `tex_size` buffer, `None` for none.
///
/// Clipped with saturating arithmetic first: a client may send any coordinates,
/// and the transform's arithmetic would overflow on extreme ones.
fn clip_to_buffer(
    region: Rectangle<i32, BufferCoord>,
    tex_size: Size<i32, BufferCoord>,
) -> Option<Rectangle<i32, BufferCoord>> {
    let left = region.loc.x.clamp(0, tex_size.w);
    let top = region.loc.y.clamp(0, tex_size.h);
    let right = region
        .loc
        .x
        .saturating_add(region.size.w)
        .clamp(left, tex_size.w);
    let bottom = region
        .loc
        .y
        .saturating_add(region.size.h)
        .clamp(top, tex_size.h);
    (right > left && bottom > top)
        .then(|| Rectangle::new((left, top).into(), (right - left, bottom - top).into()))
}

/// Where the committed widgets changed the output, for the widgets drawn this frame,
/// with `texture_size` giving the size of a buffer's imported texture.
pub(super) fn widget_change(
    texture_size: impl Fn(&ObjectId) -> Option<Size<i32, BufferCoord>>,
    scanout_transform: DisplayTransform,
    damage: &OutputDamage,
    items: &[RenderableItem],
) -> Vec<Rectangle<i32, Physical>> {
    let mut change = Vec::new();
    for item in items {
        let (Some(widget_damage), Some(tex_size)) = (
            damage.widget(&item.instance_id),
            texture_size(&item.buffer_id),
        ) else {
            continue;
        };
        match widget_damage {
            WidgetDamage::Whole => change.push(item.dst),
            WidgetDamage::Regions(regions) => {
                let transform = item.row_order.scanout_transform(scanout_transform);
                change.extend(regions.iter().filter_map(|region| {
                    buffer_damage_on_output(*region, tex_size, transform, item.dst)
                }));
            }
        }
    }
    change
}

/// Output pixels the redrawn rectangles cover; they are disjoint, so their areas add up.
#[cfg(feature = "profiling")]
pub(super) fn covered_px(
    redraw: &[Rectangle<i32, Physical>],
    output: Rectangle<i32, Physical>,
) -> u64 {
    redraw
        .iter()
        .filter_map(|rect| rect.intersection(output))
        .map(|rect| {
            u64::try_from(rect.size.w * rect.size.h).expect("BUG: an overlap has no negative area")
        })
        .sum()
}

#[cfg(test)]
mod tests;

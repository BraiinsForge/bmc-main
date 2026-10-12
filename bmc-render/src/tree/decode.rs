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

//! Decoding the guest's wire-format tree into [`TreeNode`]s.

use super::model::ButtonSkinData;
use super::model::DrawCommand;
use super::model::HostAnimationDef;
use super::model::HostTransitionDef;
use super::model::NinePatchData;
use super::model::SliderSkinData;
use super::model::SpanData;
use super::model::TreeNode;
use crate::components::skeleton::SkeletonData;
use crate::components::switcher::SwitcherTabData;
use crate::components::tag::tag_theme;
use crate::gpu::mesh::{MeshDrawArgs, MeshHighlight, MeshLighting, MeshTransform};
use anyhow::{Result, bail};
use bmc_wasm_protocol::{
    AnimProperty, ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, AutoFit, BitmapId, Color,
    ColorSpace, DRAW_ARC, DRAW_AUTOFIT_TEXT, DRAW_BITMAP, DRAW_CENTERED, DRAW_CIRCLE,
    DRAW_CURVED_TEXT, DRAW_ICON, DRAW_MESH, DRAW_MODIFIED, DRAW_NINE_PATCH, DRAW_ORBIT, DRAW_PATH,
    DRAW_QR, DRAW_RECT, DRAW_ROTATED, DRAW_SHADOW, DRAW_SPHERE, DRAW_TEXT, Dash, Easing, Fill,
    LoopMode, MeshId, NODE_BUTTON, NODE_CANVAS, NODE_CENTER, NODE_COLUMN, NODE_MODAL,
    NODE_NOTIFICATION, NODE_PARAGRAPH, NODE_PROGRESS_BAR, NODE_RELTIME, NODE_ROW, NODE_SCROLL,
    NODE_SKELETON, NODE_SPACER, NODE_SWITCHER, NODE_TAG, PathPaint, ProgressKind, PropsData,
    RelTimeClamp, RelTimeFormat, SkeletonKind, SpanFlags, SvgId, TagIconMode, TagKind, TextStyle,
    decode_arc_cap, decode_arc_fill, decode_arc_segments, decode_fill,
};
use std::ops::Range;

/// Every node opens with its one-byte `NODE_*` type.
const NODE_TYPE_LEN: usize = 1;

/// Reader for deserializing tree from bytes
pub(super) struct TreeReader<'a> {
    data: &'a [u8],
    pos: usize,
    /// Byte ranges outside the static half, in wire order; see
    /// [`crate::partition::static_bytes_key`].
    dynamic_ranges: Vec<Range<usize>>,
}

impl<'a> TreeReader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            dynamic_ranges: Vec::new(),
        }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Result<u8> {
        if self.pos >= self.data.len() {
            bail!("unexpected end of tree data");
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u16(&mut self) -> Result<u16> {
        if self.pos + 2 > self.data.len() {
            bail!("unexpected end of tree data");
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    pub(super) fn read_u32(&mut self) -> Result<u32> {
        if self.pos + 4 > self.data.len() {
            bail!("unexpected end of tree data");
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

    /// Deliberately does not reject NaN: the protocol spends it as a sentinel —
    /// `PropsData::inset_*` for "auto", `Sphere`'s `light_lat`/`light_lon` for
    /// "unlit" — so refusing it here rejects valid trees. A NaN that reaches a
    /// rotation range is caught in `damage`, which fails open to the whole
    /// canvas rather than inverting its sweep to nothing.
    fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    /// A finite, non-negative `f32`, for a width, height or radius.
    ///
    /// femtovg renders a negative extent as mirrored geometry while `damage`
    /// computes `right < x` and reports nothing, so the old pixels are never
    /// restored under target preservation.
    fn read_extent(&mut self) -> Result<f32> {
        let value = self.read_f32()?;
        // Finite and non-negative both, since an extent has no sentinel
        // reading the way the coordinates above do: NaN and either infinity
        // are as meaningless here as a negative width.
        anyhow::ensure!(
            value.is_finite() && value >= 0.0,
            "invalid extent {value} in tree at {}",
            self.pos
        );
        Ok(value)
    }

    fn read_i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_bytes(8)?.try_into()?))
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            bail!("unexpected end of tree data");
        }
        let v = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(v)
    }

    fn read_fill(&mut self) -> Result<Fill> {
        let start = self.pos;
        decode_fill(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("invalid or truncated fill in tree at {start}"))
    }

    fn read_arc_fill(&mut self) -> Result<ArcFill> {
        decode_arc_fill(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("unexpected end of tree data reading arc fill"))
    }

    fn read_arc_segments(&mut self) -> Result<ArcSegments> {
        let start = self.pos;
        decode_arc_segments(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("invalid or truncated arc segments in tree at {start}"))
    }

    fn read_arc_cap(&mut self) -> Result<ArcCap> {
        decode_arc_cap(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("unexpected end of tree data reading arc cap"))
    }

    /// Decode an `Option<SvgId>`. Wire zero lifts to `None`.
    fn read_icon_id(&mut self) -> Result<Option<SvgId>> {
        Ok(SvgId::from_wire(self.read_u16()?))
    }

    /// Decode an `Option<BitmapId>`. Wire zero lifts to `None`.
    fn read_bitmap_id(&mut self) -> Result<Option<BitmapId>> {
        Ok(BitmapId::from_wire(self.read_u16()?))
    }

    /// Decode an `Option<MeshId>`. Wire zero lifts to `None`.
    fn read_mesh_id(&mut self) -> Result<Option<MeshId>> {
        Ok(MeshId::from_wire(self.read_u16()?))
    }

    fn read_nine_patch_data(&mut self) -> Result<NinePatchData> {
        Ok(NinePatchData {
            bitmap_id: self.read_bitmap_id()?,
            left: self.read_u16()?,
            top: self.read_u16()?,
            right: self.read_u16()?,
            bottom: self.read_u16()?,
        })
    }

    pub(super) fn read_props(&mut self) -> Result<PropsData> {
        let Some(props) = PropsData::from_bytes(&self.data[self.pos..]) else {
            bail!("invalid or truncated props in tree at {}", self.pos);
        };
        self.pos += PropsData::SIZE;
        Ok(props)
    }

    fn read_string(&mut self, len: u16) -> Result<String> {
        let len = len as usize;
        if self.pos + len > self.data.len() {
            bail!("unexpected end of tree data reading string");
        }
        let s = String::from_utf8(self.data[self.pos..self.pos + len].to_vec())?;
        self.pos += len;
        Ok(s)
    }

    fn read_text_style(&mut self) -> Result<TextStyle> {
        let Some(style) = TextStyle::from_bytes(&self.data[self.pos..]) else {
            bail!("invalid or truncated text style in tree at {}", self.pos);
        };
        self.pos += TextStyle::SIZE;
        Ok(style)
    }

    pub(super) fn read_span(&mut self) -> Result<SpanData> {
        let flags = SpanFlags::unpack(self.read_u16()?, self.read_u8()?);
        let len = self.read_u16()?;
        let text = self.read_string(len)?;

        let color = flags
            .has_color
            .then(|| self.read_u32().map(Color::from_raw))
            .transpose()?;
        let size = flags.has_size.then(|| self.read_u32()).transpose()?;

        Ok(SpanData {
            text,
            weight: flags.weight,
            color,
            size,
            italic: flags.italic,
            underline: flags.underline,
            strikethrough: flags.strikethrough,
        })
    }

    pub(super) fn read_node(&mut self) -> Result<TreeNode> {
        let start = self.pos;
        let ranges_before = self.dynamic_ranges.len();
        let node = self.read_node_body()?;
        if crate::partition::node_self_is_dynamic(&node) {
            // A modal's body is dynamic along with it,
            // so its one range replaces any recorded inside.
            self.dynamic_ranges.truncate(ranges_before);
            // The type byte stays keyed: a progress bar takes height where a modal takes none,
            // so swapping one for the other moves the static siblings.
            self.dynamic_ranges.push(start + NODE_TYPE_LEN..self.pos);
        }
        Ok(node)
    }

    #[expect(clippy::too_many_lines)]
    fn read_node_body(&mut self) -> Result<TreeNode> {
        let node_type = self.read_u8()?;

        match node_type {
            NODE_COLUMN | NODE_ROW | NODE_CENTER => {
                let props = self.read_props()?;
                let child_count = self.read_u16()?;
                let mut children = Vec::with_capacity(child_count as usize);
                for _ in 0..child_count {
                    children.push(self.read_node()?);
                }
                Ok(match node_type {
                    NODE_COLUMN => TreeNode::Column(props, children),
                    NODE_ROW => TreeNode::Row(props, children),
                    NODE_CENTER => TreeNode::Center(props, children),
                    _ => unreachable!(),
                })
            }
            NODE_PARAGRAPH => {
                let props = self.read_props()?;
                let base_style = self.read_text_style()?;
                let span_count = self.read_u16()?;
                let mut spans = Vec::with_capacity(span_count as usize);
                for _ in 0..span_count {
                    spans.push(self.read_span()?);
                }
                Ok(TreeNode::Paragraph {
                    props,
                    base_style,
                    spans,
                })
            }
            NODE_BUTTON => {
                let id_len = self.read_u16()?;
                let id = self.read_string(id_len)?;
                let style = self.read_u8()?;
                let size = self.read_u8()?;
                let icon_id = self.read_icon_id()?;
                let disabled = self.read_u8()? != 0;
                let stretch = self.read_u8()? != 0;
                let len = self.read_u16()?;
                let label = self.read_string(len)?;
                // Trailing optional skin payload
                let skin = if self.remaining() > 0 && self.read_u8()? != 0 {
                    let normal = self.read_nine_patch_data()?;
                    let pressed = if self.read_u8()? != 0 {
                        Some(self.read_nine_patch_data()?)
                    } else {
                        None
                    };
                    let text_color = Color::from_raw(self.read_u32()?);
                    let pressed_text_color = Color::from_raw(self.read_u32()?);
                    let opaque = self.read_u8()? != 0;
                    Some(ButtonSkinData {
                        normal,
                        pressed,
                        text_color,
                        pressed_text_color,
                        opaque,
                    })
                } else {
                    None
                };
                Ok(TreeNode::Button {
                    id,
                    label,
                    style,
                    size,
                    icon_id,
                    disabled,
                    stretch,
                    skin,
                })
            }
            NODE_SPACER => {
                let flex = self.read_extent()?;
                Ok(TreeNode::Spacer { flex })
            }
            NODE_CANVAS => {
                let props = self.read_props()?;
                let key_len = self.read_u16()?;
                let touch_key = if key_len > 0 {
                    Some(self.read_string(key_len)?)
                } else {
                    None
                };
                // The count includes dynamic draws,
                // which may come and go under an unchanged static half.
                let count_start = self.pos;
                let draw_count = self.read_u16()?;
                self.dynamic_ranges.push(count_start..self.pos);
                let mut draws = Vec::with_capacity(draw_count as usize);
                let mut first_dynamic = None;
                for _ in 0..draw_count {
                    let start = self.pos;
                    let draw = self.read_draw()?;
                    if first_dynamic.is_none() && crate::partition::draw_is_dynamic(&draw) {
                        first_dynamic = Some(start);
                    }
                    draws.push(draw);
                }
                // Every draw from the first dynamic one on is `Band::Dynamic` or `Band::Above`.
                if let Some(start) = first_dynamic {
                    self.dynamic_ranges.push(start..self.pos);
                }
                Ok(TreeNode::Canvas {
                    props,
                    touch_key,
                    draws,
                })
            }
            NODE_MODAL => {
                let id_len = self.read_u16()?;
                let modal_id = self.read_string(id_len)?;
                let is_open = self.read_u8()? != 0;
                let padding = self.read_u16()?;
                let backdrop_alpha = self.read_u8()?;
                let title_len = self.read_u16()?;
                let title = self.read_string(title_len)?;
                let content_height = self.read_f32()?;
                let child_count = self.read_u16()?;
                let bg_color = Color::from_raw(self.read_u32()?);
                let header_color = Color::from_raw(self.read_u32()?);
                let title_color = Color::from_raw(self.read_u32()?);
                let max_width = self.read_u16()?;
                let mut body = Vec::with_capacity(child_count as usize);
                for _ in 0..child_count {
                    body.push(self.read_node()?);
                }
                // Footer descriptor
                let pk_len = self.read_u16()?;
                let footer_primary_key = self.read_string(pk_len)?;
                let pl_len = self.read_u16()?;
                let footer_primary_label = self.read_string(pl_len)?;
                let sk_len = self.read_u16()?;
                let footer_secondary_key = self.read_string(sk_len)?;
                let sl_len = self.read_u16()?;
                let footer_secondary_label = self.read_string(sl_len)?;
                let footer_danger = self.read_u8()? != 0;
                Ok(TreeNode::Modal {
                    modal_id,
                    is_open,
                    padding,
                    backdrop_alpha,
                    title,
                    content_height,
                    bg_color,
                    header_color,
                    title_color,
                    max_width,
                    body,
                    footer_primary_key,
                    footer_primary_label,
                    footer_secondary_key,
                    footer_secondary_label,
                    footer_danger,
                })
            }
            NODE_SCROLL => {
                let key_len = self.read_u16()?;
                let scroll_key = self.read_string(key_len)?;
                let props = self.read_props()?;
                let child_count = self.read_u16()?;
                let mut children = Vec::with_capacity(child_count as usize);
                for _ in 0..child_count {
                    children.push(self.read_node()?);
                }
                Ok(TreeNode::Scroll {
                    scroll_key,
                    props,
                    children,
                })
            }
            NODE_NOTIFICATION => {
                let kind = self.read_u8()?;
                let title_len = self.read_u16()?;
                let title = self.read_string(title_len)?;
                let subtitle_len = self.read_u16()?;
                let subtitle = self.read_string(subtitle_len)?;
                Ok(TreeNode::Notification {
                    kind,
                    title,
                    subtitle,
                })
            }
            NODE_RELTIME => {
                let anchor = self.read_i64()?;
                let format = RelTimeFormat::try_from(self.read_u8()?)?;
                let clamp = RelTimeClamp::try_from(self.read_u8()?)?;
                let style = self.read_text_style()?;
                Ok(TreeNode::RelTime {
                    anchor,
                    format,
                    clamp,
                    style,
                })
            }
            NODE_TAG => {
                let kind = TagKind::try_from(self.read_u8()?)?;
                let icon_mode = TagIconMode::try_from(self.read_u8()?)?;
                let custom = self.read_icon_id()?;
                let icon = match icon_mode {
                    TagIconMode::Hidden => None,
                    TagIconMode::Custom => custom,
                    TagIconMode::Default => Some(tag_theme(kind).icon),
                };
                let content = Box::new(self.read_node()?);
                Ok(TreeNode::Tag {
                    kind,
                    icon,
                    content,
                })
            }
            NODE_SWITCHER => {
                let active = self.read_u8()? as usize;
                let disabled = self.read_u8()? != 0;
                let tab_count = self.read_u8()?;
                let mut tabs = Vec::with_capacity(tab_count as usize);
                for _ in 0..tab_count {
                    let icon = self.read_icon_id()?;
                    let id_len = self.read_u16()?;
                    let click_id = self.read_string(id_len)?;
                    tabs.push(SwitcherTabData { icon, click_id });
                }
                Ok(TreeNode::Switcher {
                    active,
                    disabled,
                    tabs,
                })
            }
            NODE_SKELETON => Ok(TreeNode::Skeleton(SkeletonData {
                kind: SkeletonKind::try_from(self.read_u8()?)?,
                chars: self.read_f32()?,
                font_size: self.read_extent()?,
                width: self.read_extent()?,
                height: self.read_extent()?,
                color: Color::from_raw(self.read_u32()?),
            })),
            NODE_PROGRESS_BAR => {
                let key_len = self.read_u16()?;
                let touch_key = if key_len > 0 {
                    Some(self.read_string(key_len)?)
                } else {
                    None
                };
                let track_h = self.read_extent()?;
                let mode = ProgressKind::try_from(self.read_u8()?)?;
                let fraction = self.read_f32()?;
                let active = self.read_u8()? != 0;
                let fill_color = Color::from_raw(self.read_u32()?);
                let track_color = Color::from_raw(self.read_u32()?);
                let bg_color = Color::from_raw(self.read_u32()?);
                let skin = if self.read_u8()? != 0 {
                    Some(SliderSkinData {
                        track: NinePatchData {
                            bitmap_id: self.read_bitmap_id()?,
                            left: self.read_u16()?,
                            top: self.read_u16()?,
                            right: self.read_u16()?,
                            bottom: self.read_u16()?,
                        },
                        track_h: self.read_u16()?,
                        thumb_id: self.read_bitmap_id()?,
                        thumb_w: self.read_u16()?,
                        thumb_h: self.read_u16()?,
                        thumb_pressed_id: self.read_bitmap_id()?,
                    })
                } else {
                    None
                };
                Ok(TreeNode::ProgressBar {
                    touch_key,
                    track_h,
                    mode,
                    fraction,
                    active,
                    fill_color,
                    track_color,
                    bg_color,
                    skin,
                })
            }
            _ => bail!("unknown node type: {node_type}"),
        }
    }

    #[expect(clippy::too_many_lines)]
    pub(super) fn read_draw(&mut self) -> Result<DrawCommand> {
        let draw_type = self.read_u8()?;
        match draw_type {
            DRAW_RECT => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_extent()?;
                let h = self.read_extent()?;
                let fill = self.read_fill()?;
                Ok(DrawCommand::Rect { x, y, w, h, fill })
            }
            DRAW_CIRCLE => {
                let cx = self.read_f32()?;
                let cy = self.read_f32()?;
                let r = self.read_extent()?;
                let fill = self.read_fill()?;
                Ok(DrawCommand::Circle { cx, cy, r, fill })
            }
            DRAW_ARC => {
                let cx = self.read_f32()?;
                let cy = self.read_f32()?;
                let radius = self.read_extent()?;
                let start_angle = self.read_f32()?;
                let end_angle = self.read_f32()?;
                let width = self.read_extent()?;
                let fill = self.read_arc_fill()?;
                let segments = self.read_arc_segments()?;
                let cap = self.read_arc_cap()?;
                Ok(DrawCommand::Arc {
                    cx,
                    cy,
                    radius,
                    start_angle,
                    end_angle,
                    width,
                    fill,
                    segments,
                    cap,
                })
            }
            DRAW_ICON => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_extent()?;
                let h = self.read_extent()?;
                let color = Color::from_raw(self.read_u32()?);
                let icon_id = self.read_icon_id()?;
                let anti_alias = self.read_u8()? != 0;
                let fill_count = self.read_u16()? as usize;
                let mut fills = Vec::with_capacity(fill_count);
                for _ in 0..fill_count {
                    let id_len = self.read_u16()? as usize;
                    let id_bytes = self.read_bytes(id_len)?;
                    let id = std::str::from_utf8(id_bytes)
                        .map_err(|e| anyhow::anyhow!("DRAW_ICON fill id is not valid UTF-8: {e}"))?
                        .to_owned();
                    let color = Color::from_raw(self.read_u32()?);
                    fills.push((id, color));
                }
                Ok(DrawCommand::Svg {
                    x,
                    y,
                    w,
                    h,
                    color,
                    icon_id,
                    anti_alias,
                    fills,
                })
            }
            DRAW_BITMAP => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_extent()?;
                let h = self.read_extent()?;
                let bitmap_id = self.read_bitmap_id()?;
                Ok(DrawCommand::Bitmap {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                })
            }
            DRAW_QR => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let size = self.read_f32()?;
                let dark = Color::from_raw(self.read_u32()?);
                let light = Color::from_raw(self.read_u32()?);
                let quiet_zone = self.read_u8()?;
                let text_len = self.read_u16()? as usize;
                let text = std::str::from_utf8(self.read_bytes(text_len)?)
                    .map_err(|e| anyhow::anyhow!("DRAW_QR text is not valid UTF-8: {e}"))?
                    .to_owned();
                Ok(DrawCommand::Qr {
                    x,
                    y,
                    size,
                    dark,
                    light,
                    quiet_zone,
                    text,
                })
            }
            DRAW_CENTERED => {
                let inner = self.read_draw()?;
                Ok(DrawCommand::Centered {
                    inner: Box::new(inner),
                })
            }
            DRAW_ORBIT => {
                let radius = self.read_extent()?;
                let angle = self.read_f32()?;
                let inner = self.read_draw()?;
                Ok(DrawCommand::Orbit {
                    radius,
                    angle,
                    inner: Box::new(inner),
                })
            }
            DRAW_ROTATED => {
                let angle = self.read_f32()?;
                let inner = self.read_draw()?;
                Ok(DrawCommand::Rotated {
                    angle,
                    inner: Box::new(inner),
                })
            }
            DRAW_SHADOW => {
                let dx = self.read_f32()?;
                let dy = self.read_f32()?;
                let blur = self.read_f32()?;
                let color = Color::from_raw(self.read_u32()?);
                let inner = self.read_draw()?;
                Ok(DrawCommand::Shadow {
                    dx,
                    dy,
                    blur,
                    color,
                    inner: Box::new(inner),
                })
            }
            DRAW_MODIFIED => {
                let flags = self.read_u8()?;
                let has_animations = flags & 0x01 != 0;
                let has_transition = flags & 0x02 != 0;
                let color_space =
                    ColorSpace::from_u8((flags >> 2) & 0x03).unwrap_or(ColorSpace::Oklab);

                let animations = if has_animations {
                    let count = self.read_u8()? as usize;
                    let mut anims = Vec::with_capacity(count);
                    for _ in 0..count {
                        let property = AnimProperty::from_u8(self.read_u8()?)
                            .ok_or_else(|| anyhow::anyhow!("invalid AnimProperty"))?;
                        let from = self.read_f32()?;
                        let to = self.read_f32()?;
                        let duration_ms = self.read_u32()?;
                        let delay_ms = self.read_u16()?;
                        let easing = Easing::from_u8(self.read_u8()?)
                            .ok_or_else(|| anyhow::anyhow!("invalid Easing"))?;
                        let loop_mode = LoopMode::from_u8(self.read_u8()?)
                            .ok_or_else(|| anyhow::anyhow!("invalid LoopMode"))?;
                        anims.push(HostAnimationDef {
                            property,
                            from,
                            to,
                            duration_ms,
                            delay_ms,
                            easing,
                            loop_mode,
                        });
                    }
                    anims
                } else {
                    Vec::new()
                };

                let transition = if has_transition {
                    let id_hash = self.read_u32()?;
                    let duration_ms = self.read_u32()?;
                    let easing = Easing::from_u8(self.read_u8()?)
                        .ok_or_else(|| anyhow::anyhow!("invalid Easing for transition"))?;
                    Some(HostTransitionDef {
                        id_hash,
                        duration_ms,
                        easing,
                    })
                } else {
                    None
                };

                let inner = self.read_draw()?;
                Ok(DrawCommand::Modified {
                    animations,
                    transition,
                    color_space,
                    inner: Box::new(inner),
                })
            }
            DRAW_PATH => {
                let flags = self.read_u8()?;
                let closed = flags & 0x01 != 0;
                let smooth = flags & 0x02 != 0;
                let fill = flags & 0x04 != 0;
                let dashed = flags & 0x08 != 0;
                let point_count = self.read_u16()? as usize;
                let mut points = Vec::with_capacity(point_count);
                for _ in 0..point_count {
                    let x = self.read_f32()?;
                    let y = self.read_f32()?;
                    points.push((x, y));
                }
                let paint = if fill {
                    PathPaint::Fill(self.read_fill()?)
                } else {
                    let color = Color::from_raw(self.read_u32()?);
                    let width = self.read_extent()?;
                    let dash = if dashed {
                        // Both figures are read either way, so a rejected pattern
                        // doesn't desync the stream — it just draws solid.
                        Dash::new(self.read_f32()?, self.read_f32()?)
                    } else {
                        None
                    };
                    PathPaint::Stroke { color, width, dash }
                };
                Ok(DrawCommand::Path {
                    points,
                    paint,
                    closed,
                    smooth,
                })
            }
            DRAW_SPHERE => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_extent()?;
                let h = self.read_extent()?;
                let bitmap_id = self.read_bitmap_id()?;

                let flags = self.read_u8()?;
                let atmosphere = flags & 0x01 != 0;
                let center_lat = self.read_f32()?;
                let center_lon = self.read_f32()?;
                let zoom = self.read_f32()?;
                let light_lat = self.read_f32()?;
                let light_lon = self.read_f32()?;
                Ok(DrawCommand::Sphere {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                    atmosphere,
                    center_lat,
                    center_lon,
                    zoom,
                    light_lat,
                    light_lon,
                })
            }
            DRAW_MESH => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_extent()?;
                let h = self.read_extent()?;
                let mesh_id = self.read_mesh_id()?;

                let fov = self.read_f32()?;
                let distance = self.read_f32()?;
                let qx = self.read_f32()?;
                let qy = self.read_f32()?;
                let qz = self.read_f32()?;
                let qw = self.read_f32()?;
                let px = self.read_f32()?;
                let py = self.read_f32()?;
                let pz = self.read_f32()?;
                let scale = self.read_f32()?;
                let light_pitch = self.read_f32()?;
                let light_yaw = self.read_f32()?;
                let ambient = self.read_f32()?;
                let specular = self.read_f32()?;
                let hl_u_min = self.read_f32()?;
                let hl_v_min = self.read_f32()?;
                let hl_u_max = self.read_f32()?;
                let hl_v_max = self.read_f32()?;
                let hl_r = self.read_f32()?;
                let hl_g = self.read_f32()?;
                let hl_b = self.read_f32()?;
                Ok(DrawCommand::Mesh {
                    x,
                    y,
                    w,
                    h,
                    mesh_id,
                    args: MeshDrawArgs {
                        transform: MeshTransform {
                            fov,
                            distance,
                            quat: [qx, qy, qz, qw],
                            position: [px, py, pz],
                            scale,
                        },
                        lighting: MeshLighting {
                            pitch: light_pitch,
                            yaw: light_yaw,
                            ambient,
                            specular,
                        },
                        highlight: MeshHighlight {
                            u_min: hl_u_min,
                            v_min: hl_v_min,
                            u_max: hl_u_max,
                            v_max: hl_v_max,
                            r: hl_r,
                            g: hl_g,
                            b: hl_b,
                        },
                    },
                })
            }
            DRAW_TEXT => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let style = self.read_text_style()?;
                let len = self.read_u16()?;
                let text = self.read_string(len)?;
                Ok(DrawCommand::Text { x, y, text, style })
            }
            DRAW_CURVED_TEXT => {
                let cx = self.read_f32()?;
                let cy = self.read_f32()?;
                let radius = self.read_extent()?;
                let angle = self.read_f32()?;
                let anchor_raw = self.read_u8()?;
                let anchor = ArcAnchor::try_from(anchor_raw)?;
                let facing_raw = self.read_u8()?;
                let facing = ArcTextFacing::try_from(facing_raw)?;
                let style = self.read_text_style()?;
                let len = self.read_u16()?;
                let text = self.read_string(len)?;
                Ok(DrawCommand::CurvedText {
                    cx,
                    cy,
                    radius,
                    angle,
                    anchor,
                    facing,
                    text,
                    style,
                })
            }
            DRAW_NINE_PATCH => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_extent()?;
                let h = self.read_extent()?;
                let bitmap_id = self.read_bitmap_id()?;

                let left = self.read_u16()?;
                let top = self.read_u16()?;
                let right = self.read_u16()?;
                let bottom = self.read_u16()?;
                Ok(DrawCommand::NinePatch {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                    left,
                    top,
                    right,
                    bottom,
                })
            }
            DRAW_AUTOFIT_TEXT => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let box_width = self.read_extent()?;
                let box_height = self.read_extent()?;
                let mode = AutoFit::from_u8(self.read_u8()?);
                let min_size = self.read_u16()?;
                let max_size = self.read_u16()?;
                let style = self.read_text_style()?;
                let len = self.read_u16()?;
                let text = self.read_string(len)?;
                Ok(DrawCommand::AutofitText {
                    x,
                    y,
                    box_width,
                    box_height,
                    mode,
                    min_size,
                    max_size,
                    text,
                    style,
                })
            }
            _ => bail!("unknown draw command: {draw_type}"),
        }
    }
}

/// Deserialize a tree from bytes
pub fn deserialize_tree(data: &[u8]) -> Result<TreeNode> {
    read_tree(data).map(|(node, _)| node)
}

/// Deserialize a tree from bytes,
/// along with the [`crate::partition::static_bytes_key`] of its static half.
pub fn deserialize_tree_keyed(data: &[u8]) -> Result<(TreeNode, u64)> {
    let (node, reader) = read_tree(data)?;
    let key = crate::partition::static_bytes_key(&data[..reader.pos], &reader.dynamic_ranges);
    Ok((node, key))
}

fn read_tree(data: &[u8]) -> Result<(TreeNode, TreeReader<'_>)> {
    if data.is_empty() {
        bail!("empty tree data");
    }
    let mut reader = TreeReader::new(data);
    let node = reader.read_node()?;
    if reader.remaining() > 0 {
        tracing::warn!("tree has {} trailing bytes", reader.remaining());
    }
    Ok((node, reader))
}

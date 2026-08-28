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

//! Tree deserialization and layout computation.

#![expect(clippy::cast_precision_loss, clippy::cast_lossless)]
#![allow(clippy::wildcard_imports)]

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use bmc_wasm_protocol::*;
use bmc_wasm_protocol::{
    ArcCap, ArcFill, ArcSegments, DRAW_ARC, decode_arc_cap, decode_arc_fill, decode_arc_segments,
};

use crate::gpu::mesh::{MeshDrawArgs, MeshHighlight, MeshLighting, MeshTransform};
use crate::interaction::Rect;

/// When `DEBUG_LAYOUT=1` env var is set, draw colored outlines around every layout node.
static DEBUG_LAYOUT: AtomicBool = AtomicBool::new(false);

/// When `BMC_GPU_PASS_TIMING=1` is set, block on the GPU at each render-pass
/// boundary so a frame's GPU cost can be attributed per pass.
///
/// Not free to observe: the fences serialise passes that normally pipeline, so
/// read the split as proportions rather than as absolute times.
static GPU_PASS_TIMING: AtomicBool = AtomicBool::new(false);

/// Call once at startup to check the `DEBUG_LAYOUT` env var.
pub fn init_debug_flags() {
    if std::env::var("DEBUG_LAYOUT").is_ok_and(|v| v == "1") {
        DEBUG_LAYOUT.store(true, Ordering::Relaxed);
    }
    if std::env::var("BMC_GPU_PASS_TIMING").is_ok_and(|v| v == "1") {
        GPU_PASS_TIMING.store(true, Ordering::Relaxed);
    }
}

/// Whether per-pass GPU fences are enabled; see [`GPU_PASS_TIMING`].
#[must_use]
pub fn gpu_pass_timing_enabled() -> bool {
    GPU_PASS_TIMING.load(Ordering::Relaxed)
}

/// Returns whether debug layout outlines are enabled.
pub fn debug_layout_enabled() -> bool {
    DEBUG_LAYOUT.load(Ordering::Relaxed)
}

/// Toggle debug layout outlines on/off.
pub fn toggle_debug_layout() {
    let prev = DEBUG_LAYOUT.load(Ordering::Relaxed);
    DEBUG_LAYOUT.store(!prev, Ordering::Relaxed);
}

/// Hot-pink-ish colors cycling by depth for debug outlines.
const DEBUG_COLORS: [Color; 6] = [
    Color::from_hex(0xFF_00_FF), // magenta
    Color::from_hex(0x00_FF_FF), // cyan
    Color::from_hex(0xFF_FF_00), // yellow
    Color::from_hex(0xFF_00_00), // red
    Color::from_hex(0x00_FF_00), // green
    Color::from_hex(0xFF_80_00), // orange
];

// Re-export for other modules
pub use crate::components::notification::{
    measure_notification_banner, render_notification_banner,
};
pub use bmc_wasm_protocol::{
    AutoFit, CrossAlign, FontFamily, FontWeight, Justify, PropsData, TextAlign, TextOverflow,
    TextStyle, VerticalAlign,
};

/// A text span with style overrides
#[derive(Clone, Debug)]
pub struct SpanData {
    pub text: String,
    pub weight: Option<FontWeight>,
    pub color: Option<Color>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl SpanData {
    /// Resolve this span's effective style given a base style
    #[must_use]
    pub fn resolve_style(&self, base: &TextStyle) -> TextStyle {
        TextStyle {
            weight: self.weight.unwrap_or(base.weight),
            color: self.color.unwrap_or(base.color),
            italic: self.italic || base.italic,
            underline: self.underline || base.underline,
            strikethrough: self.strikethrough || base.strikethrough,
            ..*base
        }
    }
}

/// Host-side animation definition (deserialized from wire format).
#[derive(Debug, Clone)]
pub struct HostAnimationDef {
    pub property: AnimProperty,
    pub from: f32,
    pub to: f32,
    pub duration_ms: u32,
    pub delay_ms: u16,
    pub easing: Easing,
    pub loop_mode: LoopMode,
}

/// Host-side transition definition.
///
/// `id_hash` is the FNV1a-32 digest of the widget's
/// `Draw::transition` id argument.
///
/// The transition state map keys on `(canvas_index, id_hash)`
/// so transition state follows the logical draw across
/// tree-shape changes — adding or removing a sibling
/// does not reshuffle state into the wrong draws.
#[derive(Debug, Clone)]
pub struct HostTransitionDef {
    pub id_hash: u32,
    pub duration_ms: u32,
    pub easing: Easing,
}

/// Draw command for canvas (local coordinates)
#[derive(Debug, Clone)]
pub enum DrawCommand {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Fill,
    },
    Centered {
        inner: Box<DrawCommand>,
    },
    Orbit {
        radius: f32,
        angle: f32,
        inner: Box<DrawCommand>,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Fill,
    },
    Arc {
        cx: f32,
        cy: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        width: f32,
        fill: ArcFill,
        segments: ArcSegments,
        cap: ArcCap,
    },
    Svg {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        icon_id: Option<SvgId>,
        anti_alias: bool,
        /// Per-path fill overrides keyed by the SVG path `id`.
        /// Empty by default; populated by `Draw::svg(...).fill(id, color)`.
        fills: Vec<(String, Color)>,
    },
    Bitmap {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
    },
    Qr {
        x: f32,
        y: f32,
        size: f32,
        dark: Color,
        light: Color,
        quiet_zone: u8,
        text: String,
    },
    Rotated {
        angle: f32,
        inner: Box<DrawCommand>,
    },
    Shadow {
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: Box<DrawCommand>,
    },
    Modified {
        animations: Vec<HostAnimationDef>,
        transition: Option<HostTransitionDef>,
        color_space: ColorSpace,
        inner: Box<DrawCommand>,
    },
    Path {
        points: Vec<(f32, f32)>,
        paint: PathPaint,
        closed: bool,
        smooth: bool,
    },
    Sphere {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
        atmosphere: bool,
        center_lat: f32,
        center_lon: f32,
        zoom: f32,
        light_lat: f32,
        light_lon: f32,
    },
    Mesh {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mesh_id: Option<MeshId>,
        args: MeshDrawArgs,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        style: TextStyle,
    },
    CurvedText {
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        text: String,
        style: TextStyle,
    },
    AutofitText {
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
        text: String,
        style: TextStyle,
    },
    NinePatch {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    },
}

/// 9-patch inset data (deserialized from wire format).
#[derive(Debug, Clone, Copy)]
pub struct NinePatchData {
    pub bitmap_id: Option<BitmapId>,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

/// Button skin override data (deserialized from wire format).
#[derive(Debug, Clone)]
pub struct ButtonSkinData {
    pub normal: NinePatchData,
    pub pressed: Option<NinePatchData>,
    pub text_color: Color,
    pub pressed_text_color: Color,
    /// Bitmap already contains the visual content — skip rendering icon/label.
    pub opaque: bool,
}

/// Deserialized tree node
#[derive(Debug, Clone)]
pub enum TreeNode {
    Column(PropsData, Vec<TreeNode>),
    Row(PropsData, Vec<TreeNode>),
    Center(PropsData, Vec<TreeNode>),
    Paragraph {
        props: PropsData,
        base_style: TextStyle,
        spans: Vec<SpanData>,
    },
    Button {
        id: String,
        label: String,
        style: u8,
        size: u8,
        icon_id: Option<SvgId>,
        disabled: bool,
        stretch: bool,
        skin: Option<ButtonSkinData>,
    },
    Spacer {
        flex: f32,
    },
    /// Canvas with optional touch interaction key.
    /// When `touch_key` is `Some`, the canvas registers a hit region and reports
    /// click position via `TreeResult::clicks`.
    Canvas {
        props: PropsData,
        touch_key: Option<String>,
        draws: Vec<DrawCommand>,
    },
    /// Scrollable container
    Scroll {
        scroll_key: String,
        props: PropsData,
        children: Vec<TreeNode>,
    },
    /// Inline notification banner
    Notification {
        kind: u8,
        title: String,
        subtitle: String,
    },
    /// Host-rendered relative-time label.
    RelTime {
        anchor: i64,
        format: RelTimeFormat,
        clamp: RelTimeClamp,
        style: TextStyle,
    },
    /// Carbon status pill — themed chrome around a content child.
    /// `icon` is resolved at decode (`None` = no icon).
    Tag {
        kind: TagKind,
        icon: Option<SvgId>,
        content: Box<TreeNode>,
    },
    /// Segmented view switcher — rounded pill of icon tabs, one active.
    Switcher {
        active: usize,
        disabled: bool,
        tabs: Vec<SwitcherTabData>,
    },
    /// Loading placeholder holding a value's slot while it loads.
    Skeleton(SkeletonData),
    /// Modal dialog overlay
    Modal {
        modal_id: String,
        is_open: bool,
        padding: u16,
        backdrop_alpha: u8,
        title: String,
        content_height: f32,
        /// Modal body background color. `Color::default()` = default.
        bg_color: Color,
        /// Header background color. `Color::default()` = default.
        header_color: Color,
        /// Title text color. `Color::default()` = default.
        title_color: Color,
        /// Maximum modal width. `0` = no limit.
        max_width: u16,
        body: Vec<TreeNode>,
        /// Footer button keys/labels (empty primary_key = no footer).
        footer_primary_key: String,
        footer_primary_label: String,
        footer_secondary_key: String,
        footer_secondary_label: String,
        footer_danger: bool,
    },
    /// Host-rendered progress bar (seek/volume slider)
    ProgressBar {
        touch_key: Option<String>,
        track_h: f32,
        mode: ProgressKind,
        fraction: f32,
        active: bool,
        fill_color: Color,
        track_color: Color,
        bg_color: Color,
        skin: Option<SliderSkinData>,
    },
}

/// Column layout.
#[must_use]
pub fn col(props: PropsData, children: impl IntoIterator<Item = TreeNode>) -> TreeNode {
    TreeNode::Column(props, children.into_iter().collect())
}

/// Row layout.
#[must_use]
pub fn row(props: PropsData, children: impl IntoIterator<Item = TreeNode>) -> TreeNode {
    TreeNode::Row(props, children.into_iter().collect())
}

/// Centered container.
#[must_use]
pub fn center(props: PropsData, children: impl IntoIterator<Item = TreeNode>) -> TreeNode {
    TreeNode::Center(props, children.into_iter().collect())
}

/// Single-span paragraph.
#[must_use]
pub fn text(content: impl Into<String>, style: TextStyle) -> TreeNode {
    TreeNode::Paragraph {
        props: PropsData::default(),
        base_style: style,
        spans: vec![SpanData {
            text: content.into(),
            weight: None,
            color: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }],
    }
}

/// Create a button node.
#[must_use]
pub fn make_button(
    id: impl Into<String>,
    label: impl Into<String>,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<ButtonSkinData>,
) -> TreeNode {
    TreeNode::Button {
        id: id.into(),
        label: label.into(),
        style: style as u8,
        size: size as u8,
        icon_id,
        disabled,
        stretch: false,
        skin,
    }
}

/// Flexible spacer.
#[must_use]
pub fn spacer(flex: f32) -> TreeNode {
    TreeNode::Spacer { flex }
}

/// A fixed vertical gap: an empty column of the given height. The layout
/// engine has only uniform `padding`/`margin`, so one-sided vertical spacing
/// is added as a sized spacer between siblings.
#[must_use]
pub fn fixed_height(height: f32) -> TreeNode {
    col(
        PropsData {
            height,
            ..Default::default()
        },
        Vec::new(),
    )
}

/// Reader for deserializing tree from bytes
struct TreeReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> TreeReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
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

    fn read_u32(&mut self) -> Result<u32> {
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
        decode_fill(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("unexpected end of tree data reading fill"))
    }

    fn read_arc_fill(&mut self) -> Result<ArcFill> {
        decode_arc_fill(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("unexpected end of tree data reading arc fill"))
    }

    fn read_arc_segments(&mut self) -> Result<ArcSegments> {
        decode_arc_segments(self.data, &mut self.pos)
            .ok_or_else(|| anyhow::anyhow!("unexpected end of tree data reading arc segments"))
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

    fn read_props(&mut self) -> Result<PropsData> {
        let Some(props) = PropsData::from_bytes(&self.data[self.pos..]) else {
            bail!("unexpected end of tree data reading props");
        };
        self.pos += PropsData::SIZE;
        // taffy 0.9.2 passes a negative gap through `resolve_or_zero` and sums it
        // into placement, so siblings genuinely overlap — while
        // `can_overlap_siblings` models escape as `is_absolute() || margin < 0.0`
        // and misses it, clearing the container for layering and painting the
        // static and dynamic halves in the wrong order.
        anyhow::ensure!(
            props.gap >= 0.0,
            "negative gap {} in tree at {}",
            props.gap,
            self.pos
        );
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
            bail!("unexpected end of tree data reading text style");
        };
        self.pos += TextStyle::SIZE;
        Ok(style)
    }

    fn read_span(&mut self) -> Result<SpanData> {
        let flags = self.read_u16()?;
        let extra_flags = self.read_u8()?;
        let len = self.read_u16()?;
        let text = self.read_string(len)?;

        let has_weight = (flags >> 12) & 1 != 0;
        let has_color = (flags >> 13) & 1 != 0;

        let weight = if has_weight {
            Some(FontWeight(flags & 0xFFF))
        } else {
            None
        };

        let color = if has_color {
            Some(Color::from_raw(self.read_u32()?))
        } else {
            None
        };

        let italic = (flags >> 14) & 1 != 0;
        let underline = (flags >> 15) & 1 != 0;
        let strikethrough = extra_flags & 1 != 0;

        Ok(SpanData {
            text,
            weight,
            color,
            italic,
            underline,
            strikethrough,
        })
    }

    #[expect(clippy::too_many_lines)]
    fn read_node(&mut self) -> Result<TreeNode> {
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
                let flex = self.read_f32()?;
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
                let draw_count = self.read_u16()?;
                let mut draws = Vec::with_capacity(draw_count as usize);
                for _ in 0..draw_count {
                    draws.push(self.read_draw()?);
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
    fn read_draw(&mut self) -> Result<DrawCommand> {
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

#[cfg(test)]
mod fill_decode_tests {
    use super::*;
    use bmc_wasm_protocol::Fill;

    /// A negative extent paints mirrored geometry that `damage` reports nothing
    /// for — see `read_extent`.
    #[test]
    fn a_negative_extent_is_refused_at_the_boundary() {
        let mut data = vec![DRAW_RECT];
        data.extend_from_slice(&100.0_f32.to_le_bytes());
        data.extend_from_slice(&0.0_f32.to_le_bytes());
        data.extend_from_slice(&(-50.0_f32).to_le_bytes());
        data.extend_from_slice(&10.0_f32.to_le_bytes());
        let mut reader = TreeReader::new(&data);
        let err = reader
            .read_draw()
            .expect_err("a negative width must not reach the renderer")
            .to_string();
        assert!(err.contains("invalid extent"), "wrong rejection: {err}");
    }

    /// `f32::min`/`max` ignore NaN, so one in a rotation range leaves the sweep
    /// at `+INF`/`-INF` and the draw reports no damage while still painting —
    /// permanently, since a transition keeps it in its recorded `from`.
    #[test]
    fn a_non_finite_extent_is_refused_at_the_boundary() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut data = vec![DRAW_RECT];
            data.extend_from_slice(&0.0_f32.to_le_bytes());
            data.extend_from_slice(&0.0_f32.to_le_bytes());
            data.extend_from_slice(&bad.to_le_bytes());
            data.extend_from_slice(&10.0_f32.to_le_bytes());
            let mut reader = TreeReader::new(&data);
            let err = reader
                .read_draw()
                .expect_err("a non-finite extent must not reach the tree")
                .to_string();
            assert!(err.contains("extent"), "wrong rejection for {bad}: {err}");
        }
    }

    #[test]
    fn rect_round_trips_a_radial_fill() {
        let mut data = vec![DRAW_RECT];
        data.extend_from_slice(&1.0_f32.to_le_bytes());
        data.extend_from_slice(&2.0_f32.to_le_bytes());
        data.extend_from_slice(&3.0_f32.to_le_bytes());
        data.extend_from_slice(&4.0_f32.to_le_bytes());
        let red = Color::from_rgb(0xFF, 0, 0);
        let blue = Color::from_rgb(0, 0, 0xFF);
        bmc_wasm_protocol::encode_fill(&mut data, &Fill::radial(red, blue));

        let mut de = TreeReader::new(&data);
        let cmd = de
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_RECT");
        let DrawCommand::Rect { fill, .. } = cmd else {
            panic!("expected Rect, got {cmd:?}");
        };
        assert_eq!(fill, Fill::radial(red, blue));
    }

    #[test]
    fn circle_round_trips_a_linear_fill() {
        let mut data = vec![DRAW_CIRCLE];
        data.extend_from_slice(&5.0_f32.to_le_bytes());
        data.extend_from_slice(&6.0_f32.to_le_bytes());
        data.extend_from_slice(&7.0_f32.to_le_bytes());
        let a = Color::from_rgb(1, 2, 3);
        let b = Color::from_rgb(4, 5, 6);
        bmc_wasm_protocol::encode_fill(&mut data, &Fill::linear(0.0, a, b));

        let mut de = TreeReader::new(&data);
        let DrawCommand::Circle { cx, cy, r, fill } = de
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_CIRCLE")
        else {
            panic!("expected Circle");
        };
        assert_eq!((cx, cy, r), (5.0, 6.0, 7.0));
        assert_eq!(fill, Fill::linear(0.0, a, b));
    }

    #[test]
    fn path_fill_round_trips_a_linear_fill() {
        let mut data = vec![DRAW_PATH];
        data.push(0x04);
        data.extend_from_slice(&2_u16.to_le_bytes());
        for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 10.0)] {
            data.extend_from_slice(&x.to_le_bytes());
            data.extend_from_slice(&y.to_le_bytes());
        }
        let a = Color::from_rgb(1, 2, 3);
        let b = Color::from_rgb(4, 5, 6);
        bmc_wasm_protocol::encode_fill(&mut data, &Fill::linear(0.0, a, b));

        let mut de = TreeReader::new(&data);
        let DrawCommand::Path { paint, .. } = de
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_PATH")
        else {
            panic!("expected Path");
        };
        assert_eq!(paint, PathPaint::Fill(Fill::linear(0.0, a, b)));
    }

    #[test]
    fn path_stroke_round_trips_a_dash_pattern() {
        let mut data = vec![DRAW_PATH];
        data.push(0x08); // stroke (not fill) + dashed
        data.extend_from_slice(&2_u16.to_le_bytes());
        for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 0.0)] {
            data.extend_from_slice(&x.to_le_bytes());
            data.extend_from_slice(&y.to_le_bytes());
        }
        let raw = 0x1122_3344_u32;
        data.extend_from_slice(&raw.to_le_bytes()); // color
        data.extend_from_slice(&3.0_f32.to_le_bytes()); // width
        data.extend_from_slice(&6.0_f32.to_le_bytes()); // dash on
        data.extend_from_slice(&4.0_f32.to_le_bytes()); // dash off

        let mut de = TreeReader::new(&data);
        let DrawCommand::Path { paint, .. } = de
            .read_draw()
            .expect("BUG: test buffer encodes a valid dashed DRAW_PATH")
        else {
            panic!("expected Path");
        };
        assert_eq!(
            paint,
            PathPaint::Stroke {
                color: Color::from_raw(raw),
                width: 3.0,
                dash: Some(Dash { on: 6.0, off: 4.0 }),
            }
        );
    }

    #[test]
    fn qr_round_trips_geometry_style_and_text() {
        let mut data = vec![DRAW_QR];
        data.extend_from_slice(&1.0_f32.to_le_bytes()); // x
        data.extend_from_slice(&2.0_f32.to_le_bytes()); // y
        data.extend_from_slice(&40.0_f32.to_le_bytes()); // size
        data.extend_from_slice(&BLACK.to_u32().to_le_bytes()); // dark
        data.extend_from_slice(&WHITE.to_u32().to_le_bytes()); // light
        data.push(3); // quiet_zone
        data.extend_from_slice(&2_u16.to_le_bytes()); // text len
        data.extend_from_slice(b"hi");

        let mut de = TreeReader::new(&data);
        let DrawCommand::Qr {
            x,
            y,
            size,
            dark,
            light,
            quiet_zone,
            text,
        } = de
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_QR")
        else {
            panic!("expected Qr");
        };
        assert_eq!((x, y, size), (1.0, 2.0, 40.0));
        assert_eq!(
            (dark.to_u32(), light.to_u32()),
            (BLACK.to_u32(), WHITE.to_u32())
        );
        assert_eq!(quiet_zone, 3);
        assert_eq!(text, "hi");
    }

    #[test]
    fn autofit_text_decodes_geometry_mode_bounds_style_and_text() {
        let style = TextStyle {
            size: 40,
            ..TextStyle::default()
        };
        let mut data = vec![DRAW_AUTOFIT_TEXT];
        data.extend_from_slice(&10.0_f32.to_le_bytes());
        data.extend_from_slice(&20.0_f32.to_le_bytes());
        data.extend_from_slice(&100.0_f32.to_le_bytes());
        data.extend_from_slice(&50.0_f32.to_le_bytes());
        data.push(AutoFit::ShrinkAndGrow as u8);
        data.extend_from_slice(&14_u16.to_le_bytes());
        data.extend_from_slice(&64_u16.to_le_bytes());
        data.extend_from_slice(&style.to_bytes());
        data.extend_from_slice(&5_u16.to_le_bytes());
        data.extend_from_slice(b"HELLO");

        let mut reader = TreeReader::new(&data);
        let cmd = reader.read_draw().expect("BUG: decode");
        let DrawCommand::AutofitText {
            x,
            y,
            box_width,
            box_height,
            mode,
            min_size,
            max_size,
            text,
            style: s,
        } = cmd
        else {
            panic!("BUG: expected AutofitText");
        };
        assert_eq!((x, y, box_width, box_height), (10.0, 20.0, 100.0, 50.0));
        assert_eq!(mode, AutoFit::ShrinkAndGrow);
        assert_eq!((min_size, max_size), (14, 64));
        assert_eq!(text, "HELLO");
        assert_eq!(s.size, 40);
    }
}

/// Deserialize a tree from bytes
pub fn deserialize_tree(data: &[u8]) -> Result<TreeNode> {
    if data.is_empty() {
        bail!("empty tree data");
    }
    let mut reader = TreeReader::new(data);
    let node = reader.read_node()?;
    if reader.remaining() > 0 {
        tracing::warn!("tree has {} trailing bytes", reader.remaining());
    }
    Ok(node)
}

// ============================================================================
// Layout Engine
// ============================================================================

use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(feature = "frame-timings")]
use std::time::Instant;

use taffy::prelude::*;
use taffy::{Overflow, Point};

use crate::components::draw::render_draw_command;
use crate::components::modal::{ModalInfo, render_modal};
use crate::components::notification::{
    NotificationData, measure_notification, render_notification,
};
use crate::components::progress_bar::{ProgressBarData, render_progress_bar};
use crate::components::skeleton::{SkeletonData, render_skeleton};
use crate::components::switcher::{
    SwitcherData, SwitcherTabData, register_switcher_hits, render_switcher, switcher_size,
};
use crate::components::tag::{TAG_PAD_VERT, TagData, render_tag, tag_content_padding, tag_theme};
use crate::components::{ButtonSize, ButtonStyle, draw_button_with_target};
use crate::interaction::InteractionState;
use crate::partition::Band;
use crate::renderer::{RenderTarget, Renderer, RendererAssetResolver};
use crate::{
    AnimationState, FrameTimings, ModalState, ScrollState, TransitionState, TransitionStateKey,
};

/// Which half of the static/dynamic partition the walk should emit.
///
/// The walk always *traverses* the whole tree whatever the mode: animation and
/// transition state key on positional counters, so skipping traversal would
/// re-key every animation. Only emission is gated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EmitMode {
    /// Emit everything — full frames, and the fallback whenever no cached
    /// static layer is available.
    #[default]
    All,
    /// Emit only nodes tagged static, to build the cached layer.
    StaticOnly,
    /// Emit only nodes tagged dynamic, to composite over a cached layer.
    DynamicOnly,
}

/// What a frame does with the renderer's cached static layer.
///
/// Capturing and reusing are alternatives, not flags: a frame either rebuilds
/// the layer or paints the stored one. Only meaningful under [`EmitMode::All`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayerUse {
    /// Leave the layer alone and emit the whole tree. The fallback for a
    /// caller with no layer of its own — overlays, the gallery.
    #[default]
    Ignore,
    /// Re-render the static half into the layer, then emit the dynamic half
    /// over it. For a frame whose static content has changed, or the first one.
    Capture,
    /// Blit the stored layer instead of rasterising the static half again.
    /// Falls back to a full pass if the blit fails.
    Reuse,
}

impl EmitMode {
    /// Whether a node with this classification should draw in this mode.
    #[must_use]
    pub fn emits(self, node_is_dynamic: bool) -> bool {
        match self {
            Self::All => true,
            Self::StaticOnly => !node_is_dynamic,
            Self::DynamicOnly => node_is_dynamic,
        }
    }

    /// Whether a canvas draw in `band` should paint in this mode.
    #[must_use]
    pub fn emits_band(self, band: Band) -> bool {
        self.emits(!band.is_in_layer())
    }
}

/// Mutable animation context threaded through the render pipeline.
pub(crate) struct AnimationContext<'a> {
    /// Gates emission for static-layer caching; see [`EmitMode`].
    pub(crate) emit: EmitMode,
    pub(crate) animation_states: &'a mut HashMap<u64, AnimationState>,
    pub(crate) transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
    pub(crate) delta_ms: u32,
    pub(crate) frame_counter: u64,
    pub(crate) draw_counter: u32,
    pub(crate) canvas_index: u16,
    pub(crate) draw_in_canvas: u16,
    /// Monotonic counter for mesh atlas slot allocation (one per `draw_mesh` call).
    pub(crate) mesh_slot_counter: u8,
    /// Set to true when any animation or transition is in progress.
    pub(crate) has_active: bool,
    /// Injected clock (unix seconds) for lazily-built subtrees.
    pub(crate) now_unix_secs: i64,
}

/// Touch interaction info (position relative to element, plus element dimensions).
#[derive(Debug, Clone, Copy)]
pub struct TouchHit {
    /// Local x position (relative to element left edge)
    pub x: f32,
    /// Local y position (relative to element top edge)
    pub y: f32,
    /// Element layout width
    pub width: f32,
    /// Element layout height
    pub height: f32,
}

/// Result from processing a tree
#[derive(Debug, Default)]
pub struct TreeResult {
    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub clicks: HashMap<String, TouchHit>,
    /// Active drag positions on interactive canvases (while finger is down)
    pub drags: HashMap<String, TouchHit>,
    /// Content bounding box (width, height) after layout. May be smaller than
    /// the viewport if the story content doesn't fill the available space.
    pub content_size: (f32, f32),
    /// Soonest ms until a time node's label changes; drives the boundary re-render.
    pub next_frame_delay_ms: Option<u32>,
    /// Union of the bounds of every node that paints dynamically, in logical
    /// pixels. This is the region a damage-tracked frame would have to repaint;
    /// everything outside it is unchanged from the previous frame.
    pub dynamic_bounds: Option<Rect>,
    /// The same regions un-merged, so a repaint can scissor to each rather than
    /// to a bounding box that may cover far more than the parts do.
    pub dynamic_rects: Vec<Rect>,
    /// How many nodes contributed to [`Self::dynamic_bounds`]. A union is only
    /// a good proxy for the repaint cost when the parts are close together.
    pub dynamic_node_count: usize,
    /// Whether the frame asked for the static layer and did not get it, so the
    /// caller must not treat the layer as holding this frame's static half.
    pub static_layer_missed: bool,
}

/// Paragraph data for measurement and rendering
#[derive(Clone, Debug)]
struct ParagraphData {
    base_style: TextStyle,
    spans: Vec<SpanData>,
}

/// Button data stored in taffy node context.
#[derive(Clone, Debug)]
pub(crate) struct ButtonContext {
    id: String,
    label: String,
    style: u8,
    size: u8,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<ButtonSkinData>,
}

/// Nine-patch background image data. `bitmap_id == None` means no background.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BgNinePatch {
    bitmap_id: Option<BitmapId>,
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

/// Host-side slider skin data (deserialized from wire).
#[derive(Clone, Debug)]
pub struct SliderSkinData {
    pub(crate) track: NinePatchData,
    pub(crate) track_h: u16,
    pub(crate) thumb_id: Option<BitmapId>,
    pub(crate) thumb_w: u16,
    pub(crate) thumb_h: u16,
    #[expect(
        dead_code,
        reason = "the wire schema carries a pressed thumb, but host sliders have no pressed-state input"
    )]
    pub(crate) thumb_pressed_id: Option<BitmapId>,
}

/// Node data attached to taffy nodes
#[derive(Clone, Default, Debug)]
pub struct NodeContext {
    /// Whether this **subtree** can change while the guest is idle — drives
    /// whether a dynamic-only walk may skip it wholesale. See
    /// [`crate::partition`].
    pub(crate) dynamic: bool,
    /// Whether this node's **own paint** can change. Deliberately not the same
    /// question: a container holding an animated child is a dynamic *subtree*
    /// but paints a static background, which must come from the cached layer
    /// rather than be repainted over it.
    pub(crate) self_dynamic: bool,
    background: Color,
    /// CSS-modeled box decoration: radius rounds background
    /// and border alike, a zero width paints no border.
    border_radius: f32,
    border_width: f32,
    border_color: Color,
    bg_nine_patch: BgNinePatch,
    paragraph: Option<ParagraphData>,
    button: Option<ButtonContext>,
    draws: Vec<DrawCommand>, // canvas draw commands
    /// Touch interaction key for interactive canvases (None = decorative)
    touch_key: Option<String>,
    notification: Option<NotificationData>,
    /// String key for scroll container state tracking and interaction targeting.
    scroll_key: Option<String>,
    progress_bar: Option<ProgressBarData>,
    tag: Option<TagData>,
    switcher: Option<SwitcherData>,
    skeleton: Option<SkeletonData>,
}

/// Per-frame mutable state passed through the render pipeline.
///
/// Bundles the persistent interaction/modal/scroll/animation/transition
/// caches plus the current frame's counter and delta into a single
/// reference-bag, so [`process_tree`] and [`layout_and_render`] don't need
/// a 12-argument signature each.
#[expect(missing_debug_implementations)]
pub struct ProcessContext<'a> {
    pub interaction: &'a mut InteractionState,
    pub modal_states: &'a mut HashMap<String, ModalState>,
    pub scroll_states: &'a mut HashMap<String, ScrollState>,
    pub animation_states: &'a mut HashMap<u64, AnimationState>,
    pub transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
    pub taffy: &'a mut TaffyTree<NodeContext>,
    pub frame_counter: u64,
    pub delta_ms: u32,
    /// Injected unix-seconds clock for host-formatted time nodes.
    pub now_unix_secs: i64,
    /// Which half of the static/dynamic partition to emit; see [`EmitMode`].
    /// [`EmitMode::All`] reproduces a full frame.
    pub emit: EmitMode,

    /// Regions to repaint, in logical pixels, when the frame can reuse what the
    /// target already holds. Empty means repaint everything.
    ///
    /// These come from the *previous* frames' walks: the damage has to be known
    /// before the layer is composited rather than after the walk that would
    /// discover it, and layout is identical on a cached frame.
    pub damage_rects: &'a [Rect],
    /// What this frame does with the cached static layer; see [`LayerUse`].
    pub static_layer: LayerUse,
    /// Identifies whose static layer to capture and blit. One renderer serves
    /// every widget slot, so callers must pass distinct keys or they overwrite
    /// each other's layer. Use the caller's asset namespace, so slot teardown
    /// reclaims the layer with its other assets.
    pub static_layer_key: &'a str,
}

/// Process a tree: deserialize, layout, render.
///
/// Returns `(tree_node, result, has_active_animations, timings)` — the caller
/// can cache `tree_node` for animation-only frames to skip deserialization.
pub fn process_tree(
    data: &[u8],
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeNode, TreeResult, bool, FrameTimings)> {
    let mut timings = FrameTimings::default();

    // Phase 1: Deserialize
    #[cfg(feature = "frame-timings")]
    let t0 = Instant::now();
    let tree_node = deserialize_tree(data)?;
    #[cfg(feature = "frame-timings")]
    {
        timings.deserialize_us = elapsed_us(t0);
    }

    let (result, has_active) =
        layout_and_render(&tree_node, width, height, renderer, &mut timings, ctx)?;

    Ok((tree_node, result, has_active, timings))
}

/// Layout and render a previously deserialized tree.
///
/// Populates `timings.layout_us` and `timings.render_us`.
pub fn layout_and_render(
    tree_node: &TreeNode,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    timings: &mut FrameTimings,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeResult, bool)> {
    layout_and_render_inner(tree_node, width, height, renderer, None, timings, ctx)
}

/// Layout and render a tree, resolving suspended assets immediately before their draws.
pub fn layout_and_render_with_asset_resolver(
    tree_node: &TreeNode,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    resolver: &mut dyn RendererAssetResolver,
    timings: &mut FrameTimings,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeResult, bool)> {
    layout_and_render_inner(
        tree_node,
        width,
        height,
        renderer,
        Some(resolver),
        timings,
        ctx,
    )
}

fn layout_and_render_inner(
    tree_node: &TreeNode,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    resolver: Option<&mut dyn RendererAssetResolver>,
    timings: &mut FrameTimings,
    ctx: &mut ProcessContext<'_>,
) -> Result<(TreeResult, bool)> {
    // Phase 2: Build Taffy tree + compute layout
    #[cfg(feature = "frame-timings")]
    let t1 = Instant::now();

    let mut result = TreeResult::default();
    let mut modals: Vec<ModalInfo> = Vec::new();

    // Reuse taffy tree — clear nodes but keep internal allocations
    ctx.taffy.clear();
    let root_id = build_taffy_node(
        ctx.taffy,
        tree_node,
        ctx.now_unix_secs,
        &mut result,
        &mut modals,
    )?;

    pin_root_size(ctx.taffy, root_id, width, height)?;

    #[cfg(feature = "frame-timings")]
    {
        timings.taffy_build_us = elapsed_us(t1);
    }

    #[cfg(feature = "frame-timings")]
    let t_compute = Instant::now();
    compute_taffy_layout(ctx.taffy, root_id, renderer)?;
    #[cfg(feature = "frame-timings")]
    {
        timings.taffy_compute_us = elapsed_us(t_compute);
        let (misses, entries) = renderer.paragraph_cache_stats();
        timings.paragraph_misses = misses;
        timings.paragraph_entries = u32::try_from(entries).unwrap_or(u32::MAX);
    }

    result.content_size = root_content_size(ctx.taffy, root_id);

    #[cfg(feature = "frame-timings")]
    {
        timings.layout_us = elapsed_us(t1);
    }

    // Phase 3: Render
    #[cfg(feature = "frame-timings")]
    let t2 = Instant::now();

    let mut anim_ctx = AnimationContext {
        emit: ctx.emit,
        animation_states: ctx.animation_states,
        transition_states: ctx.transition_states,
        delta_ms: ctx.delta_ms,
        frame_counter: ctx.frame_counter,
        draw_counter: 0,
        canvas_index: 0,
        draw_in_canvas: 0,
        mesh_slot_counter: 0,
        has_active: false,
        now_unix_secs: ctx.now_unix_secs,
    };
    let resolver = resolver.map(RefCell::new);
    let mut target = RenderTarget::new(renderer, resolver.as_ref());

    render_tree(
        root_id,
        width,
        height,
        &mut target,
        ctx.taffy,
        ctx.interaction,
        ctx.scroll_states,
        ctx.static_layer,
        ctx.static_layer_key,
        ctx.damage_rects,
        &mut result,
        timings,
        &mut anim_ctx,
    );

    render_modal_overlays(
        &modals,
        width,
        height,
        &mut target,
        ctx.interaction,
        ctx.modal_states,
        ctx.scroll_states,
        ctx.delta_ms,
        ctx.taffy,
        &mut result,
        &mut anim_ctx,
    );

    record_modal_damage(ctx.modal_states, width, height, &mut result);

    sweep_stale_state(&mut anim_ctx, ctx.frame_counter, timings);
    #[cfg(feature = "frame-timings")]
    {
        timings.hit_region_count = ctx.interaction.hit_region_count();
        timings.dynamic_node_count = result.dynamic_node_count;
        timings.dynamic_area_pct = dynamic_area_pct(result.dynamic_bounds, width, height);
        timings.render_us = elapsed_us(t2);
    }

    // Ends `anim_ctx`'s borrows of `ctx` before the check below takes it.
    let has_active = anim_ctx.has_active;
    let _ = anim_ctx;

    // Progress alone is ambiguous: 0.0 means "fully closed" only when `is_open`
    // is false, and otherwise means the modal has just started opening.
    let modal_animating = ctx.modal_states.values().any(|s| {
        if s.is_open {
            s.animation_progress < 1.0
        } else {
            s.animation_progress > 0.0
        }
    });

    Ok((result, has_active || modal_animating))
}

/// Damage the whole surface while a modal is on screen.
///
/// A modal lays out as a `Display::None` leaf and is painted by a later overlay
/// pass, so nothing has described its backdrop to damage tracking. Left
/// unrecorded, the darkening lands only inside the rects the walk reported, and
/// the pixels it covered never repaint once it closes.
///
/// The whole surface, not the panel: the backdrop dims everything. Call after
/// the overlay pass, where a modal's state first appears, or the frame it opens
/// on records nothing.
fn record_modal_damage(
    modal_states: &HashMap<String, ModalState>,
    width: f32,
    height: f32,
    result: &mut TreeResult,
) {
    let visible = modal_states
        .values()
        .any(|s| s.is_open || s.animation_progress > 0.0);
    if !visible {
        return;
    }
    let surface = Rect::new(0.0, 0.0, width, height);
    Rect::union_bounds(&mut result.dynamic_bounds, surface);
    result.dynamic_rects.push(surface);
    result.dynamic_node_count += 1;
}

/// Paint the frame's open modals, always emitting in full.
///
/// A modal is never in the cached layer, so the static half of its body has
/// nowhere else to come from: emitting under [`EmitMode::DynamicOnly`], which is
/// where [`render_tree`] leaves `emit` after a blit, drops the body text.
#[expect(clippy::too_many_arguments, reason = "render plumbing is irreducible")]
fn render_modal_overlays(
    modals: &[ModalInfo],
    width: f32,
    height: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<String, ModalState>,
    scroll_states: &mut HashMap<String, ScrollState>,
    delta_ms: u32,
    taffy: &mut TaffyTree<NodeContext>,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
) {
    if modals.is_empty() {
        return;
    }
    let emit_before = anim_ctx.emit;
    anim_ctx.emit = EmitMode::All;
    for modal in modals {
        render_modal(
            modal,
            width,
            height,
            renderer,
            interaction,
            modal_states,
            scroll_states,
            delta_ms,
            result,
            anim_ctx,
            taffy,
        );
    }
    anim_ctx.emit = emit_before;
}

/// Render the tree, splitting into a static and a dynamic pass when this frame
/// is refreshing the cached layer.
///
/// A refreshing frame renders in the same shape as one reusing the layer:
/// static into the layer, blit, dynamic on top — so static content is
/// rasterised once per guest frame rather than once for the screen and again
/// for the layer. Falls back to one full pass when the layer is unavailable.
#[expect(clippy::too_many_arguments, reason = "render plumbing is irreducible")]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "surface dimensions are small positive values"
)]
fn render_tree(
    root_id: taffy::NodeId,
    width: f32,
    height: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    taffy: &TaffyTree<NodeContext>,
    interaction: &mut InteractionState,
    scroll_states: &mut HashMap<String, ScrollState>,
    static_layer: LayerUse,
    static_layer_key: &str,
    damage_rects: &[Rect],
    result: &mut TreeResult,
    timings: &mut FrameTimings,
    anim_ctx: &mut AnimationContext<'_>,
) {
    // The parameter stays either way, so callers need no cfg of their own.
    #[cfg(not(feature = "frame-timings"))]
    let _ = timings;

    // Drain whatever is already queued so the first measured pass is not
    // charged for work recorded before it.
    #[cfg(feature = "frame-timings")]
    let pass_timing = gpu_pass_timing_enabled();
    #[cfg(feature = "frame-timings")]
    if pass_timing {
        renderer.flush_and_fence_us();
    }

    let split = anim_ctx.emit == EmitMode::All
        && static_layer == LayerUse::Capture
        && renderer.begin_static_layer(static_layer_key, width as u32, height as u32);
    if anim_ctx.emit == EmitMode::All && static_layer == LayerUse::Capture && !split {
        result.static_layer_missed = true;
    }

    // Nothing static changed since the layer was captured, so paint it rather
    // than rasterising the same content again.
    if !split && anim_ctx.emit == EmitMode::All && static_layer == LayerUse::Reuse {
        // Repaint only the damaged regions when the caller supplied them; the
        // rest of the target still holds a correct earlier frame.
        let blitted = if damage_rects.is_empty() {
            renderer.blit_static_layer(static_layer_key)
        } else {
            renderer.blit_static_layer_rects(static_layer_key, damage_rects)
        };
        if blitted {
            #[cfg(feature = "frame-timings")]
            if pass_timing {
                timings.gpu_blit_us = renderer.flush_and_fence_us();
            }
            anim_ctx.emit = EmitMode::DynamicOnly;
        } else {
            // The walk below repaints every draw but only where a draw paints,
            // so on a target the caller left uncleared, whatever the dynamic
            // half painted last frame stays where it was. Lay down the frame
            // base first, opaque black to match what `begin_static_layer`
            // captured against. Only reachable with a captured layer, so the
            // transparent-based overlays never take it.
            renderer.fill_rect(0.0, 0.0, width, height, Color::from_rgb(0, 0, 0));
            result.static_layer_missed = true;
        }
    }

    if split {
        anim_ctx.emit = EmitMode::StaticOnly;
        render_taffy_node(
            taffy,
            root_id,
            0.0,
            0.0,
            renderer,
            interaction,
            scroll_states,
            result,
            anim_ctx,
            0,
        );
        renderer.end_static_layer(static_layer_key);
        #[cfg(feature = "frame-timings")]
        if pass_timing {
            timings.gpu_capture_us = renderer.flush_and_fence_us();
        }
        renderer.blit_static_layer(static_layer_key);
        #[cfg(feature = "frame-timings")]
        if pass_timing {
            timings.gpu_blit_us = renderer.flush_and_fence_us();
        }

        // Restart the counters so this pass matches a cached frame's exactly —
        // it is the same walk, and animation state keys on `draw_counter`.
        anim_ctx.emit = EmitMode::DynamicOnly;
        anim_ctx.draw_counter = 0;
        anim_ctx.canvas_index = 0;
        // The mesh atlas caches per slot and skips re-rendering when a slot
        // already holds the same mesh, so a draw landing on a different slot
        // depending on frame type would re-render on every alternation.
        anim_ctx.mesh_slot_counter = 0;
    }
    render_taffy_node(
        taffy,
        root_id,
        0.0,
        0.0,
        renderer,
        interaction,
        scroll_states,
        result,
        anim_ctx,
        0,
    );

    #[cfg(feature = "frame-timings")]
    if pass_timing {
        timings.gpu_dynamic_us = renderer.flush_and_fence_us();
    }
}

/// The root's laid-out content extent, or zero when layout produced none.
fn root_content_size(taffy: &TaffyTree<NodeContext>, root_id: NodeId) -> (f32, f32) {
    taffy.layout(root_id).map_or((0.0, 0.0), |layout| {
        (layout.content_size.width, layout.content_size.height)
    })
}

/// Override the root's size per axis: a non-zero value pins that axis, and zero
/// keeps the node's own style so its content sizes it.
fn pin_root_size(
    taffy: &mut TaffyTree<NodeContext>,
    root_id: NodeId,
    width: f32,
    height: f32,
) -> Result<()> {
    if width <= 0.0 && height <= 0.0 {
        return Ok(());
    }
    if let Ok(style) = taffy.style(root_id) {
        let mut new_style = style.clone();
        if width > 0.0 {
            new_style.size.width = length(width);
        }
        if height > 0.0 {
            new_style.size.height = length(height);
        }
        taffy.set_style(root_id, new_style)?;
    }
    Ok(())
}

/// Microseconds since `start`, saturating rather than wrapping.
#[cfg(feature = "frame-timings")]
fn elapsed_us(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX)
}

/// Share of a `width` x `height` surface covered by `bounds`, in percent.
#[cfg(feature = "frame-timings")]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a percentage of a surface is a small non-negative value"
)]
fn dynamic_area_pct(bounds: Option<Rect>, width: f32, height: f32) -> u32 {
    let surface = width * height;
    if surface <= 0.0 {
        return 0;
    }
    bounds.map_or(0, |Rect { w, h, .. }| {
        (100.0 * w * h / surface).round() as u32
    })
}

/// Drop animation and transition states no walk touched this frame, and record
/// what survived.
///
/// Both are keyed by content, so a node that stopped drawing would otherwise
/// keep its state alive forever. The counts ride along because this is the only
/// point where the swept sizes are known.
fn sweep_stale_state(
    anim_ctx: &mut AnimationContext<'_>,
    frame_counter: u64,
    timings: &mut FrameTimings,
) {
    // The parameter stays either way, so callers need no cfg of their own.
    #[cfg(not(feature = "frame-timings"))]
    let _ = timings;

    anim_ctx
        .animation_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);
    anim_ctx
        .transition_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);

    #[cfg(feature = "frame-timings")]
    {
        timings.animation_state_count = anim_ctx.animation_states.len();
        timings.transition_state_count = anim_ctx.transition_states.len();
    }
}

/// Build the taffy node for `node` and tag it with its static/dynamic
/// classification.
///
/// The classification rides on [`NodeContext`] because the render walk sees only
/// taffy nodes, and this is the last point where both are in scope. Recursion
/// goes through this wrapper so every node owning a context gets tagged; one
/// built without a context stays untagged and is treated as dynamic.
pub(crate) fn build_taffy_node(
    taffy: &mut TaffyTree<NodeContext>,
    node: &TreeNode,
    now_unix_secs: i64,
    result: &mut TreeResult,
    modals: &mut Vec<ModalInfo>,
) -> Result<taffy::NodeId> {
    let id = build_taffy_node_inner(taffy, node, now_unix_secs, result, modals)?;
    // O(nodes x depth) boolean work, negligible next to the Taffy build it
    // wraps — not worth a bottom-up accumulator threaded through every arm.
    let dynamic = crate::partition::node_is_dynamic(node);
    let self_dynamic = crate::partition::node_self_is_dynamic(node);
    if let Some(ctx) = taffy.get_node_context_mut(id) {
        ctx.dynamic = dynamic;
        ctx.self_dynamic = self_dynamic;
    }
    Ok(id)
}

#[expect(clippy::too_many_lines)]
fn build_taffy_node_inner(
    taffy: &mut TaffyTree<NodeContext>,
    node: &TreeNode,
    now_unix_secs: i64,
    result: &mut TreeResult,
    modals: &mut Vec<ModalInfo>,
) -> Result<taffy::NodeId> {
    match node {
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, now_unix_secs, result, modals))
                .collect::<Result<_>>()?;

            let is_center = matches!(node, TreeNode::Center(_, _));
            let flex_dir = if matches!(node, TreeNode::Row(_, _)) {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };

            let is_abs = props.is_absolute();
            let style = Style {
                position: if is_abs {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: if is_abs {
                    inset_from_props(props)
                } else {
                    taffy::Rect::auto()
                },
                flex_direction: flex_dir,
                flex_wrap: if props.wrap {
                    FlexWrap::Wrap
                } else {
                    FlexWrap::NoWrap
                },
                justify_content: if is_center || props.wrap {
                    Some(JustifyContent::Center)
                } else {
                    match props.justify_content {
                        Justify::Start => None,
                        Justify::Center => Some(JustifyContent::Center),
                        Justify::End => Some(JustifyContent::End),
                        Justify::SpaceBetween => Some(JustifyContent::SpaceBetween),
                    }
                },
                align_items: match props.cross_align {
                    CrossAlign::Center => Some(AlignItems::Center),
                    CrossAlign::Start => Some(AlignItems::Start),
                    CrossAlign::End => Some(AlignItems::End),
                    CrossAlign::Stretch => {
                        if is_center {
                            Some(AlignItems::Center)
                        } else {
                            None
                        }
                    }
                },
                align_content: if props.wrap {
                    Some(AlignContent::Center)
                } else {
                    None
                },
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                max_size: max_size_from_props(props),
                flex_grow: if is_center && props.flex == 0.0 {
                    1.0
                } else {
                    props.flex
                },
                // flex > 0 acts like CSS `flex: N` (grow N, basis 0) so the item
                // starts at zero and grows into available space.
                flex_basis: if props.flex > 0.0 || (is_center && props.flex == 0.0) {
                    length(0.0_f32)
                } else {
                    Dimension::auto()
                },
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            // Attached even when nothing paints: the context is the only place
            // the static/dynamic tag can live. Pixel-neutral, because the walk
            // guards the background on `!= Color::default()` and every other
            // field on `Some`, so an otherwise-empty context draws nothing.
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    border_radius: props.border_radius,
                    border_width: props.border_width,
                    border_color: props.border_color,
                    bg_nine_patch: bg_np_from_props(props),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Paragraph {
            props,
            base_style,
            spans,
        } => {
            let is_abs = props.is_absolute();
            // Don't pre-measure - use Taffy's measure callback with available width
            let style = Style {
                position: if is_abs {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: if is_abs {
                    inset_from_props(props)
                } else {
                    taffy::Rect::auto()
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                flex_grow: props.flex,
                // Let measure function determine size based on available width
                ..Default::default()
            };

            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    bg_nine_patch: bg_np_from_props(props),
                    paragraph: Some(ParagraphData {
                        base_style: *base_style,
                        spans: spans.clone(),
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::RelTime {
            anchor,
            format,
            clamp,
            style,
        } => {
            let delta = now_unix_secs - *anchor;
            let text = crate::components::format_rel(delta, *format, *clamp);
            // A pinned label yields no delay, so it never schedules a wake.
            if let Some(delay) = crate::components::next_change_delay_ms(delta, *format, *clamp) {
                result.next_frame_delay_ms =
                    Some(result.next_frame_delay_ms.map_or(delay, |d| d.min(delay)));
            }
            let id = taffy.new_leaf(Style::default())?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    paragraph: Some(ParagraphData {
                        base_style: *style,
                        spans: vec![SpanData {
                            text,
                            weight: None,
                            color: None,
                            italic: false,
                            underline: false,
                            strikethrough: false,
                        }],
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Tag {
            kind,
            icon,
            content,
        } => {
            let content_id = build_taffy_node(taffy, content, now_unix_secs, result, modals)?;
            let (pad_left, pad_right) = tag_content_padding(icon.is_some());
            let style = Style {
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                // Size to content, don't stretch in a column parent.
                align_self: Some(AlignSelf::FlexStart),
                flex_shrink: 0.0,
                // Pill height = content + vertical padding; corners clamp to a stadium.
                padding: taffy::Rect {
                    left: length(pad_left),
                    right: length(pad_right),
                    top: length(TAG_PAD_VERT),
                    bottom: length(TAG_PAD_VERT),
                },
                ..Default::default()
            };
            let id = taffy.new_with_children(style, &[content_id])?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    tag: Some(TagData {
                        kind: *kind,
                        icon: *icon,
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Button {
            id: btn_id,
            label,
            style: btn_style,
            size: btn_size,
            icon_id,
            disabled,
            stretch,
            skin,
        } => {
            let sz = ButtonSize::from(*btn_size);
            let height = sz.height();

            let style = Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(height),
                },
                // Buttons should never shrink below their content size
                flex_shrink: 0.0,
                // Stretch fills the cross axis (full-width in a column); else content.
                align_self: Some(if *stretch {
                    AlignSelf::Stretch
                } else {
                    AlignSelf::FlexStart
                }),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    button: Some(ButtonContext {
                        id: btn_id.clone(),
                        label: label.clone(),
                        style: *btn_style,
                        size: *btn_size,
                        icon_id: *icon_id,
                        disabled: *disabled,
                        skin: skin.clone(),
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Spacer { flex } => {
            let style = Style {
                flex_grow: *flex,
                ..Default::default()
            };
            Ok(taffy.new_leaf(style)?)
        }

        TreeNode::Canvas {
            props,
            touch_key,
            draws,
        } => {
            let has_explicit_size = props.width > 0.0 || props.height > 0.0;
            let is_abs = props.is_absolute();
            let style = Style {
                position: if is_abs {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: if is_abs {
                    inset_from_props(props)
                } else {
                    taffy::Rect::auto()
                },
                size: size_from_props(props),
                max_size: max_size_from_props(props),
                flex_grow: props.flex,
                // Prevent taffy from shrinking canvases with explicit sizes
                flex_shrink: if has_explicit_size { 0.0 } else { 1.0 },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    bg_nine_patch: bg_np_from_props(props),
                    draws: draws.clone(),
                    touch_key: touch_key.clone(),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Scroll {
            scroll_key,
            props,
            children,
        } => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, now_unix_secs, result, modals))
                .collect::<Result<_>>()?;

            // Extra right padding so content doesn't sit under the scrollbar
            // overlay. CDS spacing scale is 4/8/16/24 — 8 px is the smallest
            // step that keeps content off the scrollbar visual without
            // looking padded twice on top of `props.padding`.
            let scrollbar_clearance = 8.0;
            let mut padding = padding_uniform(props.padding);
            padding.right = length(props.padding + scrollbar_clearance);

            let style = Style {
                flex_direction: FlexDirection::Column,
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding,
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                max_size: max_size_from_props(props),
                flex_grow: props.flex,
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Scroll,
                },
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    bg_nine_patch: bg_np_from_props(props),
                    scroll_key: Some(scroll_key.clone()),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Notification {
            kind,
            title,
            subtitle,
        } => {
            let style = Style::default();
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    notification: Some(NotificationData {
                        kind: *kind,
                        title: title.clone(),
                        subtitle: subtitle.clone(),
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Switcher {
            active,
            disabled,
            tabs,
        } => {
            let data = SwitcherData {
                active: *active,
                disabled: *disabled,
                tabs: tabs.clone(),
            };
            let (sw, sh) = switcher_size(&data);
            let style = Style {
                size: Size {
                    width: length(sw),
                    height: length(sh),
                },
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    switcher: Some(data),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Skeleton(data) => {
            let (width, height) = data.layout_size();
            let style = Style {
                size: Size {
                    width: width.map_or_else(auto, length),
                    height: length(height),
                },
                // A `Fill` skeleton spans its row by growing: a childless
                // leaf has no content to size an `auto` width from, and a
                // percentage resolves to zero while the row's own width is
                // still indefinite.
                flex_grow: if width.is_none() { 1.0 } else { 0.0 },
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    skeleton: Some(*data),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        TreeNode::Modal {
            modal_id,
            is_open,
            padding,
            backdrop_alpha,
            title,
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
            ..
        } => {
            // Store modal for overlay rendering
            modals.push(ModalInfo {
                modal_id: modal_id.clone(),
                is_open: *is_open,
                padding: *padding,
                backdrop_alpha: *backdrop_alpha,
                title: title.clone(),
                bg_color: *bg_color,
                header_color: *header_color,
                title_color: *title_color,
                max_width: *max_width,
                body: body.clone(),
                footer_primary_key: footer_primary_key.clone(),
                footer_primary_label: footer_primary_label.clone(),
                footer_secondary_key: footer_secondary_key.clone(),
                footer_secondary_label: footer_secondary_label.clone(),
                footer_danger: *footer_danger,
            });

            // Modal doesn't participate in normal layout — hidden from flex/gap.
            let style = Style {
                display: Display::None,
                ..Default::default()
            };
            Ok(taffy.new_leaf(style)?)
        }

        TreeNode::ProgressBar {
            touch_key,
            track_h,
            mode,
            fraction,
            active,
            fill_color,
            track_color,
            bg_color,
            skin,
        } => {
            // When skinned, use the skin's track height for layout
            let effective_h = skin.as_ref().map_or(*track_h, |s| f32::from(s.track_h));
            let dot_radius = effective_h * 2.0;
            let bar_height = if skin.is_some() {
                // Skinned: thumb may be taller than track
                let thumb_h = skin.as_ref().map_or(0.0, |s| f32::from(s.thumb_h));
                effective_h.max(thumb_h)
            } else if *mode == ProgressKind::Meter {
                // No drag thumb to budget for — the meter is just its track.
                effective_h
            } else {
                dot_radius * 2.0 + effective_h
            };
            let style = Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(bar_height),
                },
                flex_grow: 1.0,
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    touch_key: touch_key.clone(),
                    progress_bar: Some(ProgressBarData {
                        track_h: *track_h,
                        mode: *mode,
                        fraction: *fraction,
                        active: *active,
                        fill_color: *fill_color,
                        track_color: *track_color,
                        bg_color: *bg_color,
                        skin: skin.clone(),
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }
    }
}

/// Whitespace a line may break at. Non-breaking spaces glue their
/// neighbors into one word, exactly as the wrapper treats them.
fn is_breaking_whitespace(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{A0}' | '\u{2007}' | '\u{202F}')
}

/// Split spans into words for min-content probing. A word ends only at
/// breaking whitespace — it may cross span boundaries, so each word is
/// a list of styled fragments preserving its parts' span styles.
fn split_min_content_words(spans: &[SpanData]) -> Vec<Vec<SpanData>> {
    let mut words: Vec<Vec<SpanData>> = Vec::new();
    let mut word: Vec<SpanData> = Vec::new();
    for span in spans {
        for (i, fragment) in span.text.split(is_breaking_whitespace).enumerate() {
            if i > 0 && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if !fragment.is_empty() {
                word.push(SpanData {
                    text: fragment.to_owned(),
                    ..span.clone()
                });
            }
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Min-content width of a paragraph: the widest single word measured
/// unwrapped. Wrapping at this width breaks at word boundaries only,
/// so both axes of the resulting measurement match CSS min-content.
pub(crate) fn min_content_paragraph_width(
    renderer: &mut dyn Renderer,
    base_style: &TextStyle,
    spans: &[SpanData],
) -> f32 {
    let mut min_width = 0.0_f32;
    for word in split_min_content_words(spans) {
        let (w, _) = renderer.measure_paragraph(base_style, &word, None);
        min_width = min_width.max(w);
    }
    min_width
}

/// Compute taffy layout with the standard measure function for paragraphs,
/// notifications, and buttons.
pub(crate) fn compute_taffy_layout(
    taffy: &mut TaffyTree<NodeContext>,
    root_id: taffy::NodeId,
    renderer: &mut dyn Renderer,
) -> Result<()> {
    taffy.compute_layout_with_measure(
        root_id,
        Size::MAX_CONTENT,
        |known_dimensions, available_space, _node_id, node_context, _style| {
            if let (Some(w), Some(h)) = (known_dimensions.width, known_dimensions.height) {
                return Size {
                    width: w,
                    height: h,
                };
            }

            if let Some(ctx) = node_context {
                if let Some(ref para) = ctx.paragraph {
                    // A min-content probe wraps at the widest single word.
                    // Probing at max_width 0 instead breaks per glyph into an
                    // absurdly tall tower whose height becomes the min-size
                    // floor of every ancestor, freezing flex containers at
                    // bogus minimums; probing unwrapped inflates the min
                    // width to the whole line, so shrinkable panels blow out
                    // their row instead of wrapping.
                    let available_width = known_dimensions.width.or(match available_space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        AvailableSpace::MinContent => Some(min_content_paragraph_width(
                            renderer,
                            &para.base_style,
                            &para.spans,
                        )),
                        AvailableSpace::MaxContent => None,
                    });
                    let max_width = if para.base_style.text_overflow != TextOverflow::Wrap {
                        None
                    } else if para.base_style.max_width > 0 {
                        Some(
                            (para.base_style.max_width as f32)
                                .min(available_width.unwrap_or(f32::MAX)),
                        )
                    } else {
                        available_width
                    };
                    let (w, h) =
                        renderer.measure_paragraph(&para.base_style, &para.spans, max_width);
                    return Size {
                        width: known_dimensions.width.unwrap_or(w),
                        height: known_dimensions.height.unwrap_or(h),
                    };
                }

                if let Some(ref notif) = ctx.notification {
                    return measure_notification(
                        notif,
                        known_dimensions,
                        available_space,
                        renderer,
                    );
                }

                if let Some(ref btn) = ctx.button {
                    let sz = ButtonSize::from(btn.size);
                    let h = sz.height();
                    let w = if btn.icon_id.is_some() && btn.label.is_empty() {
                        h
                    } else if btn.icon_id.is_some() {
                        let text_w = renderer.measure_text(&btn.label, sz.font_size());
                        sz.h_padding()
                            + sz.icon_size()
                            + sz.icon_text_gap()
                            + text_w
                            + sz.h_padding()
                    } else {
                        let text_w = renderer.measure_text(&btn.label, sz.font_size());
                        text_w + sz.h_padding() * 2.0
                    };
                    return Size {
                        width: known_dimensions.width.unwrap_or(w),
                        height: known_dimensions.height.unwrap_or(h),
                    };
                }
            }

            Size::ZERO
        },
    )?;
    Ok(())
}

#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn render_taffy_node(
    taffy: &TaffyTree<NodeContext>,
    node_id: taffy::NodeId,
    parent_x: f32,
    parent_y: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    scroll_states: &mut HashMap<String, ScrollState>,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
    depth: usize,
) {
    let Ok(layout) = taffy.layout(node_id) else {
        return;
    };
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;
    let w = layout.size.width;
    let h = layout.size.height;

    // Extract IDs before immutable context borrow
    let scroll_key = taffy
        .get_node_context(node_id)
        .and_then(|ctx| ctx.scroll_key.clone());
    let touch_key = taffy
        .get_node_context(node_id)
        .and_then(|ctx| ctx.touch_key.clone());

    // Traverse everything, always: emission is what costs, and skipping a
    // subtree would mean restoring the positional counters its walk would have
    // advanced and replaying the hit regions it never registered.
    //
    // The gate is per node, not per subtree: a dynamic subtree still holds
    // static nodes a dynamic-only pass must not repaint, and vice versa. An
    // untagged node falls through to `true` and draws in every mode rather
    // than vanishing.
    let emits = taffy
        .get_node_context(node_id)
        .is_none_or(|ctx| anim_ctx.emit.emits(ctx.self_dynamic));

    // The index this node's canvas draws key their transition state on, read
    // before the walk below advances it.
    let canvas_index = anim_ctx.canvas_index;

    if let Some(ctx) = taffy.get_node_context(node_id) {
        if !emits {
            // Fall through to children: a static container can hold a dynamic
            // descendant, and vice versa.
        } else if let Some(bitmap_id) = ctx.bg_nine_patch.bitmap_id {
            let np = &ctx.bg_nine_patch;
            renderer.draw_nine_patch(x, y, w, h, bitmap_id, np.left, np.top, np.right, np.bottom);
        } else {
            if ctx.background != Color::default() {
                if ctx.border_radius > 0.0 {
                    renderer.fill_rounded_rect(x, y, w, h, ctx.border_radius, ctx.background);
                } else {
                    renderer.fill_rect(x, y, w, h, ctx.background);
                }
            }
            // The border strokes inside the box, over the background's edge,
            // as CSS paints a border-box border.
            if ctx.border_width > 0.0 {
                let inset = ctx.border_width / 2.0;
                renderer.stroke_rounded_rect(
                    x + inset,
                    y + inset,
                    w - ctx.border_width,
                    h - ctx.border_width,
                    (ctx.border_radius - inset).max(0.0),
                    ctx.border_width,
                    ctx.border_color,
                );
            }
        }

        // Tag pill + leading icon, painted behind the content child.
        if emits && let Some(ref tag) = ctx.tag {
            render_tag(tag, x, y, w, h, renderer);
        }

        if emits && let Some(ref para) = ctx.paragraph {
            renderer.draw_paragraph(&para.base_style, &para.spans, x, y, w);
        }

        // Registration is unconditional, only the painting is gated: a button
        // whose half this pass skipped still has to claim its hit region and
        // consume a pending click, or it goes dead.
        if let Some(ref btn) = ctx.button {
            let (clicked, click_pos) = if emits {
                draw_button_with_target(
                    renderer,
                    interaction,
                    &btn.id,
                    &btn.label,
                    x,
                    y,
                    w,
                    h,
                    ButtonStyle::from(btn.style as u32),
                    ButtonSize::from(btn.size),
                    btn.icon_id,
                    btn.disabled,
                    btn.skin.as_ref(),
                )
            } else if btn.disabled {
                (false, None)
            } else {
                interaction.button_with_pos(&btn.id, crate::interaction::Rect::new(x, y, w, h))
            };
            if clicked {
                result.clicks.insert(
                    btn.id.clone(),
                    TouchHit {
                        x: click_pos.map_or(0.0, |p| p.0),
                        y: click_pos.map_or(0.0, |p| p.1),
                        width: w,
                        height: h,
                    },
                );
            }
        }

        // Render canvas draw commands with local coordinates
        if !ctx.draws.is_empty() {
            renderer.push_scissor(x, y, w, h);
            anim_ctx.draw_in_canvas = 0;
            // Split per draw: a canvas is tagged dynamic when any single draw
            // is, so the node-level gate is too coarse here. Skipping a
            // non-`Modified` draw cannot desynchronise `draw_counter`, which
            // only advances inside the `Modified` arm.
            let bands = crate::partition::canvas_bands(&ctx.draws);
            for (draw, band) in ctx.draws.iter().zip(bands) {
                if anim_ctx.emit.emits_band(band) {
                    render_draw_command(renderer, draw, x, y, w, h, anim_ctx);
                }
                anim_ctx.draw_in_canvas += 1;
            }
            anim_ctx.canvas_index += 1;
            renderer.pop_scissor();
        }

        // Progress bar: host-rendered track + fill + squiggle + dot
        if emits && let Some(ref pb) = ctx.progress_bar {
            renderer.push_scissor(x, y, w, h);
            let has_active = render_progress_bar(renderer, pb, x, y, w, h, anim_ctx);
            if has_active {
                anim_ctx.has_active = true;
            }
            renderer.pop_scissor();
        }
    }

    // Record what a damage-tracked frame would have to repaint. Leaves only:
    // `dynamic` on a container merely says a descendant animates, so counting
    // containers would union the whole subtree and report the root as damaged.
    //
    // After the draws above, not before: that pass advances each transition's
    // recorded span, and a rect taken beforehand describes the span the
    // previous walk interpolated, so a target the guest just moved sweeps
    // outside it.
    if taffy.child_count(node_id) == 0
        && let Some(ctx) = taffy.get_node_context(node_id)
        && ctx.dynamic
    {
        let node = Rect::new(x, y, w, h);
        // A canvas is one leaf holding many draws, so its own rect is far
        // coarser than what the dynamic pass repaints: `crate::damage` bounds
        // each draw, and only one it cannot bound falls back to the whole
        // canvas. A dynamic leaf with no draws is host-driven and repaints
        // entirely.
        let mut damaged = ctx.draws.is_empty().then_some(node);
        let bands = crate::partition::canvas_bands(&ctx.draws);
        for (draw, _) in ctx
            .draws
            .iter()
            .zip(bands)
            .filter(|(_, band)| !band.is_in_layer())
        {
            match crate::damage::canvas_draw_damage(
                draw,
                node,
                anim_ctx.transition_states,
                canvas_index,
            ) {
                crate::damage::DrawDamage::Region(rect) => {
                    Rect::union_bounds(&mut damaged, rect);
                }
                crate::damage::DrawDamage::Nothing => {}
                crate::damage::DrawDamage::WholeCanvas => {
                    damaged = Some(node);
                    break;
                }
            }
        }
        if let Some(damaged) = damaged {
            Rect::union_bounds(&mut result.dynamic_bounds, damaged);
            result.dynamic_rects.push(damaged);
            result.dynamic_node_count += 1;
        }
    }

    // Canvas touch hit-testing (needs mutable interaction, outside the immutable ctx borrow)
    if let Some(ref tk) = touch_key {
        let bounds = crate::interaction::Rect::new(x, y, w, h);
        let (clicked, click_pos) = interaction.button_with_pos(tk, bounds);
        if clicked && let Some((lx, ly)) = click_pos {
            result.clicks.insert(
                tk.clone(),
                TouchHit {
                    x: lx,
                    y: ly,
                    width: w,
                    height: h,
                },
            );
        }
        // Active drag: finger is down on this element
        if let Some((lx, ly)) = interaction.get_drag_pos(tk, bounds) {
            result.drags.insert(
                tk.clone(),
                TouchHit {
                    x: lx,
                    y: ly,
                    width: w,
                    height: h,
                },
            );
        }
    }

    // Gated like every other paint site: ungated, a static draw accumulates a
    // copy per frame, since nothing repaints underneath it.
    if emits
        && let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref notif) = ctx.notification
    {
        render_notification(notif, x, y, w, h, renderer);
    }

    if let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref sw) = ctx.switcher
    {
        if emits {
            render_switcher(sw, x, y, w, h, renderer);
        }
        // Ungated, like the canvas hit-testing below: a pass that paints
        // nothing still owns its share of the frame's hit regions.
        register_switcher_hits(sw, x, y, w, h, interaction, result);
    }

    if emits
        && let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref sk) = ctx.skeleton
    {
        render_skeleton(&mut *renderer, sk, x, y, w, h);
    }

    let Ok(children) = taffy.children(node_id) else {
        return;
    };

    // Scroll container: scissor-clip + offset children by scroll amount
    if let Some(ref sk) = scroll_key {
        // Compute content height from children's layouts
        let content_height = children
            .iter()
            .filter_map(|&cid| taffy.layout(cid).ok())
            .map(|cl| cl.location.y + cl.size.height)
            .fold(0.0_f32, f32::max);

        let max_scroll = (content_height - h).max(0.0);
        let has_scrollbar = content_height > h;

        // Scrollbar hit region (overlaid, right edge of viewport)
        let sbar_width_normal = 4.0_f32;
        let sbar_width_active = 12.0_f32;
        let sbar_margin = 2.0_f32;
        // Hit region is generous (active width) so it's easy to grab
        let sbar_hit_w = sbar_width_active + sbar_margin;
        let sbar_key = format!("{sk}::sbar");
        let sbar_pressed = if has_scrollbar {
            let sbar_hit_rect = crate::interaction::Rect::new(x + w - sbar_hit_w, y, sbar_hit_w, h);
            interaction.button(&sbar_key, sbar_hit_rect);
            interaction.is_pressed(&sbar_key)
        } else {
            false
        };

        // Register content hit region for drag/wheel scrolling
        let scroll_region = crate::interaction::Rect::new(x, y, w, h);
        interaction.button(sk, scroll_region);

        // Read scroll delta — scrollbar drag scales by content/viewport ratio
        let scroll_delta = if sbar_pressed {
            let ratio = if h > 0.0 { content_height / h } else { 1.0 };
            interaction.get_scroll_delta(&sbar_key) * ratio
        } else if interaction.is_pressed(sk) {
            -interaction.get_scroll_delta(sk)
        } else {
            interaction.get_scroll_delta_in(&scroll_region)
        };
        // Once per container per frame: the offset advances inside the walk,
        // and a capture frame walks the tree twice before the modal overlays
        // are walked a third time.
        let scroll_delta = interaction.take_scroll_delta(sk, scroll_delta);

        let state = scroll_states.entry(sk.clone()).or_default();
        state.scroll_offset += scroll_delta;
        state.scroll_offset = state.scroll_offset.clamp(0.0, max_scroll);

        let offset = state.scroll_offset;

        renderer.push_scissor(x, y, w, h);
        for child_id in children {
            // Render culling: skip children entirely outside the visible viewport
            if let Ok(cl) = taffy.layout(child_id) {
                let child_top = y - offset + cl.location.y;
                let child_bottom = child_top + cl.size.height;
                if child_bottom < y || child_top > y + h {
                    continue;
                }
            }
            render_taffy_node(
                taffy,
                child_id,
                x,
                y - offset,
                renderer,
                interaction,
                scroll_states,
                result,
                anim_ctx,
                depth + 1,
            );
        }
        renderer.pop_scissor();

        // Draw scrollbar overlay (no reflow — drawn after content, outside scissor)
        if has_scrollbar {
            let sbar_w = if sbar_pressed {
                sbar_width_active
            } else {
                sbar_width_normal
            };
            let sbar_x = x + w - sbar_w - sbar_margin;
            let thumb_ratio = h / content_height;
            let thumb_height = (h * thumb_ratio).max(16.0);
            let scroll_ratio = if max_scroll > 0.0 {
                offset / max_scroll
            } else {
                0.0
            };
            let thumb_y = y + scroll_ratio * (h - thumb_height);
            let sbar_color = if sbar_pressed { GRAY_50 } else { GRAY_60 };

            renderer.fill_rect(sbar_x, thumb_y, sbar_w, thumb_height, sbar_color);
        }

        return;
    }

    for child_id in children {
        render_taffy_node(
            taffy,
            child_id,
            x,
            y,
            renderer,
            interaction,
            scroll_states,
            result,
            anim_ctx,
            depth + 1,
        );
    }

    // Debug layout outlines — drawn last so they're on top of everything
    if DEBUG_LAYOUT.load(Ordering::Relaxed) && w > 0.0 && h > 0.0 {
        let color = DEBUG_COLORS[depth % DEBUG_COLORS.len()];
        renderer.stroke_rect(x, y, w, h, 1.0, color);
    }
}

fn bg_np_from_props(props: &PropsData) -> BgNinePatch {
    BgNinePatch {
        bitmap_id: props.bg_np_id,
        left: props.bg_np_left,
        top: props.bg_np_top,
        right: props.bg_np_right,
        bottom: props.bg_np_bottom,
    }
}

fn padding_uniform(v: f32) -> taffy::Rect<LengthPercentage> {
    taffy::Rect {
        top: length(v),
        right: length(v),
        bottom: length(v),
        left: length(v),
    }
}

fn margin_uniform(v: f32) -> taffy::Rect<LengthPercentageAuto> {
    taffy::Rect {
        top: LengthPercentageAuto::length(v),
        right: LengthPercentageAuto::length(v),
        bottom: LengthPercentageAuto::length(v),
        left: LengthPercentageAuto::length(v),
    }
}

fn size_from_props(props: &PropsData) -> Size<Dimension> {
    Size {
        width: if props.width == 0.0 {
            Dimension::auto()
        } else {
            length(props.width)
        },
        height: if props.height == 0.0 {
            Dimension::auto()
        } else {
            length(props.height)
        },
    }
}

fn max_size_from_props(props: &PropsData) -> Size<Dimension> {
    Size {
        width: if props.max_width == 0.0 {
            Dimension::auto()
        } else {
            length(props.max_width)
        },
        height: if props.max_height == 0.0 {
            Dimension::auto()
        } else {
            length(props.max_height)
        },
    }
}

/// Convert `PropsData` inset fields to Taffy `position` and `inset`.
/// Finite values become `LengthPercentageAuto::Length`, NAN becomes `Auto`.
fn inset_from_props(props: &PropsData) -> taffy::Rect<LengthPercentageAuto> {
    fn inset_val(v: f32) -> LengthPercentageAuto {
        if v.is_finite() {
            LengthPercentageAuto::length(v)
        } else {
            LengthPercentageAuto::auto()
        }
    }
    taffy::Rect {
        top: inset_val(props.inset_top),
        right: inset_val(props.inset_right),
        bottom: inset_val(props.inset_bottom),
        left: inset_val(props.inset_left),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_mode_partitions_nodes_without_overlap_or_gap() {
        for dynamic in [false, true] {
            assert!(
                EmitMode::All.emits(dynamic),
                "the default mode must emit everything"
            );
            assert_ne!(
                EmitMode::StaticOnly.emits(dynamic),
                EmitMode::DynamicOnly.emits(dynamic),
                "static and dynamic passes must not overlap or leave a gap"
            );
        }
        assert!(EmitMode::StaticOnly.emits(false));
        assert!(EmitMode::DynamicOnly.emits(true));
    }

    /// The tag must survive the trip into `NodeContext`, which is all the
    /// render walk ever sees of it.
    #[test]
    fn build_taffy_node_tags_context_with_the_partition_classification() {
        fn build(node: &TreeNode) -> (TaffyTree<NodeContext>, taffy::NodeId) {
            let mut taffy = TaffyTree::new();
            let mut result = TreeResult::default();
            let mut modals = Vec::new();
            let id = build_taffy_node(&mut taffy, node, 0, &mut result, &mut modals)
                .expect("BUG: taffy build failed");
            (taffy, id)
        }

        let static_canvas = TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![DrawCommand::Rect {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 4.0,
                fill: Fill::Solid(Color::from_rgb(1, 2, 3)),
            }],
        };
        let (taffy, id) = build(&static_canvas);
        assert!(
            !taffy
                .get_node_context(id)
                .expect("BUG: canvas must own a context")
                .dynamic,
            "a canvas of plain draws must be tagged static"
        );

        let dynamic_tree = TreeNode::Column(
            PropsData::default(),
            vec![TreeNode::RelTime {
                anchor: 0,
                format: RelTimeFormat {
                    length: RelTimeLength::Short,
                    segments: RelTimeSegments::Single,
                },
                clamp: RelTimeClamp::default(),
                style: TextStyle::default(),
            }],
        );
        let (taffy, id) = build(&dynamic_tree);
        assert!(
            taffy
                .get_node_context(id)
                .expect("BUG: every container must own a context to carry the tag")
                .dynamic,
            "a container holding a host-driven node must be tagged dynamic"
        );
    }

    fn span(text: &str) -> SpanData {
        SpanData {
            text: text.to_owned(),
            weight: None,
            color: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    fn word_texts(words: &[Vec<SpanData>]) -> Vec<Vec<&str>> {
        words
            .iter()
            .map(|w| w.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn min_content_words_join_across_span_boundaries() {
        // "hashrate" is one word split across two styled spans: probing its
        // fragments separately would under-report the min-content width.
        let words = split_min_content_words(&[span("current hash"), span("rate now")]);
        assert_eq!(
            word_texts(&words),
            [vec!["current"], vec!["hash", "rate"], vec!["now"],]
        );
    }

    #[test]
    fn min_content_words_treat_nbsp_as_glue() {
        // A line cannot break at U+00A0, so "10\u{A0}TH/s" must probe as one
        // word; regular spaces still split.
        let words = split_min_content_words(&[span("about 10\u{A0}TH/s total")]);
        assert_eq!(
            word_texts(&words),
            [vec!["about"], vec!["10\u{A0}TH/s"], vec!["total"]]
        );
    }

    #[test]
    fn min_content_words_drop_leading_and_trailing_whitespace() {
        let words = split_min_content_words(&[span("  spaced  out  ")]);
        assert_eq!(word_texts(&words), [vec!["spaced"], vec!["out"]]);
    }

    #[test]
    fn test_props_size() {
        // In-memory size may differ from wire SIZE due to alignment padding
        assert!(std::mem::size_of::<PropsData>() >= PropsData::SIZE);
    }

    #[test]
    fn read_draw_decodes_shadow() {
        // Wire layout: [DRAW_SHADOW][dx f32][dy f32][blur f32][color u32][inner draw]
        let mut bytes = vec![DRAW_SHADOW];
        bytes.extend_from_slice(&2.0_f32.to_le_bytes());
        bytes.extend_from_slice(&3.0_f32.to_le_bytes());
        bytes.extend_from_slice(&7.5_f32.to_le_bytes());
        bytes.extend_from_slice(&0x1122_3344_u32.to_le_bytes());
        // Inner: a plain rect — [DRAW_RECT][x][y][w][h][fill].
        bytes.push(DRAW_RECT);
        for v in [1.0_f32, 1.0, 4.0, 5.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bmc_wasm_protocol::encode_fill(&mut bytes, &Fill::Solid(Color::from_raw(0xFF00_FF00)));

        let draw = TreeReader::new(&bytes)
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_SHADOW");
        let DrawCommand::Shadow {
            dx,
            dy,
            blur,
            color,
            inner,
        } = draw
        else {
            panic!("expected DrawCommand::Shadow, got {draw:?}");
        };
        assert_eq!((dx, dy, blur), (2.0, 3.0, 7.5));
        assert_eq!(color, Color::from_raw(0x1122_3344));
        assert!(
            matches!(*inner, DrawCommand::Rect { .. }),
            "shadow inner should decode as the wrapped rect",
        );
    }

    #[test]
    fn read_draw_decodes_arc() {
        let red = Color::from_rgb(0xFF, 0x00, 0x00);
        let teal = Color::from_rgb(0x00, 0x80, 0x80);
        let expected_fill = ArcFill::gradient(red, teal);
        let expected_segments = ArcSegments::Explicit(vec![(0.0, 0.5), (0.6, 1.5)]);

        let mut data = vec![DRAW_ARC];
        for value in [1.0_f32, 2.0, 30.0, 0.0, 1.5, 6.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        bmc_wasm_protocol::encode_arc_fill(&mut data, &expected_fill);
        bmc_wasm_protocol::encode_arc_segments(&mut data, &expected_segments);
        bmc_wasm_protocol::encode_arc_cap(&mut data, ArcCap::Butt);

        let DrawCommand::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            segments,
            cap,
        } = TreeReader::new(&data)
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_ARC")
        else {
            panic!("BUG: expected DrawCommand::Arc");
        };

        assert_eq!((cx, cy, radius), (1.0, 2.0, 30.0));
        assert_eq!((start_angle, end_angle, width), (0.0, 1.5, 6.0));
        assert_eq!(fill, expected_fill);
        assert_eq!(segments, expected_segments);
        assert_eq!(cap, ArcCap::Butt);
    }

    #[test]
    fn curved_text_decodes_geometry_enums_style_and_text() {
        let style = TextStyle {
            size: 18,
            color: Color::from_rgb(9, 8, 7),
            ..TextStyle::default()
        };
        let mut data = vec![DRAW_CURVED_TEXT];
        for value in [1.0_f32, 2.0, 44.0, std::f32::consts::PI] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.push(u8::from(ArcAnchor::Center));
        data.push(u8::from(ArcTextFacing::Outward));
        data.extend_from_slice(&style.to_bytes());
        data.extend_from_slice(&5_u16.to_le_bytes());
        data.extend_from_slice(b"HELLO");

        let mut de = TreeReader::new(&data);
        let DrawCommand::CurvedText {
            cx,
            cy,
            radius,
            angle,
            anchor,
            facing,
            text,
            style: decoded_style,
        } = de
            .read_draw()
            .expect("BUG: test buffer encodes a valid DRAW_CURVED_TEXT")
        else {
            panic!("expected CurvedText");
        };

        assert_eq!(
            (
                cx.to_bits(),
                cy.to_bits(),
                radius.to_bits(),
                angle.to_bits()
            ),
            (
                1.0_f32.to_bits(),
                2.0_f32.to_bits(),
                44.0_f32.to_bits(),
                std::f32::consts::PI.to_bits(),
            ),
        );
        assert_eq!(anchor, ArcAnchor::Center);
        assert_eq!(facing, ArcTextFacing::Outward);
        assert_eq!(text, "HELLO");
        assert_eq!(decoded_style.color, style.color);
        assert_eq!(decoded_style.size, style.size);
    }
}

#[cfg(test)]
mod intrinsic_button_width_tests {
    use super::*;
    use crate::renderer::test_support::ShapingRecorder;

    #[test]
    fn a_button_sizes_itself_around_its_shaped_label() {
        const LABEL: &str = "Disconnect";
        let size = ButtonSize::Normal;

        let mut recorder = ShapingRecorder::default();
        let label_w = recorder.measure_text(LABEL, size.font_size());

        let mut taffy: TaffyTree<NodeContext> = TaffyTree::new();
        let button = taffy
            .new_leaf_with_context(
                Style::default(),
                NodeContext {
                    button: Some(ButtonContext {
                        id: "btn".to_owned(),
                        label: LABEL.to_owned(),
                        style: 0,
                        size: size as u8,
                        icon_id: None,
                        disabled: false,
                        skin: None,
                    }),
                    ..NodeContext::default()
                },
            )
            .expect("BUG: taffy must accept a leaf node");
        compute_taffy_layout(&mut taffy, button, &mut recorder)
            .expect("BUG: laying out a single leaf must succeed");

        let laid_out = taffy
            .layout(button)
            .expect("BUG: the laid-out leaf must have a layout");
        assert!(
            (laid_out.size.width - (label_w + size.h_padding() * 2.0)).abs() < 0.5,
            "a label-only button must be as wide as its shaped label plus padding, got {}",
            laid_out.size.width
        );
    }
}

#[cfg(test)]
mod frame_pass_tests {
    use std::collections::HashMap;

    use super::{LayerUse, ProcessContext, TreeNode, layout_and_render};
    use crate::FrameTimings;
    use crate::interaction::InteractionState;
    use crate::renderer::Renderer;
    use bmc_wasm_protocol::colors::Color;
    use bmc_wasm_protocol::{ArcCap, ArcFill, ArcSegments, BitmapId, Fill, MeshId, SvgId};
    use taffy::prelude::TaffyTree;

    use super::{MeshDrawArgs, PropsData, SpanData};
    use crate::tree::{ArcAnchor, ArcTextFacing, AutoFit, TextStyle};

    /// Records which key each static-layer call carried: one renderer serves
    /// every slot, so the key is all that keeps two slots' layers apart.
    #[derive(Default)]
    struct KeyRecordingRenderer {
        draws: Vec<crate::interaction::Rect>,
        calls: Vec<(&'static str, String)>,
        captured: Vec<String>,
        paragraphs: Vec<String>,
    }

    impl Renderer for KeyRecordingRenderer {
        fn begin_static_layer(&mut self, key: &str, _width: u32, _height: u32) -> bool {
            self.calls.push(("begin", key.to_owned()));
            true
        }

        fn end_static_layer(&mut self, key: &str) {
            self.calls.push(("end", key.to_owned()));
            self.captured.push(key.to_owned());
        }

        fn blit_static_layer(&mut self, key: &str) -> bool {
            self.calls.push(("blit", key.to_owned()));
            self.captured.iter().any(|k| k == key)
        }

        fn invalidate_static_layer(&mut self, key: &str) {
            self.calls.push(("invalidate", key.to_owned()));
            self.captured.retain(|k| k != key);
        }

        fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, _color: Color) {
            self.draws.push(crate::interaction::Rect::new(x, y, w, h));
        }

        fn fill_rounded_rect(
            &mut self,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            _radius: f32,
            _color: Color,
        ) {
            self.draws.push(crate::interaction::Rect::new(x, y, w, h));
        }

        fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, _color: Color) {
            self.draws.push(crate::interaction::Rect::new(
                cx - r,
                cy - r,
                2.0 * r,
                2.0 * r,
            ));
        }

        fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, _fill: &Fill) {
            self.draws.push(crate::interaction::Rect::new(x, y, w, h));
        }

        fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, _fill: &Fill) {
            self.draws.push(crate::interaction::Rect::new(
                cx - r,
                cy - r,
                2.0 * r,
                2.0 * r,
            ));
        }

        fn stroke_arc(
            &mut self,
            _cx: f32,
            _cy: f32,
            _radius: f32,
            _start_angle: f32,
            _end_angle: f32,
            _width: f32,
            _fill: &ArcFill,
            _segments: &ArcSegments,
            _cap: ArcCap,
        ) {
        }

        fn stroke_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _border_width: f32,
            _color: Color,
        ) {
        }

        fn draw_line(
            &mut self,
            _x1: f32,
            _y1: f32,
            _x2: f32,
            _y2: f32,
            _width: f32,
            _color: Color,
        ) {
        }

        fn save(&mut self) {}

        fn restore(&mut self) {}

        fn translate(&mut self, _x: f32, _y: f32) {}

        fn rotate(&mut self, _angle_radians: f32) {}

        fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

        fn pop_scissor(&mut self) {}

        fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: Color) {}

        fn measure_text(&mut self, _text: &str, _size: f32) -> f32 {
            0.0
        }

        fn measure_paragraph(
            &mut self,
            _style: &TextStyle,
            _spans: &[SpanData],
            _max_width: Option<f32>,
        ) -> (f32, f32) {
            (0.0, 0.0)
        }

        fn draw_paragraph(
            &mut self,
            _style: &TextStyle,
            spans: &[SpanData],
            _x: f32,
            _y: f32,
            _max_width: f32,
        ) {
            self.paragraphs
                .extend(spans.iter().map(|span| span.text.clone()));
        }

        fn draw_paragraph_clipped(
            &mut self,
            _style: &TextStyle,
            _spans: &[SpanData],
            _x: f32,
            _y: f32,
            _max_width: f32,
            _clip_top: f32,
            _clip_bottom: f32,
        ) {
        }

        fn register_svg(&mut self, _tag: &str, _data: &[u8]) -> Option<SvgId> {
            None
        }

        fn draw_svg(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _color: Color,
            _icon_id: SvgId,
            _anti_alias: bool,
            _fills: &[(String, Color)],
        ) {
        }

        fn register_bitmap(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
            None
        }

        fn register_bitmap_nearest(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
            None
        }

        fn register_bitmap_rgba(
            &mut self,
            _tag: &str,
            _rgba: &[u8],
            _width: u32,
            _height: u32,
        ) -> Option<BitmapId> {
            None
        }

        fn draw_bitmap(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _bitmap_id: BitmapId) {}

        fn draw_nine_patch(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _bitmap_id: BitmapId,
            _left: u16,
            _top: u16,
            _right: u16,
            _bottom: u16,
        ) {
        }

        fn stroke_rounded_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _radius: f32,
            _border_width: f32,
            _color: Color,
        ) {
        }

        fn register_bitmap_rgba_nearest(
            &mut self,
            _tag: &str,
            _rgba: &[u8],
            _width: u32,
            _height: u32,
        ) -> Option<BitmapId> {
            None
        }

        fn register_mesh(&mut self, _tag: &str, _data: &[u8]) -> Option<MeshId> {
            None
        }

        fn draw_mesh(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _slot_index: u8,
            _mesh_id: MeshId,
            _args: MeshDrawArgs,
        ) {
        }

        fn draw_sphere(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _bitmap_id: BitmapId,
            _center_lat: f32,
            _center_lon: f32,
            _zoom: f32,
            _light_lat: f32,
            _light_lon: f32,
            _atmosphere: bool,
        ) {
        }

        fn draw_canvas_text(&mut self, _text: &str, _x: f32, _y: f32, _style: &TextStyle) {}

        fn draw_autofit_text(
            &mut self,
            _x: f32,
            _y: f32,
            _box_width: f32,
            _box_height: f32,
            _text: &str,
            _style: &TextStyle,
            _mode: AutoFit,
            _min_size: u16,
            _max_size: u16,
        ) {
        }

        fn draw_curved_text(
            &mut self,
            _cx: f32,
            _cy: f32,
            _radius: f32,
            _angle: f32,
            _anchor: ArcAnchor,
            _facing: ArcTextFacing,
            _text: &str,
            _style: &TextStyle,
        ) {
        }

        fn stroke_path(
            &mut self,
            _points: &[(f32, f32)],
            _stroke_width: f32,
            _color: Color,
            _closed: bool,
            _smooth: bool,
        ) {
        }

        fn fill_path_paint(&mut self, _points: &[(f32, f32)], _fill: &Fill, _smooth: bool) {}

        fn drop_shadow(
            &mut self,
            _cx: f32,
            _cy: f32,
            _fbo_w: u32,
            _fbo_h: u32,
            _dx: f32,
            _dy: f32,
            _blur: f32,
            _color: Color,
            _inner: &mut dyn FnMut(&mut dyn Renderer),
        ) {
        }

        fn begin_frame(&mut self, _width: u32, _height: u32, _dpi_scale: f32) {}

        fn flush(&mut self) {}

        fn width(&self) -> f32 {
            0.0
        }

        fn height(&self) -> f32 {
            0.0
        }

        fn evict_prefix(&mut self, _prefix: &str) -> usize {
            0
        }

        fn bitmap_resident_bytes(&self) -> u64 {
            0
        }
    }

    struct SlotState {
        interaction: InteractionState,
        modal_states: HashMap<String, crate::ModalState>,
        scroll_states: HashMap<String, crate::ScrollState>,
        animation_states: HashMap<u64, crate::AnimationState>,
        transition_states: HashMap<crate::TransitionStateKey, crate::TransitionState>,
        taffy: TaffyTree<super::NodeContext>,
    }

    impl Default for SlotState {
        fn default() -> Self {
            Self {
                interaction: InteractionState::new(),
                modal_states: HashMap::new(),
                scroll_states: HashMap::new(),
                animation_states: HashMap::new(),
                transition_states: HashMap::new(),
                taffy: TaffyTree::new(),
            }
        }
    }

    fn capture_frame(state: &mut SlotState, renderer: &mut KeyRecordingRenderer, key: &str) {
        let mut ctx = ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter: 1,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: super::EmitMode::All,
            static_layer: LayerUse::Capture,
            static_layer_key: key,
            damage_rects: &[],
        };
        let tree = TreeNode::Column(PropsData::default(), vec![TreeNode::Spacer { flex: 1.0 }]);
        let mut timings = FrameTimings::default();
        layout_and_render(&tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
            .expect("BUG: layout_and_render must succeed for a trivial tree");
    }

    /// An open modal whose body is a single static paragraph.
    fn modal_tree_with_body_text(text: &str) -> TreeNode {
        let TreeNode::Column(props, mut children) = modal_tree(true) else {
            unreachable!("BUG: modal_tree builds a Column")
        };
        if let Some(TreeNode::Modal { body, .. }) = children.last_mut() {
            *body = vec![TreeNode::Paragraph {
                props: PropsData::default(),
                base_style: TextStyle::default(),
                spans: vec![SpanData {
                    text: text.to_owned(),
                    weight: None,
                    color: None,
                    italic: false,
                    underline: false,
                    strikethrough: false,
                }],
            }];
        }
        TreeNode::Column(props, children)
    }

    fn modal_tree(is_open: bool) -> TreeNode {
        TreeNode::Column(
            PropsData::default(),
            vec![
                TreeNode::Spacer { flex: 1.0 },
                TreeNode::Modal {
                    modal_id: "confirm".to_owned(),
                    is_open,
                    padding: 8,
                    backdrop_alpha: 160,
                    title: String::new(),
                    content_height: 40.0,
                    bg_color: Color::default(),
                    header_color: Color::default(),
                    title_color: Color::default(),
                    max_width: 0,
                    body: Vec::new(),
                    footer_primary_key: String::new(),
                    footer_primary_label: String::new(),
                    footer_secondary_key: String::new(),
                    footer_secondary_label: String::new(),
                    footer_danger: false,
                },
            ],
        )
    }

    /// A modal that scrolls, with enough children to overflow its body.
    fn scrollable_modal_tree() -> TreeNode {
        let TreeNode::Column(props, mut children) = modal_tree(true) else {
            unreachable!("BUG: modal_tree builds a Column")
        };
        if let Some(TreeNode::Modal { body, .. }) = children.last_mut() {
            *body = (0..4)
                .map(|_| TreeNode::Canvas {
                    props: PropsData {
                        width: 60.0,
                        height: 60.0,
                        ..PropsData::default()
                    },
                    touch_key: None,
                    draws: Vec::new(),
                })
                .collect();
        }
        TreeNode::Column(props, children)
    }

    /// The modal overlays are walked after `render_tree` returns, so anything
    /// that spends the frame's drag inside that walk strands them: a capture
    /// frame is the *normal* case while a modal drags, because a non-zero
    /// global delta forces the layer to recapture.
    #[test]
    fn a_modal_body_scrolls_on_a_capture_frame() {
        use crate::interaction::TouchEvent;

        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();

        // Let the modal's entrance animation settle, so its body is on screen
        // and the drag below can land in it.
        for _ in 0..40 {
            capture_once(&mut state, &mut renderer, &scrollable_modal_tree());
        }

        state
            .interaction
            .push_event(TouchEvent::Down { x: 50.0, y: 50.0 });
        state
            .interaction
            .push_event(TouchEvent::Move { x: 50.0, y: 30.0 });
        state.interaction.begin_frame();
        capture_once(&mut state, &mut renderer, &scrollable_modal_tree());

        let offset = state
            .scroll_states
            .get("confirm::body")
            .map_or(0.0, |s| s.scroll_offset);
        assert!(
            offset > 0.0,
            "the modal's body must take the drag the main tree's passes did not, got {offset}"
        );
    }

    /// A modal's body is not in the cached layer, so a layer-reusing frame must
    /// still emit it in full: inheriting `DynamicOnly` from the blit drops every
    /// static node inside the modal while the modal itself keeps painting.
    #[test]
    fn an_open_modal_emits_its_static_body_on_a_layer_reusing_frame() {
        let mut state = SlotState::default();
        let mut renderer = KeyRecordingRenderer::default();
        let tree = modal_tree_with_body_text("are you sure?");

        // First frame captures the layer, so the second can reuse it.
        render_with(&mut state, &mut renderer, &tree, LayerUse::Capture);
        renderer.paragraphs.clear();
        render_with(&mut state, &mut renderer, &tree, LayerUse::Reuse);

        assert!(
            renderer.calls.iter().any(|(call, _)| *call == "blit"),
            "the second frame must reuse the layer, or this proves nothing",
        );
        assert!(
            renderer.paragraphs.iter().any(|t| t == "are you sure?"),
            "the modal's body text must be painted on a layer-reusing frame; \
             got {:?}",
            renderer.paragraphs,
        );
    }

    fn render_with(
        state: &mut SlotState,
        renderer: &mut KeyRecordingRenderer,
        tree: &TreeNode,
        static_layer: LayerUse,
    ) {
        let mut ctx = ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter: 1,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: super::EmitMode::All,
            static_layer,
            static_layer_key: "modal-test",
            damage_rects: &[],
        };
        let mut timings = FrameTimings::default();
        layout_and_render(tree, 200.0, 200.0, renderer, &mut timings, &mut ctx)
            .expect("BUG: layout_and_render must succeed for the modal tree");
    }

    /// The cached-frame pass: dynamic draws only, scissored to `damage`.
    fn render_dynamic_pass(
        state: &mut SlotState,
        renderer: &mut KeyRecordingRenderer,
        tree: &TreeNode,
        damage: &[crate::interaction::Rect],
    ) {
        let mut ctx = ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter: 3,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: super::EmitMode::DynamicOnly,
            static_layer: LayerUse::Reuse,
            static_layer_key: "damage-test",
            damage_rects: damage,
        };
        let mut timings = FrameTimings::default();
        layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
            .expect("BUG: layout_and_render must succeed for the dynamic pass");
    }

    fn render_once(
        state: &mut SlotState,
        renderer: &mut KeyRecordingRenderer,
        tree: &TreeNode,
    ) -> super::TreeResult {
        let mut ctx = ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter: 1,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: super::EmitMode::All,
            static_layer: LayerUse::Ignore,
            static_layer_key: "modal-test",
            damage_rects: &[],
        };
        let mut timings = FrameTimings::default();
        let (result, _) = layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
            .expect("BUG: layout_and_render must succeed for a modal tree");
        result
    }

    /// `render_once` with the static layer live, so the frame splits into
    /// a capture pass and a dynamic pass — every scrolling frame's shape.
    fn capture_once(
        state: &mut SlotState,
        renderer: &mut KeyRecordingRenderer,
        tree: &TreeNode,
    ) -> super::TreeResult {
        let mut ctx = ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter: 1,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: super::EmitMode::All,
            static_layer: LayerUse::Capture,
            static_layer_key: "scroll-test",
            damage_rects: &[],
        };
        let mut timings = FrameTimings::default();
        let (result, _) = layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
            .expect("BUG: layout_and_render must succeed for a scroll tree");
        result
    }

    /// A scroll container holding static content, so the frame both
    /// splits and scrolls — 200 px of children in a 100 px viewport.
    fn scrolling_tree() -> TreeNode {
        let child = |y: f32| TreeNode::Canvas {
            props: PropsData {
                width: 100.0,
                height: 100.0,
                ..PropsData::default()
            },
            touch_key: None,
            draws: vec![super::DrawCommand::Rect {
                x: 0.0,
                y,
                w: 100.0,
                h: 100.0,
                fill: Fill::Solid(Color::from_rgb(9, 9, 9)),
            }],
        };
        TreeNode::Scroll {
            scroll_key: "list".to_owned(),
            props: PropsData::default(),
            children: vec![child(0.0), child(100.0)],
        }
    }

    /// A capture frame walks the tree twice. The scroll offset advances inside
    /// that walk, so without a gate the second pass adds the same drag again:
    /// content runs at twice the finger and the captured half lags a delta
    /// behind the dynamic one for the whole gesture.
    #[test]
    fn a_capture_frame_applies_a_scroll_drag_once() {
        use crate::interaction::TouchEvent;

        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();

        // First frame registers the scroll region the drag hit-tests against.
        capture_once(&mut state, &mut renderer, &scrolling_tree());

        state
            .interaction
            .push_event(TouchEvent::Down { x: 50.0, y: 50.0 });
        state
            .interaction
            .push_event(TouchEvent::Move { x: 50.0, y: 30.0 });
        // The host drains the queue here, hit-testing against the regions
        // that the frame above registered.
        state.interaction.begin_frame();
        capture_once(&mut state, &mut renderer, &scrolling_tree());

        let offset = state
            .scroll_states
            .get("list")
            .expect("BUG: the scroll container must record state")
            .scroll_offset;
        assert!(
            (offset - 20.0).abs() < f32::EPSILON,
            "a 20 px drag must move the content 20 px, got {offset}"
        );
    }

    /// A skeleton's bar is a distinctive height, so it can be counted among
    /// the other draws a frame makes.
    const SKELETON_BAR_H: f32 = 7.0;

    fn skeleton_beside_animation() -> TreeNode {
        TreeNode::Column(
            PropsData::default(),
            vec![
                moving_dot(0.0),
                TreeNode::Skeleton(crate::components::skeleton::SkeletonData {
                    kind: bmc_wasm_protocol::SkeletonKind::Placeholder,
                    chars: 0.0,
                    font_size: 0.0,
                    width: 40.0,
                    height: SKELETON_BAR_H,
                    color: Color::from_rgb(9, 9, 9),
                }),
            ],
        )
    }

    /// Every paint site is gated on the emission mode. An ungated one paints
    /// into the layer on the capture pass, then again over the blit on the
    /// dynamic pass — and being static, nothing repaints underneath, so each
    /// frame blends another copy onto the last. Translucent fills darken,
    /// and edges thicken over a few seconds of cached frames.
    #[test]
    fn a_skeleton_paints_once_on_a_capture_frame() {
        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();

        capture_once(&mut state, &mut renderer, &skeleton_beside_animation());

        let bars = renderer
            .draws
            .iter()
            .filter(|r| (r.h - SKELETON_BAR_H).abs() < f32::EPSILON)
            .count();
        assert_eq!(
            bars, 1,
            "the capture pass paints the bar into the layer; the dynamic pass \
             must not paint it again, got {:?}",
            renderer.draws
        );
    }

    /// A canvas whose one draw animates, so the walk tags it dynamic and the
    /// cached-frame pass repaints it.
    fn moving_dot(x: f32) -> TreeNode {
        use bmc_wasm_protocol::animation::{AnimProperty, ColorSpace, Easing, LoopMode};

        TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: None,
            draws: vec![super::DrawCommand::Modified {
                animations: vec![super::HostAnimationDef {
                    property: AnimProperty::Alpha,
                    from: 0.0,
                    to: 1.0,
                    duration_ms: 500,
                    delay_ms: 0,
                    easing: Easing::Linear,
                    loop_mode: LoopMode::Forever,
                }],
                transition: None,
                color_space: ColorSpace::default(),
                inner: Box::new(super::DrawCommand::Rect {
                    x,
                    y: 10.0,
                    w: 12.0,
                    h: 12.0,
                    fill: Fill::Solid(Color::from_rgb(9, 9, 9)),
                }),
            }],
        }
    }

    fn within(outer: &[crate::interaction::Rect], inner: crate::interaction::Rect) -> bool {
        outer.iter().any(|o| {
            o.x <= inner.x
                && o.y <= inner.y
                && o.x + o.w >= inner.x + inner.w
                && o.y + o.h >= inner.y + inner.h
        })
    }

    /// The contract a damage-scissored frame rests on: everything the dynamic
    /// pass paints falls inside the reported damage, or the scissor drops a
    /// pixel that then never lands.
    ///
    /// The span is two walks wide because the device rotates export buffers:
    /// this frame paints into the buffer the frame *before* last drew, so the
    /// region that one moved is still stale in it.
    #[test]
    fn every_dynamic_draw_lands_inside_the_two_walk_damage_span() {
        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();

        let older = render_once(&mut state, &mut renderer, &moving_dot(10.0));
        let newer = render_once(&mut state, &mut renderer, &moving_dot(60.0));

        let span: Vec<_> = newer
            .dynamic_rects
            .iter()
            .chain(older.dynamic_rects.iter())
            .copied()
            .collect();
        assert!(
            !span.is_empty(),
            "an animating draw must report damage, or a cached frame repaints nothing"
        );

        renderer.draws.clear();
        render_dynamic_pass(&mut state, &mut renderer, &moving_dot(60.0), &span);

        assert!(
            !renderer.draws.is_empty(),
            "the dynamic pass must paint the animating draw, or this proves nothing"
        );
        for draw in &renderer.draws {
            assert!(
                within(&span, *draw),
                "draw {draw:?} falls outside the damage span {span:?}, so the scissor drops it"
            );
        }
    }

    /// The backdrop dims the whole surface from outside the walk, so a frame
    /// scissored to the walk's rects darkens only those.
    #[test]
    fn an_open_modal_damages_the_whole_surface() {
        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();

        let result = render_once(&mut state, &mut renderer, &modal_tree(true));

        assert!(
            result
                .dynamic_rects
                .iter()
                .any(|r| r.w >= 100.0 && r.h >= 100.0),
            "an open modal must damage the surface, got {:?}",
            result.dynamic_rects
        );
    }

    #[test]
    fn a_closed_modal_damages_nothing() {
        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();

        let result = render_once(&mut state, &mut renderer, &modal_tree(false));

        assert!(
            result.dynamic_rects.is_empty(),
            "a modal that never opened paints nothing, got {:?}",
            result.dynamic_rects
        );
    }

    /// Two slots sharing one renderer must address distinct layers, or each
    /// blits whatever the other just captured.
    #[test]
    fn each_slot_captures_under_its_own_key() {
        let mut renderer = KeyRecordingRenderer::default();
        let (mut a, mut b) = (SlotState::default(), SlotState::default());

        for (slot, key) in [(0, "widget-a"), (1, "widget-b"), (0, "widget-a")] {
            renderer.calls.clear();
            let state = if slot == 0 { &mut a } else { &mut b };
            capture_frame(state, &mut renderer, key);

            assert!(
                !renderer.calls.is_empty(),
                "slot '{key}' made no static-layer calls, so this asserts nothing"
            );
            for (op, seen) in &renderer.calls {
                assert_eq!(
                    seen, key,
                    "'{op}' addressed layer '{seen}' while rendering slot '{key}'"
                );
            }
        }
    }
}

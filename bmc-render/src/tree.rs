// Copyright (C) 2026  Braiins Systems s.r.o.

//! Tree deserialization and layout computation.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
#![allow(clippy::wildcard_imports)]

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use bmc_wasm_protocol::*;

use crate::gpu::mesh::{MeshDrawArgs, MeshHighlight, MeshLighting, MeshTransform};

/// When `DEBUG_LAYOUT=1` env var is set, draw colored outlines around every layout node.
static DEBUG_LAYOUT: AtomicBool = AtomicBool::new(false);

/// Call once at startup to check the `DEBUG_LAYOUT` env var.
pub fn init_debug_flags() {
    if std::env::var("DEBUG_LAYOUT").is_ok_and(|v| v == "1") {
        DEBUG_LAYOUT.store(true, Ordering::Relaxed);
    }
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
pub use bmc_wasm_protocol::{CrossAlign, PropsData, TextAlign, TextOverflow, TextStyle};

/// A text span with style overrides
#[derive(Clone, Debug)]
pub struct SpanData {
    pub text: String,
    pub weight: Option<u16>,
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
#[derive(Debug, Clone)]
pub struct HostTransitionDef {
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
        color: Color,
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
        color: Color,
    },
    Svg {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        icon_id: Option<SvgId>,
        anti_alias: bool,
    },
    Bitmap {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
    },
    Rotated {
        angle: f32,
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
        color: Color,
        stroke_width: f32,
        closed: bool,
        fill: bool,
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
        /// 0 = Fraction, 1 = Indeterminate
        mode: u8,
        fraction: f32,
        active: bool,
        fill_color: Color,
        track_color: Color,
        bg_color: Color,
        skin: Option<SliderSkinData>,
    },
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

    fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
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
        if self.pos + PropsData::SIZE > self.data.len() {
            bail!("unexpected end of tree data reading props");
        }
        let props = PropsData::from_bytes(&self.data[self.pos..self.pos + PropsData::SIZE]);
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
        if self.pos + TextStyle::SIZE > self.data.len() {
            bail!("unexpected end of tree data reading text style");
        }
        let style = TextStyle::from_bytes(&self.data[self.pos..self.pos + TextStyle::SIZE]);
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
            Some(flags & 0xFFF)
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
            NODE_PROGRESS_BAR => {
                let key_len = self.read_u16()?;
                let touch_key = if key_len > 0 {
                    Some(self.read_string(key_len)?)
                } else {
                    None
                };
                let track_h = self.read_f32()?;
                let mode = self.read_u8()?;
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
                let w = self.read_f32()?;
                let h = self.read_f32()?;
                let color = Color::from_raw(self.read_u32()?);
                Ok(DrawCommand::Rect { x, y, w, h, color })
            }
            DRAW_CIRCLE => {
                let cx = self.read_f32()?;
                let cy = self.read_f32()?;
                let r = self.read_f32()?;
                let color = Color::from_raw(self.read_u32()?);
                Ok(DrawCommand::Circle { cx, cy, r, color })
            }
            DRAW_ICON => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_f32()?;
                let h = self.read_f32()?;
                let color = Color::from_raw(self.read_u32()?);
                let icon_id = self.read_icon_id()?;
                let anti_alias = self.read_u8()? != 0;
                Ok(DrawCommand::Svg {
                    x,
                    y,
                    w,
                    h,
                    color,
                    icon_id,
                    anti_alias,
                })
            }
            DRAW_BITMAP => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_f32()?;
                let h = self.read_f32()?;
                let bitmap_id = self.read_bitmap_id()?;
                Ok(DrawCommand::Bitmap {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                })
            }
            DRAW_CENTERED => {
                let inner = self.read_draw()?;
                Ok(DrawCommand::Centered {
                    inner: Box::new(inner),
                })
            }
            DRAW_ORBIT => {
                let radius = self.read_f32()?;
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
                    let duration_ms = self.read_u32()?;
                    let easing = Easing::from_u8(self.read_u8()?)
                        .ok_or_else(|| anyhow::anyhow!("invalid Easing for transition"))?;
                    Some(HostTransitionDef {
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
                let point_count = self.read_u16()? as usize;
                let mut points = Vec::with_capacity(point_count);
                for _ in 0..point_count {
                    let x = self.read_f32()?;
                    let y = self.read_f32()?;
                    points.push((x, y));
                }
                let color = Color::from_raw(self.read_u32()?);
                let stroke_width = if fill { 0.0 } else { self.read_f32()? };
                Ok(DrawCommand::Path {
                    points,
                    color,
                    stroke_width,
                    closed,
                    fill,
                    smooth,
                })
            }
            DRAW_SPHERE => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_f32()?;
                let h = self.read_f32()?;
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
                let w = self.read_f32()?;
                let h = self.read_f32()?;
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
            DRAW_NINE_PATCH => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_f32()?;
                let h = self.read_f32()?;
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
            _ => bail!("unknown draw command: {draw_type}"),
        }
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

use std::collections::HashMap;
use std::time::Instant;

use taffy::prelude::*;
use taffy::{Overflow, Point};

use crate::components::draw::render_draw_command;
use crate::components::modal::{ModalInfo, render_modal};
use crate::components::notification::{
    NotificationData, measure_notification, render_notification,
};
use crate::components::progress_bar::{ProgressBarData, render_progress_bar};
use crate::components::{ButtonSize, ButtonStyle, draw_button};
use crate::interaction::InteractionState;
use crate::renderer::Renderer;
use crate::{AnimationState, FrameTimings, ModalState, ScrollState, TransitionState};

/// Mutable animation context threaded through the render pipeline.
pub(crate) struct AnimationContext<'a> {
    pub(crate) animation_states: &'a mut HashMap<u64, AnimationState>,
    pub(crate) transition_states: &'a mut HashMap<(u16, u16), TransitionState>,
    pub(crate) delta_ms: u32,
    pub(crate) frame_counter: u64,
    pub(crate) draw_counter: u32,
    pub(crate) canvas_index: u16,
    pub(crate) draw_in_canvas: u16,
    /// Monotonic counter for mesh atlas slot allocation (one per `draw_mesh` call).
    pub(crate) mesh_slot_counter: u8,
    /// Set to true when any animation or transition is in progress.
    pub(crate) has_active: bool,
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
    #[expect(dead_code)] // used when touch-pressed state rendering is added
    pub(crate) thumb_pressed_id: Option<BitmapId>,
}

/// Node data attached to taffy nodes
#[derive(Clone, Default, Debug)]
pub struct NodeContext {
    background: Color,
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
    pub transition_states: &'a mut HashMap<(u16, u16), TransitionState>,
    pub taffy: &'a mut TaffyTree<NodeContext>,
    pub frame_counter: u64,
    pub delta_ms: u32,
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
    let t0 = Instant::now();
    let tree_node = deserialize_tree(data)?;
    timings.deserialize_us = t0.elapsed().as_micros() as u32;

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
    // Phase 2: Build Taffy tree + compute layout
    let t1 = Instant::now();

    let mut result = TreeResult::default();
    let mut modals: Vec<ModalInfo> = Vec::new();

    // Reuse taffy tree — clear nodes but keep internal allocations
    ctx.taffy.clear();
    let root_id = build_taffy_node(ctx.taffy, tree_node, &mut result, &mut modals)?;

    // Override root size per-axis. Zero means "keep the node's own style"
    // (content-sized). The storybook passes (panel_width, 0) so children
    // can flex horizontally but the root doesn't stretch vertically.
    if let Ok(style) = ctx.taffy.style(root_id) {
        let mut new_style = style.clone();
        if width > 0.0 {
            new_style.size.width = length(width);
        }
        if height > 0.0 {
            new_style.size.height = length(height);
        }
        if width > 0.0 || height > 0.0 {
            ctx.taffy.set_style(root_id, new_style)?;
        }
    }

    compute_taffy_layout(ctx.taffy, root_id, renderer)?;

    // Extract actual content extent from the root's layout.
    if let Ok(root_layout) = ctx.taffy.layout(root_id) {
        result.content_size = (
            root_layout.content_size.width,
            root_layout.content_size.height,
        );
    }

    timings.layout_us = t1.elapsed().as_micros() as u32;

    // Phase 3: Render
    let t2 = Instant::now();

    let mut anim_ctx = AnimationContext {
        animation_states: ctx.animation_states,
        transition_states: ctx.transition_states,
        delta_ms: ctx.delta_ms,
        frame_counter: ctx.frame_counter,
        draw_counter: 0,
        canvas_index: 0,
        draw_in_canvas: 0,
        mesh_slot_counter: 0,
        has_active: false,
    };

    // Render main tree
    render_taffy_node(
        ctx.taffy,
        root_id,
        0.0,
        0.0,
        renderer,
        ctx.interaction,
        ctx.scroll_states,
        &mut result,
        &mut anim_ctx,
        0,
    );

    // Render modal overlays
    for modal in &modals {
        render_modal(
            modal,
            width,
            height,
            renderer,
            ctx.interaction,
            ctx.modal_states,
            ctx.scroll_states,
            ctx.delta_ms,
            &mut result,
            &mut anim_ctx,
            ctx.taffy,
        );
    }

    // GC: remove animation/transition states not seen this frame
    anim_ctx
        .animation_states
        .retain(|_, s| s.last_seen_frame >= ctx.frame_counter);
    anim_ctx
        .transition_states
        .retain(|_, s| s.last_seen_frame >= ctx.frame_counter);

    timings.render_us = t2.elapsed().as_micros() as u32;

    // A modal is animating whenever its progress hasn't yet caught up to its
    // target state (open=1.0, closed=0.0). Progress alone is ambiguous —
    // e.g. progress==0.0 means "fully closed" only if `is_open` is also false;
    // if `is_open` is true, it means the modal *just* started opening and
    // needs more frames to animate up to 1.0.
    let modal_animating = ctx.modal_states.values().any(|s| {
        if s.is_open {
            s.animation_progress < 1.0
        } else {
            s.animation_progress > 0.0
        }
    });

    Ok((result, anim_ctx.has_active || modal_animating))
}

#[expect(clippy::too_many_lines, clippy::only_used_in_recursion)]
pub(crate) fn build_taffy_node(
    taffy: &mut TaffyTree<NodeContext>,
    node: &TreeNode,
    result: &mut TreeResult,
    modals: &mut Vec<ModalInfo>,
) -> Result<taffy::NodeId> {
    match node {
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, result, modals))
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
                    None
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
                    length(0.0)
                } else {
                    Dimension::auto()
                },
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            if props.background != Color::default() || props.bg_np_id.is_some() {
                taffy.set_node_context(
                    id,
                    Some(NodeContext {
                        background: props.background,
                        bg_nine_patch: bg_np_from_props(props),
                        ..Default::default()
                    }),
                )?;
            }
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

        TreeNode::Button {
            id: btn_id,
            label,
            style: btn_style,
            size: btn_size,
            icon_id,
            disabled,
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
                align_self: Some(AlignSelf::FlexStart),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
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
                .map(|c| build_taffy_node(taffy, c, result, modals))
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
                    let available_width = known_dimensions.width.or(match available_space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        AvailableSpace::MinContent => Some(0.0),
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
    renderer: &mut dyn Renderer,
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

    if let Some(ctx) = taffy.get_node_context(node_id) {
        if let Some(bitmap_id) = ctx.bg_nine_patch.bitmap_id {
            let np = &ctx.bg_nine_patch;
            renderer.draw_nine_patch(x, y, w, h, bitmap_id, np.left, np.top, np.right, np.bottom);
        } else if ctx.background != Color::default() {
            renderer.fill_rect(x, y, w, h, ctx.background);
        }

        if let Some(ref para) = ctx.paragraph {
            renderer.draw_paragraph(&para.base_style, &para.spans, x, y, w);
        }

        if let Some(ref btn) = ctx.button {
            let (clicked, click_pos) = draw_button(
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
            );
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
            for draw in &ctx.draws {
                render_draw_command(renderer, draw, x, y, w, h, anim_ctx);
                anim_ctx.draw_in_canvas += 1;
            }
            anim_ctx.canvas_index += 1;
            renderer.pop_scissor();
        }

        // Progress bar: host-rendered track + fill + squiggle + dot
        if let Some(ref pb) = ctx.progress_bar {
            renderer.push_scissor(x, y, w, h);
            let has_active = render_progress_bar(renderer, pb, x, y, w, h, anim_ctx);
            if has_active {
                anim_ctx.has_active = true;
            }
            renderer.pop_scissor();
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

    if let Some(ctx) = taffy.get_node_context(node_id)
        && let Some(ref notif) = ctx.notification
    {
        render_notification(notif, x, y, w, h, renderer);
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
    fn test_props_size() {
        // In-memory size may differ from wire SIZE due to alignment padding
        assert!(std::mem::size_of::<PropsData>() >= PropsData::SIZE);
    }
}

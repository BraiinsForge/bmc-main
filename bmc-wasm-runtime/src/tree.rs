// Copyright (C) 2026  Braiins Systems s.r.o.

//! Tree deserialization and layout computation.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
#![allow(clippy::wildcard_imports)]

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use bmc_wasm_protocol::*;

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
const DEBUG_COLORS: [u32; 6] = [
    0xFF_00_FF_FF, // magenta
    0x00_FF_FF_FF, // cyan
    0xFF_FF_00_FF, // yellow
    0xFF_00_00_FF, // red
    0x00_FF_00_FF, // green
    0xFF_80_00_FF, // orange
];

// Re-export for other modules
pub use bmc_wasm_protocol::{CrossAlign, PropsData, TextAlign, TextOverflow, TextStyle};

/// A text span with style overrides
#[derive(Clone, Debug)]
pub struct SpanData {
    pub text: String,
    pub weight: Option<u16>,
    pub color: Option<u32>,
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
        color: u32,
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
        color: u32,
    },
    Icon {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
        icon_id: u16,
        anti_alias: bool,
    },
    Bitmap {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: u16,
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
        color: u32,
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
        bitmap_id: u16,
        atmosphere: bool,
        center_lat: f32,
        center_lon: f32,
        zoom: f32,
        light_lat: f32,
        light_lon: f32,
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
        bitmap_id: u16,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    },
}

/// 9-patch inset data (deserialized from wire format).
#[derive(Debug, Clone, Copy)]
pub struct NinePatchData {
    pub bitmap_id: u16,
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
    pub text_color: u32,
    pub pressed_text_color: u32,
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
        label: String,
        style: u8,
        size: u8,
        icon_id: u16,
        disabled: bool,
        skin: Option<ButtonSkinData>,
    },
    Spacer {
        flex: f32,
    },
    /// Canvas with optional touch interaction key.
    /// When `touch_key` is `Some`, the canvas registers a hit region and reports
    /// click position via `TreeResult::touch_clicks`.
    Canvas {
        props: PropsData,
        touch_key: Option<String>,
        draws: Vec<DrawCommand>,
    },
    /// Scrollable container
    Scroll {
        scroll_id: u16,
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
        modal_id: u16,
        is_open: bool,
        padding: u16,
        backdrop_alpha: u8,
        title: String,
        content_height: f32,
        /// Modal body background color. `0` = default.
        bg_color: u32,
        /// Header background color. `0` = default.
        header_color: u32,
        /// Title text color. `0` = default.
        title_color: u32,
        body: Vec<TreeNode>,
    },
    /// Host-rendered progress bar (seek/volume slider)
    ProgressBar {
        touch_key: Option<String>,
        track_h: f32,
        /// 0 = Fraction, 1 = Indeterminate
        mode: u8,
        fraction: f32,
        active: bool,
        fill_color: u32,
        track_color: u32,
        bg_color: u32,
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

    fn read_nine_patch_data(&mut self) -> Result<NinePatchData> {
        Ok(NinePatchData {
            bitmap_id: self.read_u16()?,
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
            Some(self.read_u32()?)
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
                let style = self.read_u8()?;
                let size = self.read_u8()?;
                let icon_id = self.read_u16()?;
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
                    let text_color = self.read_u32()?;
                    let pressed_text_color = self.read_u32()?;
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
                let modal_id = self.read_u16()?;
                let is_open = self.read_u8()? != 0;
                let padding = self.read_u16()?;
                let backdrop_alpha = self.read_u8()?;
                let title_len = self.read_u16()?;
                let title = self.read_string(title_len)?;
                let content_height = self.read_f32()?;
                let child_count = self.read_u16()?;
                let bg_color = self.read_u32()?;
                let header_color = self.read_u32()?;
                let title_color = self.read_u32()?;
                let mut body = Vec::with_capacity(child_count as usize);
                for _ in 0..child_count {
                    body.push(self.read_node()?);
                }
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
                    body,
                })
            }
            NODE_SCROLL => {
                let scroll_id = self.read_u16()?;
                let props = self.read_props()?;
                let child_count = self.read_u16()?;
                let mut children = Vec::with_capacity(child_count as usize);
                for _ in 0..child_count {
                    children.push(self.read_node()?);
                }
                Ok(TreeNode::Scroll {
                    scroll_id,
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
                let fill_color = self.read_u32()?;
                let track_color = self.read_u32()?;
                let bg_color = self.read_u32()?;
                let skin = if self.read_u8()? != 0 {
                    Some(SliderSkinData {
                        track: NinePatchData {
                            bitmap_id: self.read_u16()?,
                            left: self.read_u16()?,
                            top: self.read_u16()?,
                            right: self.read_u16()?,
                            bottom: self.read_u16()?,
                        },
                        track_h: self.read_u16()?,
                        thumb_id: self.read_u16()?,
                        thumb_w: self.read_u16()?,
                        thumb_h: self.read_u16()?,
                        thumb_pressed_id: self.read_u16()?,
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
                let color = self.read_u32()?;
                Ok(DrawCommand::Rect { x, y, w, h, color })
            }
            DRAW_CIRCLE => {
                let cx = self.read_f32()?;
                let cy = self.read_f32()?;
                let r = self.read_f32()?;
                let color = self.read_u32()?;
                Ok(DrawCommand::Circle { cx, cy, r, color })
            }
            DRAW_ICON => {
                let x = self.read_f32()?;
                let y = self.read_f32()?;
                let w = self.read_f32()?;
                let h = self.read_f32()?;
                let color = self.read_u32()?;
                let icon_id = self.read_u16()?;
                let anti_alias = self.read_u8()? != 0;
                Ok(DrawCommand::Icon {
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
                let bitmap_id = self.read_u16()?;
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
                let color = self.read_u32()?;
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
                let bitmap_id = self.read_u16()?;
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
                let bitmap_id = self.read_u16()?;
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

use crate::animation::{apply_easing, compute_animation_value, interpolate_color, multiply_alpha};
use crate::components::{ButtonSize, ButtonStyle, draw_button};
use crate::host_api::{
    AnimationState, FrameTimings, ModalState, PrevDrawValues, ScrollState, TransitionState,
};
use crate::interaction::InteractionState;
use crate::renderer::Renderer;

/// Mutable animation context threaded through the render pipeline.
struct AnimationContext<'a> {
    animation_states: &'a mut HashMap<u64, AnimationState>,
    transition_states: &'a mut HashMap<(u16, u16), TransitionState>,
    delta_ms: u32,
    frame_counter: u64,
    draw_counter: u32,
    canvas_index: u16,
    draw_in_canvas: u16,
    /// Set to true when any animation or transition is in progress.
    has_active: bool,
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
    /// Click state for each button (in order of appearance)
    pub clicks: Vec<bool>,
    /// One-shot touch clicks on interactive canvases (on finger-up)
    pub touch_clicks: HashMap<String, TouchHit>,
    /// Active drag positions on interactive canvases (while finger is down)
    pub touch_drags: HashMap<String, TouchHit>,
}

/// Paragraph data for measurement and rendering
#[derive(Clone, Debug)]
struct ParagraphData {
    base_style: TextStyle,
    spans: Vec<SpanData>,
}

/// Notification data for measurement and rendering
#[derive(Clone)]
struct NotificationData {
    kind: u8,
    title: String,
    subtitle: String,
}

/// Button data stored in taffy node context.
#[derive(Clone)]
pub(crate) struct ButtonContext {
    id: u32,
    label: String,
    style: u8,
    size: u8,
    icon_id: u16,
    disabled: bool,
    skin: Option<ButtonSkinData>,
}

/// Nine-patch background image data (bitmap_id == 0 means none).
#[derive(Clone, Copy, Default)]
pub(crate) struct BgNinePatch {
    bitmap_id: u16,
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

/// Host-side slider skin data (deserialized from wire).
#[derive(Clone, Debug)]
pub struct SliderSkinData {
    track: NinePatchData,
    track_h: u16,
    thumb_id: u16,
    thumb_w: u16,
    thumb_h: u16,
    #[expect(dead_code)] // used when touch-pressed state rendering is added
    thumb_pressed_id: u16,
}

/// Host-side progress bar rendering data.
#[derive(Clone, Default)]
struct ProgressBarData {
    track_h: f32,
    /// 0 = Fraction, 1 = Indeterminate
    mode: u8,
    fraction: f32,
    active: bool,
    fill_color: u32,
    track_color: u32,
    bg_color: u32,
    skin: Option<SliderSkinData>,
}

/// Node data attached to taffy nodes
#[derive(Clone, Default)]
pub(crate) struct NodeContext {
    background: u32,
    bg_nine_patch: BgNinePatch,
    paragraph: Option<ParagraphData>,
    button: Option<ButtonContext>,
    draws: Vec<DrawCommand>, // canvas draw commands
    /// Touch interaction key for interactive canvases (None = decorative)
    touch_key: Option<String>,
    notification: Option<NotificationData>,
    scroll_id: Option<u16>,
    progress_bar: Option<ProgressBarData>,
}

/// Collected modal info for overlay rendering
struct ModalInfo {
    modal_id: u16,
    is_open: bool,
    padding: u16,
    backdrop_alpha: u8,
    title: String,
    content_height: f32,
    /// Modal body background color. `0` = default.
    bg_color: u32,
    /// Header background color. `0` = default.
    header_color: u32,
    /// Title text color. `0` = default.
    title_color: u32,
    body: Vec<TreeNode>,
    /// Starting button index for this modal's buttons
    button_index_start: u32,
}

/// Process a tree: deserialize, layout, render.
///
/// Returns `(tree_node, result, has_active_animations, timings)` — the caller
/// can cache `tree_node` for animation-only frames to skip deserialization.
#[expect(clippy::too_many_arguments)]
pub(crate) fn process_tree(
    data: &[u8],
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<u16, ModalState>,
    scroll_states: &mut HashMap<u16, ScrollState>,
    animation_states: &mut HashMap<u64, AnimationState>,
    transition_states: &mut HashMap<(u16, u16), TransitionState>,
    frame_counter: u64,
    delta_ms: u32,
    taffy: &mut TaffyTree<NodeContext>,
) -> Result<(TreeNode, TreeResult, bool, FrameTimings)> {
    let mut timings = FrameTimings::default();

    // Phase 1: Deserialize
    let t0 = Instant::now();
    let tree_node = deserialize_tree(data)?;
    timings.deserialize_us = t0.elapsed().as_micros() as u32;

    let (result, has_active) = layout_and_render(
        &tree_node,
        width,
        height,
        renderer,
        interaction,
        modal_states,
        scroll_states,
        animation_states,
        transition_states,
        frame_counter,
        delta_ms,
        &mut timings,
        taffy,
    )?;

    Ok((tree_node, result, has_active, timings))
}

/// Layout and render a previously deserialized tree.
///
/// Populates `timings.layout_us` and `timings.render_us`.
#[expect(clippy::too_many_arguments)]
pub(crate) fn layout_and_render(
    tree_node: &TreeNode,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<u16, ModalState>,
    scroll_states: &mut HashMap<u16, ScrollState>,
    animation_states: &mut HashMap<u64, AnimationState>,
    transition_states: &mut HashMap<(u16, u16), TransitionState>,
    frame_counter: u64,
    delta_ms: u32,
    timings: &mut FrameTimings,
    taffy: &mut TaffyTree<NodeContext>,
) -> Result<(TreeResult, bool)> {
    // Phase 2: Build Taffy tree + compute layout
    let t1 = Instant::now();

    let mut result = TreeResult::default();
    let mut button_id: u32 = 0;
    let mut modals: Vec<ModalInfo> = Vec::new();

    // Reuse taffy tree — clear nodes but keep internal allocations
    taffy.clear();
    let root_id = build_taffy_node(taffy, tree_node, &mut result, &mut button_id, &mut modals)?;

    // Set root size
    if let Ok(style) = taffy.style(root_id) {
        let mut new_style = style.clone();
        new_style.size = Size {
            width: length(width),
            height: length(height),
        };
        taffy.set_style(root_id, new_style)?;
    }

    compute_taffy_layout(taffy, root_id, renderer)?;

    timings.layout_us = t1.elapsed().as_micros() as u32;

    // Phase 3: Render
    let t2 = Instant::now();

    let mut anim_ctx = AnimationContext {
        animation_states,
        transition_states,
        delta_ms,
        frame_counter,
        draw_counter: 0,
        canvas_index: 0,
        draw_in_canvas: 0,
        has_active: false,
    };

    // Render main tree
    render_taffy_node(
        taffy,
        root_id,
        0.0,
        0.0,
        renderer,
        interaction,
        scroll_states,
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
            interaction,
            modal_states,
            scroll_states,
            delta_ms,
            &mut result,
            &mut anim_ctx,
            taffy,
        );
    }

    // GC: remove animation/transition states not seen this frame
    anim_ctx
        .animation_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);
    anim_ctx
        .transition_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);

    timings.render_us = t2.elapsed().as_micros() as u32;

    // Check modal animations too
    let modal_animating = modal_states
        .values()
        .any(|s| s.animation_progress > 0.0 && s.animation_progress < 1.0);

    Ok((result, anim_ctx.has_active || modal_animating))
}

#[expect(clippy::too_many_lines)]
fn build_taffy_node(
    taffy: &mut TaffyTree<NodeContext>,
    node: &TreeNode,
    result: &mut TreeResult,
    button_id: &mut u32,
    modals: &mut Vec<ModalInfo>,
) -> Result<taffy::NodeId> {
    match node {
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, result, button_id, modals))
                .collect::<Result<_>>()?;

            let is_center = matches!(node, TreeNode::Center(_, _));
            let flex_dir = if matches!(node, TreeNode::Row(_, _)) {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };

            let style = Style {
                flex_direction: flex_dir,
                justify_content: if is_center {
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
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            if props.background != 0 || props.bg_np_id != 0 {
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
            // Don't pre-measure - use Taffy's measure callback with available width
            let style = Style {
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
            label,
            style: btn_style,
            size: btn_size,
            icon_id,
            disabled,
            skin,
        } => {
            let id_num = *button_id;
            *button_id += 1;
            result.clicks.push(false);

            let sz = ButtonSize::from(*btn_size);
            let height = sz.height();

            let style = Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(height),
                },
                align_self: Some(AlignSelf::FlexStart),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    button: Some(ButtonContext {
                        id: id_num,
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
            let style = Style {
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
            scroll_id,
            props,
            children,
        } => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, result, button_id, modals))
                .collect::<Result<_>>()?;

            // Extra right padding so content doesn't sit under the scrollbar overlay
            let scrollbar_clearance = 16.0;
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
                    scroll_id: Some(*scroll_id),
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
            content_height,
            bg_color,
            header_color,
            title_color,
            body,
        } => {
            // Record button index start for this modal
            let button_index_start = *button_id;

            // Count buttons in modal body and allocate slots
            if *is_open {
                for child in body {
                    count_tree_buttons(child, button_id, result);
                }
                // Close button gets the next index
                result.clicks.push(false);
                *button_id += 1;
            }

            // Store modal for overlay rendering
            modals.push(ModalInfo {
                modal_id: *modal_id,
                is_open: *is_open,
                padding: *padding,
                backdrop_alpha: *backdrop_alpha,
                title: title.clone(),
                content_height: *content_height,
                bg_color: *bg_color,
                header_color: *header_color,
                title_color: *title_color,
                body: body.clone(),
                button_index_start,
            });

            // Modal doesn't participate in normal layout - zero-size placeholder
            let style = Style {
                size: Size {
                    width: length(0.0),
                    height: length(0.0),
                },
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
fn compute_taffy_layout(
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
                    let w = if btn.icon_id != 0 && btn.label.is_empty() {
                        h
                    } else if btn.icon_id != 0 {
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
fn render_taffy_node(
    taffy: &TaffyTree<NodeContext>,
    node_id: taffy::NodeId,
    parent_x: f32,
    parent_y: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    scroll_states: &mut HashMap<u16, ScrollState>,
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
    let scroll_id = taffy
        .get_node_context(node_id)
        .and_then(|ctx| ctx.scroll_id);
    let touch_key = taffy
        .get_node_context(node_id)
        .and_then(|ctx| ctx.touch_key.clone());

    if let Some(ctx) = taffy.get_node_context(node_id) {
        if ctx.bg_nine_patch.bitmap_id != 0 {
            let np = &ctx.bg_nine_patch;
            renderer.draw_nine_patch(
                x,
                y,
                w,
                h,
                np.bitmap_id,
                np.left,
                np.top,
                np.right,
                np.bottom,
            );
        } else if ctx.background != 0 {
            renderer.fill_rect(x, y, w, h, ctx.background);
        }

        if let Some(ref para) = ctx.paragraph {
            renderer.draw_paragraph(&para.base_style, &para.spans, x, y, w);
        }

        if let Some(ref btn) = ctx.button {
            let mut key_buf = [0_u8; 16];
            let key = format_btn_key(btn.id, &mut key_buf);
            let (clicked, _) = draw_button(
                renderer,
                interaction,
                key,
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
            if let Some(slot) = result.clicks.get_mut(btn.id as usize) {
                *slot = clicked;
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
        let bounds = crate::interaction::Rect::new(x as i32, y as i32, w as u32, h as u32);
        let (clicked, click_pos) = interaction.button_with_pos(tk, bounds);
        if clicked && let Some((lx, ly)) = click_pos {
            result.touch_clicks.insert(
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
            result.touch_drags.insert(
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
    if let Some(sid) = scroll_id {
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
        let sbar_key = format!("sbar_{sid}");
        let sbar_pressed = if has_scrollbar {
            let sbar_hit_rect = crate::interaction::Rect::new(
                (x + w - sbar_hit_w) as i32,
                y as i32,
                sbar_hit_w as u32,
                h as u32,
            );
            interaction.button(&sbar_key, sbar_hit_rect);
            interaction.is_pressed(&sbar_key)
        } else {
            false
        };

        // Register content hit region for drag/wheel scrolling
        let scroll_key = format!("scroll_{sid}");
        let scroll_region = crate::interaction::Rect::new(x as i32, y as i32, w as u32, h as u32);
        interaction.button(&scroll_key, scroll_region);

        // Read scroll delta — scrollbar drag scales by content/viewport ratio
        let scroll_delta = if sbar_pressed {
            let ratio = if h > 0.0 { content_height / h } else { 1.0 };
            (interaction.get_scroll_delta(&sbar_key) as f32 * ratio) as i32
        } else if interaction.is_pressed(&scroll_key) {
            -interaction.get_scroll_delta(&scroll_key)
        } else {
            interaction.get_global_scroll_delta()
        };

        let state = scroll_states.entry(sid).or_default();
        state.scroll_offset += scroll_delta as f32;
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

// ── Progress bar rendering ────────────────────────────────────────────

/// Squiggle wave constants (same as the previous WASM-side implementation).
const PB_WAVE_POINTS_PER_CYCLE: usize = 8;
const PB_WAVE_LENGTH: f32 = 16.0;

/// Render a host-side progress bar. Returns `true` if animations are active
/// (caller should request next frame).
fn render_progress_bar(
    renderer: &mut dyn Renderer,
    pb: &ProgressBarData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    anim_ctx: &mut AnimationContext<'_>,
) -> bool {
    if let Some(skin) = &pb.skin {
        render_progress_bar_skinned(renderer, pb, skin, x, y, w, h)
    } else {
        render_progress_bar_flat(renderer, pb, x, y, w, anim_ctx)
    }
}

/// Flat (unskinned) progress bar: rects, circles, squiggle.
fn render_progress_bar_flat(
    renderer: &mut dyn Renderer,
    pb: &ProgressBarData,
    x: f32,
    y: f32,
    w: f32,
    anim_ctx: &mut AnimationContext<'_>,
) -> bool {
    let track_h = pb.track_h;
    let dot_radius = track_h * 2.0;
    let bar_height = dot_radius * 2.0 + track_h;
    let half_track = track_h / 2.0;
    let mid_y = y + bar_height / 2.0;
    let is_indeterminate = pb.mode == 1;
    let fraction = pb.fraction.clamp(0.0, 1.0);
    let fill_w = w * fraction;

    let mut animating = false;

    if is_indeterminate && pb.active {
        // Full-width animated squiggle
        render_squiggle(renderer, x, mid_y, w, track_h, pb.fill_color, anim_ctx);
        animating = true;
    } else {
        // Background track (full width)
        renderer.fill_rect(x, mid_y - half_track, w, track_h, pb.track_color);

        if pb.active && fill_w > track_h {
            // Animated squiggle on the filled portion
            render_squiggle(renderer, x, mid_y, fill_w, track_h, pb.fill_color, anim_ctx);

            // Clip rect: hide squiggle past the playhead
            let clip_x = x + fill_w;
            renderer.fill_rect(clip_x, y, w - fill_w + 1.0, bar_height, pb.bg_color);

            // Remaining track after the playhead
            let track_x = clip_x + dot_radius;
            renderer.fill_rect(
                track_x,
                mid_y - half_track,
                (w - fill_w - dot_radius).max(0.0),
                track_h,
                pb.track_color,
            );
            animating = true;
        } else if fill_w > 0.0 {
            // Static fill (not active, or fill too small for squiggle)
            renderer.fill_rect(x, mid_y - half_track, fill_w, track_h, pb.fill_color);
        }

        // Playhead dot
        if fill_w > 0.0 {
            renderer.fill_circle(x + fill_w, mid_y, dot_radius, pb.fill_color);
        }
    }

    animating
}

/// Skinned progress bar: 9-patch track + bitmap thumb.
fn render_progress_bar_skinned(
    renderer: &mut dyn Renderer,
    pb: &ProgressBarData,
    skin: &SliderSkinData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> bool {
    let np = &skin.track;
    let track_h = f32::from(skin.track_h);
    let track_y = y + (h - track_h) / 2.0;

    // Draw track background (9-patch stretched to full width)
    renderer.draw_nine_patch(
        x,
        track_y,
        w,
        track_h,
        np.bitmap_id,
        np.left,
        np.top,
        np.right,
        np.bottom,
    );

    // Draw thumb at progress position (scale down when bar is narrow)
    if skin.thumb_id != 0 && pb.mode == 0 {
        let fraction = pb.fraction.clamp(0.0, 1.0);
        let thumb_w = f32::from(skin.thumb_w);
        let thumb_h = f32::from(skin.thumb_h);
        let scale = (w / (thumb_w * 4.0)).min(1.0);
        let tw = thumb_w * scale;
        let th = thumb_h * scale;
        let thumb_x = x + fraction * (w - tw);
        let thumb_y = y + (h - th) / 2.0;
        renderer.draw_bitmap(thumb_x, thumb_y, tw, th, skin.thumb_id);
    }

    false // skinned bars don't animate (no squiggle)
}

/// Render an animated sine-wave squiggle.
///
/// The squiggle scrolls left via a time-based phase offset, recreating the
/// same visual as the old WASM-side `TranslateX` animation.
fn render_squiggle(
    renderer: &mut dyn Renderer,
    x: f32,
    mid_y: f32,
    width: f32,
    track_h: f32,
    color: u32,
    anim_ctx: &AnimationContext<'_>,
) {
    let amplitude = track_h / 2.0;
    let step = PB_WAVE_LENGTH / PB_WAVE_POINTS_PER_CYCLE as f32;

    // Time-based scroll offset: one full wavelength per 800ms cycle
    let cycle_ms = 800.0;
    let phase_frac = (anim_ctx.frame_counter as f32 * anim_ctx.delta_ms as f32 / cycle_ms).fract();
    let offset = -phase_frac * PB_WAVE_LENGTH;

    let start_x = -PB_WAVE_LENGTH + offset;
    let end_x = width + PB_WAVE_LENGTH + offset;
    let n_points = ((end_x - start_x) / step) as usize + 1;

    let points: Vec<(f32, f32)> = (0..n_points)
        .map(|i| {
            let lx = start_x + i as f32 * step;
            let phase = lx / PB_WAVE_LENGTH * std::f32::consts::TAU;
            (x + lx - offset, mid_y + phase.sin() * amplitude)
        })
        .collect();

    renderer.stroke_path(&points, track_h, color, false, true);
}

/// Render a draw command with canvas-local coordinates
fn render_draw_command(
    renderer: &mut dyn Renderer,
    draw: &DrawCommand,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    anim_ctx: &mut AnimationContext<'_>,
) {
    render_draw_inner(
        renderer, draw, cx, cy, cw, ch, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, None, anim_ctx,
    );
}

/// Get the bounds (width, height) of a draw command
fn get_draw_bounds(draw: &DrawCommand) -> (f32, f32) {
    match draw {
        DrawCommand::Rect { w, h, .. }
        | DrawCommand::Icon { w, h, .. }
        | DrawCommand::Bitmap { w, h, .. }
        | DrawCommand::Sphere { w, h, .. }
        | DrawCommand::NinePatch { w, h, .. } => (*w, *h),
        DrawCommand::Circle { r, .. } => (*r * 2.0, *r * 2.0),
        DrawCommand::Centered { inner }
        | DrawCommand::Rotated { inner, .. }
        | DrawCommand::Modified { inner, .. }
        | DrawCommand::Orbit { inner, .. } => get_draw_bounds(inner),
        DrawCommand::Text { .. } => (0.0, 0.0),
        DrawCommand::Path { points, .. } => {
            if points.is_empty() {
                (0.0, 0.0)
            } else {
                let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
                let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
                for &(x, y) in points {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
                (max_x - min_x, max_y - min_y)
            }
        }
    }
}

/// Render a draw command with accumulated transforms and animation modifiers.
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_draw_inner(
    renderer: &mut dyn Renderer,
    draw: &DrawCommand,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    offset_x: f32,
    offset_y: f32,
    rotation: f32,
    scale: f32,
    alpha: f32,
    orbit_angle_offset: f32,
    color_override: Option<u32>,
    anim_ctx: &mut AnimationContext<'_>,
) {
    match draw {
        DrawCommand::Rect { x, y, w, h, color } => {
            let ew = *w * scale;
            let eh = *h * scale;
            // Center-anchored scaling: offset by half the size difference
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            let final_color = if alpha < 1.0 {
                multiply_alpha(color_override.unwrap_or(*color), alpha)
            } else {
                color_override.unwrap_or(*color)
            };
            if rotation == 0.0 {
                renderer.fill_rect(rx, ry, ew, eh, final_color);
            } else {
                // Rotate around canvas center (like CSS transform-origin: center)
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.fill_rect(rx - pivot_x, ry - pivot_y, ew, eh, final_color);
                renderer.restore();
            }
        }
        DrawCommand::Icon {
            x,
            y,
            w,
            h,
            color,
            icon_id,
            anti_alias,
        } => {
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            let final_color = if alpha < 1.0 {
                multiply_alpha(color_override.unwrap_or(*color), alpha)
            } else {
                color_override.unwrap_or(*color)
            };
            if rotation == 0.0 {
                renderer.draw_icon(rx, ry, ew, eh, final_color, *icon_id, *anti_alias);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_icon(
                    rx - pivot_x,
                    ry - pivot_y,
                    ew,
                    eh,
                    final_color,
                    *icon_id,
                    *anti_alias,
                );
                renderer.restore();
            }
        }
        DrawCommand::Bitmap {
            x,
            y,
            w,
            h,
            bitmap_id,
        } => {
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            if rotation == 0.0 {
                renderer.draw_bitmap(rx, ry, ew, eh, *bitmap_id);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_bitmap(rx - pivot_x, ry - pivot_y, ew, eh, *bitmap_id);
                renderer.restore();
            }
        }
        DrawCommand::NinePatch {
            x,
            y,
            w,
            h,
            bitmap_id,
            left,
            top,
            right,
            bottom,
        } => {
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            if rotation == 0.0 {
                renderer.draw_nine_patch(rx, ry, ew, eh, *bitmap_id, *left, *top, *right, *bottom);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_nine_patch(
                    rx - pivot_x,
                    ry - pivot_y,
                    ew,
                    eh,
                    *bitmap_id,
                    *left,
                    *top,
                    *right,
                    *bottom,
                );
                renderer.restore();
            }
        }
        DrawCommand::Circle {
            cx: circle_cx,
            cy: circle_cy,
            r,
            color,
        } => {
            let er = *r * scale;
            let scx = *circle_cx + offset_x;
            let scy = *circle_cy + offset_y;
            let final_color = if alpha < 1.0 {
                multiply_alpha(color_override.unwrap_or(*color), alpha)
            } else {
                color_override.unwrap_or(*color)
            };
            renderer.fill_circle(cx + scx, cy + scy, er, final_color);
        }
        DrawCommand::Centered { inner } => {
            let (iw, ih) = get_draw_bounds(inner);
            let new_offset_x = (cw - iw) / 2.0;
            let new_offset_y = (ch - ih) / 2.0;
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                new_offset_x,
                new_offset_y,
                rotation,
                scale,
                alpha,
                orbit_angle_offset,
                color_override,
                anim_ctx,
            );
        }
        DrawCommand::Orbit {
            radius,
            angle,
            inner,
        } => {
            let effective_angle = *angle + orbit_angle_offset;
            let center_offset_x = cw / 2.0;
            let center_offset_y = ch / 2.0;
            let (iw, ih) = get_draw_bounds(inner);
            let new_offset_x = center_offset_x + radius * effective_angle.cos() - iw / 2.0;
            let new_offset_y = center_offset_y + radius * effective_angle.sin() - ih / 2.0;
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                new_offset_x,
                new_offset_y,
                rotation,
                scale,
                alpha,
                0.0, // orbit_angle_offset consumed
                color_override,
                anim_ctx,
            );
        }
        DrawCommand::Rotated { angle, inner } => {
            render_draw_inner(
                renderer,
                inner,
                cx,
                cy,
                cw,
                ch,
                offset_x,
                offset_y,
                rotation + angle,
                scale,
                alpha,
                orbit_angle_offset,
                color_override,
                anim_ctx,
            );
        }
        DrawCommand::Modified {
            animations,
            transition,
            color_space,
            inner,
        } => {
            let mut acc_rotation = rotation;
            let mut acc_scale = scale;
            let mut acc_alpha = alpha;
            let mut acc_offset_x = offset_x;
            let mut acc_offset_y = offset_y;
            let mut acc_orbit_angle = orbit_angle_offset;
            let mut acc_color: Option<u32> = color_override;
            let mut sphere_override: Option<(f32, f32, f32, f32, f32)> = None;

            // Process animations
            for anim_def in animations {
                let key = animation_key(anim_def, anim_ctx.draw_counter);
                let state =
                    anim_ctx
                        .animation_states
                        .entry(key)
                        .or_insert_with(|| AnimationState {
                            elapsed_ms: 0,
                            last_seen_frame: anim_ctx.frame_counter,
                        });
                state.last_seen_frame = anim_ctx.frame_counter;

                let (value, active) = compute_animation_value(anim_def, state, anim_ctx.delta_ms);
                if active {
                    anim_ctx.has_active = true;
                }

                match anim_def.property {
                    AnimProperty::Rotate => acc_rotation += value,
                    AnimProperty::Scale => acc_scale *= value,
                    AnimProperty::Alpha => acc_alpha *= value,
                    AnimProperty::TranslateX => acc_offset_x += value,
                    AnimProperty::TranslateY => acc_offset_y += value,
                    AnimProperty::OrbitAngle => acc_orbit_angle += value,
                    AnimProperty::Color => {
                        let from_color = f32::to_bits(anim_def.from);
                        let to_color = f32::to_bits(anim_def.to);
                        // value is the raw lerped f32, recompute t for color
                        let range = anim_def.to - anim_def.from;
                        let t = if range.abs() > f32::EPSILON {
                            (value - anim_def.from) / range
                        } else {
                            0.0
                        };
                        acc_color = Some(interpolate_color(from_color, to_color, t, *color_space));
                    }
                }
            }

            // Process transition
            if let Some(trans_def) = transition {
                let current_values = extract_draw_values(inner);
                let key = (anim_ctx.canvas_index, anim_ctx.draw_in_canvas);
                let state = anim_ctx.transition_states.entry(key).or_insert_with(|| {
                    TransitionState {
                        from: current_values,
                        target: current_values,
                        elapsed_ms: trans_def.duration_ms, // start finished
                        last_seen_frame: anim_ctx.frame_counter,
                    }
                });
                state.last_seen_frame = anim_ctx.frame_counter;

                // Detect target change
                if state.target != current_values {
                    // D3-style: interpolate from current interpolated position
                    let t = if trans_def.duration_ms > 0 {
                        (state.elapsed_ms as f32 / trans_def.duration_ms as f32).min(1.0)
                    } else {
                        1.0
                    };
                    let eased_t = apply_easing(trans_def.easing, t);
                    state.from =
                        interpolate_draw_values(&state.from, &state.target, eased_t, *color_space);
                    state.target = current_values;
                    state.elapsed_ms = 0;
                }

                state.elapsed_ms = state.elapsed_ms.saturating_add(anim_ctx.delta_ms);

                if state.elapsed_ms < trans_def.duration_ms {
                    anim_ctx.has_active = true;
                    let t = state.elapsed_ms as f32 / trans_def.duration_ms as f32;
                    let eased_t = apply_easing(trans_def.easing, t);
                    let interp =
                        interpolate_draw_values(&state.from, &state.target, eased_t, *color_space);
                    // Apply interpolated overrides to accumulated state
                    acc_offset_x += interp.x - current_values.x;
                    acc_offset_y += interp.y - current_values.y;
                    acc_scale *= if current_values.w > 0.0 {
                        interp.w / current_values.w
                    } else {
                        1.0
                    };
                    acc_orbit_angle += interp.angle - current_values.angle;
                    acc_rotation += interp.rotation - current_values.rotation;
                    if interp.color != current_values.color {
                        acc_color = Some(interp.color);
                    }
                    if matches!(inner.as_ref(), DrawCommand::Sphere { .. }) {
                        sphere_override = Some((
                            interp.center_lat,
                            interp.center_lon,
                            interp.zoom,
                            interp.light_lat,
                            interp.light_lon,
                        ));
                    }
                }
            }

            anim_ctx.draw_counter += 1;

            if let (
                Some((center_lat, center_lon, zoom, light_lat, light_lon)),
                DrawCommand::Sphere {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                    atmosphere,
                    ..
                },
            ) = (sphere_override, inner.as_ref())
            {
                let overridden = DrawCommand::Sphere {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                    bitmap_id: *bitmap_id,
                    atmosphere: *atmosphere,
                    center_lat,
                    center_lon,
                    zoom,
                    light_lat,
                    light_lon,
                };
                render_draw_inner(
                    renderer,
                    &overridden,
                    cx,
                    cy,
                    cw,
                    ch,
                    acc_offset_x,
                    acc_offset_y,
                    acc_rotation,
                    acc_scale,
                    acc_alpha,
                    acc_orbit_angle,
                    acc_color,
                    anim_ctx,
                );
            } else {
                render_draw_inner(
                    renderer,
                    inner,
                    cx,
                    cy,
                    cw,
                    ch,
                    acc_offset_x,
                    acc_offset_y,
                    acc_rotation,
                    acc_scale,
                    acc_alpha,
                    acc_orbit_angle,
                    acc_color,
                    anim_ctx,
                );
            }
        }
        DrawCommand::Path {
            points,
            color,
            stroke_width,
            closed,
            fill,
            smooth,
        } => {
            if points.len() < 2 {
                return;
            }
            let final_color = if alpha < 1.0 {
                multiply_alpha(color_override.unwrap_or(*color), alpha)
            } else {
                color_override.unwrap_or(*color)
            };

            // Transform points: apply canvas offset + accumulated offset + scale
            let transformed: Vec<(f32, f32)> = points
                .iter()
                .map(|&(px, py)| (cx + (px + offset_x) * scale, cy + (py + offset_y) * scale))
                .collect();

            if rotation != 0.0 {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                // Re-transform relative to pivot
                let pivoted: Vec<(f32, f32)> = transformed
                    .iter()
                    .map(|&(px, py)| (px - pivot_x, py - pivot_y))
                    .collect();
                if *fill {
                    renderer.fill_path_points(&pivoted, final_color, *smooth);
                } else {
                    renderer.stroke_path(
                        &pivoted,
                        *stroke_width * scale,
                        final_color,
                        *closed,
                        *smooth,
                    );
                }
                renderer.restore();
            } else if *fill {
                renderer.fill_path_points(&transformed, final_color, *smooth);
            } else {
                renderer.stroke_path(
                    &transformed,
                    *stroke_width * scale,
                    final_color,
                    *closed,
                    *smooth,
                );
            }
        }
        DrawCommand::Text { x, y, text, style } => {
            let rx = cx + *x + offset_x;
            let ry = cy + *y + offset_y;
            let mut render_style = *style;
            render_style.size = (style.size as f32 * scale) as u32;
            render_style.color = if alpha < 1.0 {
                multiply_alpha(color_override.unwrap_or(style.color), alpha)
            } else {
                color_override.unwrap_or(style.color)
            };
            if rotation == 0.0 {
                renderer.draw_canvas_text(text, rx, ry, &render_style);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_canvas_text(text, rx - pivot_x, ry - pivot_y, &render_style);
                renderer.restore();
            }
        }
        DrawCommand::Sphere {
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
        } => {
            let ew = *w * scale;
            let eh = *h * scale;
            let sx = *x + offset_x + (*w - ew) / 2.0;
            let sy = *y + offset_y + (*h - eh) / 2.0;
            let rx = cx + sx;
            let ry = cy + sy;
            if rotation == 0.0 {
                renderer.draw_sphere(
                    rx,
                    ry,
                    ew,
                    eh,
                    *bitmap_id,
                    *center_lat,
                    *center_lon,
                    *zoom,
                    *light_lat,
                    *light_lon,
                    *atmosphere,
                );
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_sphere(
                    rx - pivot_x,
                    ry - pivot_y,
                    ew,
                    eh,
                    *bitmap_id,
                    *center_lat,
                    *center_lon,
                    *zoom,
                    *light_lat,
                    *light_lon,
                    *atmosphere,
                );
                renderer.restore();
            }
        }
    }
}

/// Compute a content-based hash key for an animation definition + draw counter salt.
fn animation_key(def: &HostAnimationDef, draw_counter: u32) -> u64 {
    // Simple FNV-like hash of the animation definition bytes
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    h ^= def.property as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.from.to_bits() as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.to.to_bits() as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.duration_ms as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.delay_ms as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.easing as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^= def.loop_mode as u64;
    h = h.wrapping_mul(0x0100_0000_01b3);
    h ^ draw_counter as u64
}

/// Extract the static values from a draw command's innermost content for transition tracking.
fn extract_draw_values(draw: &DrawCommand) -> PrevDrawValues {
    match draw {
        DrawCommand::Bitmap { x, y, w, h, .. } | DrawCommand::NinePatch { x, y, w, h, .. } => {
            PrevDrawValues {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                ..Default::default()
            }
        }
        DrawCommand::Sphere {
            x,
            y,
            w,
            h,
            center_lat,
            center_lon,
            zoom,
            light_lat,
            light_lon,
            ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            center_lat: *center_lat,
            center_lon: *center_lon,
            zoom: *zoom,
            light_lat: *light_lat,
            light_lon: *light_lon,
            ..Default::default()
        },
        DrawCommand::Rect { x, y, w, h, color }
        | DrawCommand::Icon {
            x, y, w, h, color, ..
        } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            color: *color,
            ..Default::default()
        },
        DrawCommand::Circle {
            cx, cy, r, color, ..
        } => PrevDrawValues {
            x: *cx,
            y: *cy,
            w: *r,
            color: *color,
            ..Default::default()
        },
        DrawCommand::Orbit {
            radius,
            angle,
            inner,
        } => {
            let mut vals = extract_draw_values(inner);
            vals.angle = *angle;
            vals.radius = *radius;
            vals
        }
        DrawCommand::Rotated { angle, inner } => {
            let mut vals = extract_draw_values(inner);
            vals.rotation = *angle;
            vals
        }
        DrawCommand::Centered { inner } | DrawCommand::Modified { inner, .. } => {
            extract_draw_values(inner)
        }
        DrawCommand::Path { color, .. } => PrevDrawValues {
            color: *color,
            ..Default::default()
        },
        DrawCommand::Text { x, y, style, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            color: style.color,
            ..Default::default()
        },
    }
}

/// Shortest-path delta for angle interpolation (wraps around TAU).
fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    let mut d = to - from;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    if d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// Shortest-path delta for degrees (wraps around 360°).
fn shortest_angle_delta_deg(from: f32, to: f32) -> f32 {
    let mut d = to - from;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// Linearly interpolate between two sets of draw values.
fn interpolate_draw_values(
    a: &PrevDrawValues,
    b: &PrevDrawValues,
    t: f32,
    color_space: ColorSpace,
) -> PrevDrawValues {
    PrevDrawValues {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        w: a.w + (b.w - a.w) * t,
        h: a.h + (b.h - a.h) * t,
        color: if a.color == b.color {
            a.color
        } else {
            interpolate_color(a.color, b.color, t, color_space)
        },
        angle: a.angle + shortest_angle_delta(a.angle, b.angle) * t,
        radius: a.radius + (b.radius - a.radius) * t,
        rotation: a.rotation + shortest_angle_delta(a.rotation, b.rotation) * t,
        center_lat: a.center_lat + (b.center_lat - a.center_lat) * t,
        center_lon: a.center_lon + shortest_angle_delta_deg(a.center_lon, b.center_lon) * t,
        zoom: a.zoom + (b.zoom - a.zoom) * t,
        light_lat: a.light_lat + (b.light_lat - a.light_lat) * t,
        light_lon: a.light_lon + shortest_angle_delta_deg(a.light_lon, b.light_lon) * t,
    }
}

/// Count buttons in a tree node (for modal body button allocation)
fn count_tree_buttons(node: &TreeNode, button_id: &mut u32, result: &mut TreeResult) {
    match node {
        TreeNode::Scroll { children, .. }
        | TreeNode::Column(_, children)
        | TreeNode::Row(_, children)
        | TreeNode::Center(_, children) => {
            for child in children {
                count_tree_buttons(child, button_id, result);
            }
        }
        TreeNode::Button { .. } => {
            result.clicks.push(false);
            *button_id += 1;
        }
        TreeNode::Modal { is_open, body, .. } => {
            if *is_open {
                for child in body {
                    count_tree_buttons(child, button_id, result);
                }
                // Close button
                result.clicks.push(false);
                *button_id += 1;
            }
        }
        TreeNode::Paragraph { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Canvas { .. }
        | TreeNode::Notification { .. }
        | TreeNode::ProgressBar { .. } => {}
    }
}

// Modal rendering constants
const MODAL_HEADER_HEIGHT: f32 = 48.0;
const MODAL_ANIMATION_OPEN_MS: f32 = 250.0;
const MODAL_ANIMATION_CLOSE_MS: f32 = 180.0;

/// Render a modal overlay
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_modal(
    modal: &ModalInfo,
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<u16, ModalState>,
    scroll_states: &mut HashMap<u16, ScrollState>,
    delta_ms: u32,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
    taffy: &mut TaffyTree<NodeContext>,
) {
    // Get or create modal state
    let state = modal_states.entry(modal.modal_id).or_default();

    // Detect state transitions and update animation
    let was_open = state.is_open;
    state.is_open = modal.is_open;
    state.content_height = modal.content_height;

    if modal.is_open && !was_open {
        // Opening: start animation from current progress (or 0)
        // Animation will progress towards 1.0
    } else if !modal.is_open && was_open {
        // Closing: animation will progress towards 0.0
        // Reset scroll when closing
        state.scroll_offset = 0.0;
        state.is_dragging = false;
    }

    // Advance animation
    if modal.is_open {
        let delta = delta_ms as f32 / MODAL_ANIMATION_OPEN_MS;
        state.animation_progress = (state.animation_progress + delta).min(1.0);
    } else {
        let delta = delta_ms as f32 / MODAL_ANIMATION_CLOSE_MS;
        state.animation_progress = (state.animation_progress - delta).max(0.0);
    }

    // Skip rendering if fully closed
    if state.animation_progress <= 0.0 {
        return;
    }

    // Easing functions (ease-out for open, ease-in for close)
    let progress = if modal.is_open {
        ease_out(state.animation_progress)
    } else {
        ease_in(state.animation_progress)
    };

    // Draw backdrop (alpha from modal props, scaled by animation progress)
    let padding = f32::from(modal.padding);
    let backdrop_alpha = ((f32::from(modal.backdrop_alpha) / 255.0) * progress * 255.0) as u8;
    let backdrop_color = u32::from_be_bytes([0, 0, 0, backdrop_alpha]);
    renderer.fill_rect(0.0, 0.0, width, height, backdrop_color);

    // Modal content dimensions
    let modal_width = (width - padding * 2.0).max(0.0);
    let modal_height = (height - padding * 2.0).max(0.0);
    // Body height = modal height - header - internal padding (16px top + 16px bottom)
    let body_padding = 16.0;
    let body_height = (modal_height - MODAL_HEADER_HEIGHT - body_padding * 2.0).max(0.0);
    state.viewport_height = body_height;

    // Animate content position (slide down from -100px)
    let slide_offset = (1.0 - progress) * -100.0;
    let modal_x = padding;
    let modal_y = padding + slide_offset;

    // Content is always fully opaque - only backdrop animates opacity
    // This prevents ugly alpha blending of text over background content

    // Draw modal background
    let modal_bg = if modal.bg_color != 0 {
        modal.bg_color
    } else {
        GRAY_90
    };
    renderer.fill_rect(modal_x, modal_y, modal_width, modal_height, modal_bg);

    // Draw header background
    let header_bg = if modal.header_color != 0 {
        modal.header_color
    } else {
        GRAY_100
    };
    renderer.fill_rect(
        modal_x,
        modal_y,
        modal_width,
        MODAL_HEADER_HEIGHT,
        header_bg,
    );

    // Draw header title
    let title_fg = if modal.title_color != 0 {
        modal.title_color
    } else {
        GRAY_10
    };
    let title_style = TextStyle {
        size: 16,
        weight: 600,
        color: title_fg,
        ..Default::default()
    };
    let title_spans = vec![SpanData {
        text: modal.title.clone(),
        weight: None,
        color: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }];
    renderer.draw_paragraph(
        &title_style,
        &title_spans,
        modal_x + 16.0,
        modal_y + 12.0,
        modal_width - 64.0, // Leave space for close button
    );

    // Draw close button (X icon in top-right)
    let close_btn_x = modal_x + modal_width - 48.0;
    let close_btn_y = modal_y;
    let close_btn_size = MODAL_HEADER_HEIGHT;

    // Close button uses the last button index for this modal
    let close_btn_id = modal.button_index_start + count_modal_body_buttons(&modal.body);
    let mut key_buf = [0_u8; 16];
    let close_key = format_btn_key(close_btn_id, &mut key_buf);

    let close_clicked = draw_button(
        renderer,
        interaction,
        close_key,
        "",
        close_btn_x,
        close_btn_y,
        close_btn_size,
        close_btn_size,
        ButtonStyle::Ghost,
        ButtonSize::Normal,
        ICON_CLOSE,
        false,
        None,
    );

    let (close_was_clicked, _) = close_clicked;
    if close_was_clicked && (close_btn_id as usize) < result.clicks.len() {
        result.clicks[close_btn_id as usize] = true;
    }

    // Body area with scrolling
    let body_x = modal_x;
    let body_y = modal_y + MODAL_HEADER_HEIGHT;

    // Register scroll region for touch handling
    let body_key = format!("modal_{}_body", modal.modal_id);
    let scroll_region = crate::interaction::Rect::new(
        body_x as i32,
        body_y as i32,
        modal_width as u32,
        body_height as u32,
    );
    interaction.button(&body_key, scroll_region);

    // Apply scroll delta from touch drag or mouse wheel
    let scroll_delta = if interaction.is_pressed(&body_key) {
        // Touch drag: invert delta (drag up = scroll down)
        -interaction.get_scroll_delta(&body_key)
    } else {
        // Mouse wheel: use global delta (positive = scroll down)
        interaction.get_global_scroll_delta()
    };
    state.scroll_offset += scroll_delta as f32;

    // Clamp scroll offset
    let max_scroll = (modal.content_height - body_height).max(0.0);
    state.scroll_offset = state.scroll_offset.clamp(0.0, max_scroll);

    // Render body using the full taffy layout engine (same as root content).
    let body_content_width = modal_width - body_padding * 2.0;
    let scroll_offset = state.scroll_offset;
    let body_col = TreeNode::Column(
        PropsData {
            gap: 8.0,
            ..PropsData::default()
        },
        modal.body.clone(),
    );
    let mut modal_button_id = modal.button_index_start;
    let mut dummy_modals: Vec<ModalInfo> = Vec::new();
    taffy.clear();
    if let Ok(body_root) = build_taffy_node(
        taffy,
        &body_col,
        result,
        &mut modal_button_id,
        &mut dummy_modals,
    ) {
        if let Ok(style) = taffy.style(body_root) {
            let mut new_style = style.clone();
            new_style.size = Size {
                width: length(body_content_width),
                height: Dimension::auto(),
            };
            let _ = taffy.set_style(body_root, new_style);
        }
        let _ = compute_taffy_layout(taffy, body_root, renderer);
        renderer.push_scissor(
            body_x + body_padding,
            body_y + body_padding,
            body_content_width,
            body_height,
        );
        render_taffy_node(
            taffy,
            body_root,
            body_x + body_padding,
            body_y + body_padding - scroll_offset,
            renderer,
            interaction,
            scroll_states,
            result,
            anim_ctx,
            0,
        );
        renderer.pop_scissor();
    }

    // Draw scrollbar if content exceeds viewport
    if modal.content_height > body_height {
        let scrollbar_width = 4.0;
        let scrollbar_x = modal_x + modal_width - scrollbar_width - 4.0;
        let scrollbar_track_y = body_y + 4.0;
        let scrollbar_track_height = body_height - 8.0;

        // Track
        let track_color = GRAY_70;
        renderer.fill_rect(
            scrollbar_x,
            scrollbar_track_y,
            scrollbar_width,
            scrollbar_track_height,
            track_color,
        );

        // Thumb
        let thumb_ratio = body_height / modal.content_height;
        let thumb_height = (scrollbar_track_height * thumb_ratio).max(20.0);
        let scroll_ratio = state.scroll_offset / max_scroll.max(1.0);
        let thumb_y = scrollbar_track_y + scroll_ratio * (scrollbar_track_height - thumb_height);

        let thumb_color = GRAY_50;
        renderer.fill_rect(
            scrollbar_x,
            thumb_y,
            scrollbar_width,
            thumb_height,
            thumb_color,
        );
    }
}

/// Count buttons in modal body (excluding close button)
fn count_modal_body_buttons(body: &[TreeNode]) -> u32 {
    let mut count = 0;
    for node in body {
        count += count_node_buttons(node);
    }
    count
}

fn count_node_buttons(node: &TreeNode) -> u32 {
    match node {
        TreeNode::Scroll { children, .. }
        | TreeNode::Column(_, children)
        | TreeNode::Row(_, children)
        | TreeNode::Center(_, children) => children.iter().map(count_node_buttons).sum(),
        TreeNode::Button { .. } => 1,
        TreeNode::Modal { is_open, body, .. } => {
            if *is_open {
                count_modal_body_buttons(body) + 1
            } else {
                0
            }
        }
        TreeNode::Paragraph { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Canvas { .. }
        | TreeNode::Notification { .. }
        | TreeNode::ProgressBar { .. } => 0,
    }
}

/// Ease-out: fast start, slow end
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Ease-in: slow start, fast end
fn ease_in(t: f32) -> f32 {
    t.powi(3)
}

// ── Notification helpers ─────────────────────────────────────────────

const NOTIF_BORDER_W: f32 = 3.0;
const NOTIF_PAD: f32 = 12.0;
const NOTIF_ICON_SIZE: f32 = 16.0;
const NOTIF_ICON_GAP: f32 = 8.0;
/// Left offset from notification edge to text start
const NOTIF_TEXT_LEFT: f32 = NOTIF_BORDER_W + NOTIF_PAD + NOTIF_ICON_SIZE + NOTIF_ICON_GAP;

/// Returns (accent_color, icon_id) for a notification kind byte.
fn notification_accent(kind: u8) -> (u32, u16) {
    match kind {
        0 => (RED_60, ICON_ERROR),
        1 => (ORANGE_40, ICON_WARNING),
        2 => (GREEN_40, ICON_SUCCESS),
        _ => (VIOLET_50, ICON_INFO),
    }
}

fn notification_title_style() -> TextStyle {
    TextStyle {
        size: 14,
        weight: 600,
        color: GRAY_10,
        ..Default::default()
    }
}

fn notification_subtitle_style() -> TextStyle {
    TextStyle {
        size: 14,
        weight: 400,
        color: GRAY_50,
        ..Default::default()
    }
}

fn plain_spans(text: &str) -> [SpanData; 1] {
    [SpanData {
        text: text.to_owned(),
        weight: None,
        color: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }]
}

fn measure_notification(
    notif: &NotificationData,
    known_dimensions: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
    renderer: &mut dyn Renderer,
) -> Size<f32> {
    let avail_w = known_dimensions.width.or(match available_space.width {
        AvailableSpace::Definite(w) => Some(w),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    });
    let text_w = avail_w.map(|w| (w - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0));

    let mut text_h = 0.0;
    if !notif.title.is_empty() {
        let spans = plain_spans(&notif.title);
        let (_, h) = renderer.measure_paragraph(&notification_title_style(), &spans, text_w);
        text_h += h;
    }
    if !notif.subtitle.is_empty() {
        if text_h > 0.0 {
            text_h += 2.0;
        }
        let spans = plain_spans(&notif.subtitle);
        let (_, h) = renderer.measure_paragraph(&notification_subtitle_style(), &spans, text_w);
        text_h += h;
    }

    let content_h = text_h.max(NOTIF_ICON_SIZE);
    Size {
        width: known_dimensions.width.unwrap_or(avail_w.unwrap_or(300.0)),
        height: known_dimensions
            .height
            .unwrap_or(NOTIF_PAD * 2.0 + content_h),
    }
}

fn render_notification(
    notif: &NotificationData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut dyn Renderer,
) {
    let (accent, icon_id) = notification_accent(notif.kind);
    render_notification_banner(
        &notif.title,
        &notif.subtitle,
        accent,
        icon_id,
        x,
        y,
        w,
        h,
        renderer,
    );
}

/// Compute the height of a notification banner for a given width.
pub fn measure_notification_banner(
    title: &str,
    subtitle: &str,
    width: f32,
    renderer: &mut dyn Renderer,
) -> f32 {
    let text_w = (width - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0);

    let mut text_h = 0.0;
    if !title.is_empty() {
        let spans = plain_spans(title);
        let (_, h) = renderer.measure_paragraph(&notification_title_style(), &spans, Some(text_w));
        text_h += h;
    }
    if !subtitle.is_empty() {
        if text_h > 0.0 {
            text_h += 2.0;
        }
        let spans = plain_spans(subtitle);
        let (_, h) =
            renderer.measure_paragraph(&notification_subtitle_style(), &spans, Some(text_w));
        text_h += h;
    }

    NOTIF_PAD * 2.0 + text_h.max(NOTIF_ICON_SIZE)
}

/// Render a notification-style banner at a given position.
///
/// This is the shared visual used by both the tree notification node and
/// host-side overlays (e.g. the fuel-limiter dead state).
#[expect(clippy::too_many_arguments)]
pub fn render_notification_banner(
    title: &str,
    subtitle: &str,
    accent: u32,
    icon_id: u16,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut dyn Renderer,
) {
    // Background
    renderer.fill_rect(x, y, w, h, GRAY_90);

    // Left accent border
    renderer.fill_rect(x, y, NOTIF_BORDER_W, h, accent);

    // Icon — vertically centered with the title line
    let title_line_h = notification_title_style().size as f32 * 1.3;
    let icon_x = x + NOTIF_BORDER_W + NOTIF_PAD;
    let icon_y = y + NOTIF_PAD + (title_line_h - NOTIF_ICON_SIZE) / 2.0;
    renderer.draw_icon(
        icon_x,
        icon_y,
        NOTIF_ICON_SIZE,
        NOTIF_ICON_SIZE,
        accent,
        icon_id,
        false,
    );

    // Text
    let text_x = x + NOTIF_TEXT_LEFT;
    let text_w = (w - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0);
    let mut text_y = y + NOTIF_PAD;

    if !title.is_empty() {
        let style = notification_title_style();
        let spans = plain_spans(title);
        let (_, th) = renderer.measure_paragraph(&style, &spans, Some(text_w));
        renderer.draw_paragraph(&style, &spans, text_x, text_y, text_w);
        text_y += th + 2.0;
    }
    if !subtitle.is_empty() {
        let style = notification_subtitle_style();
        let spans = plain_spans(subtitle);
        renderer.draw_paragraph(&style, &spans, text_x, text_y, text_w);
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

fn format_btn_key(id: u32, buf: &mut [u8; 16]) -> &str {
    buf[0..4].copy_from_slice(b"btn_");
    if id == 0 {
        buf[4] = b'0';
        // Safety: buffer contains only ASCII bytes
        return core::str::from_utf8(&buf[0..5]).unwrap_or("btn_0");
    }
    let mut n = id;
    let mut i = 16;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let num_len = 16 - i;
    buf.copy_within(i..16, 4);
    // Safety: buffer contains only ASCII bytes
    core::str::from_utf8(&buf[0..4 + num_len]).unwrap_or("btn_?")
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

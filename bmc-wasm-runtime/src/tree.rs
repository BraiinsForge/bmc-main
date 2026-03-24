// Copyright (C) 2026  Braiins Systems s.r.o.

//! Tree deserialization and layout computation.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

use anyhow::{Result, bail};
use bmc_wasm_protocol::{
    AnimProperty, ColorSpace, DRAW_BITMAP, DRAW_CENTERED, DRAW_CIRCLE, DRAW_ICON, DRAW_MODIFIED,
    DRAW_ORBIT, DRAW_RECT, DRAW_ROTATED, Easing, GRAY_10, GRAY_50, GRAY_70, GRAY_90, GRAY_100,
    GREEN_40, ICON_CLOSE, ICON_ERROR, ICON_INFO, ICON_SUCCESS, ICON_WARNING, LoopMode, NODE_BUTTON,
    NODE_CANVAS, NODE_CENTER, NODE_COLUMN, NODE_MODAL, NODE_NOTIFICATION, NODE_PARAGRAPH, NODE_ROW,
    NODE_SPACER, ORANGE_40, RED_60, VIOLET_50,
};

// Re-export for other modules
pub use bmc_wasm_protocol::{PropsData, TextAlign, TextStyle};

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
        icon_id: u16,
    },
    Spacer {
        flex: f32,
    },
    Canvas(PropsData, Vec<DrawCommand>),
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
        body: Vec<TreeNode>,
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
                let icon_id = self.read_u16()?;
                let len = self.read_u16()?;
                let label = self.read_string(len)?;
                Ok(TreeNode::Button {
                    label,
                    style,
                    icon_id,
                })
            }
            NODE_SPACER => {
                let flex = self.read_f32()?;
                Ok(TreeNode::Spacer { flex })
            }
            NODE_CANVAS => {
                let props = self.read_props()?;
                let draw_count = self.read_u16()?;
                let mut draws = Vec::with_capacity(draw_count as usize);
                for _ in 0..draw_count {
                    draws.push(self.read_draw()?);
                }
                Ok(TreeNode::Canvas(props, draws))
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
                    body,
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
            _ => bail!("unknown node type: {}", node_type),
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
                Ok(DrawCommand::Icon {
                    x,
                    y,
                    w,
                    h,
                    color,
                    icon_id,
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
            _ => bail!("unknown draw command: {}", draw_type),
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

use taffy::prelude::*;

use crate::animation::{apply_easing, compute_animation_value, interpolate_color, multiply_alpha};
use crate::components::{ButtonStyle, draw_button};
use crate::host_api::{AnimationState, ModalState, PrevDrawValues, TransitionState};
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

/// Result from processing a tree
#[derive(Debug, Default)]
pub struct TreeResult {
    /// Click state for each button (in order of appearance)
    pub clicks: Vec<bool>,
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

/// Node data attached to taffy nodes
#[derive(Clone, Default)]
struct NodeContext {
    background: u32,
    paragraph: Option<ParagraphData>,
    button: Option<(u32, String, u8, u16)>, // id, label, style, icon_id
    draws: Vec<DrawCommand>,                // canvas draw commands
    notification: Option<NotificationData>,
}

/// Collected modal info for overlay rendering
struct ModalInfo {
    modal_id: u16,
    is_open: bool,
    padding: u16,
    backdrop_alpha: u8,
    title: String,
    content_height: f32,
    body: Vec<TreeNode>,
    /// Starting button index for this modal's buttons
    button_index_start: u32,
}

/// Process a tree: deserialize, layout, render.
///
/// Returns `(result, has_active_animations)` — caller should request next frame when active.
#[expect(clippy::too_many_arguments, clippy::implicit_hasher)]
pub fn process_tree(
    data: &[u8],
    width: f32,
    height: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<u16, ModalState>,
    animation_states: &mut HashMap<u64, AnimationState>,
    transition_states: &mut HashMap<(u16, u16), TransitionState>,
    frame_counter: u64,
    delta_ms: u32,
) -> Result<(TreeResult, bool)> {
    let tree_node = deserialize_tree(data)?;
    let mut result = TreeResult::default();
    let mut button_id: u32 = 0;
    let mut modals: Vec<ModalInfo> = Vec::new();

    // Build taffy tree (collects modals separately)
    let mut taffy: TaffyTree<NodeContext> = TaffyTree::new();
    let root_id = build_taffy_node(
        &mut taffy,
        &tree_node,
        &mut result,
        &mut button_id,
        &mut modals,
    )?;

    // Set root size
    if let Ok(style) = taffy.style(root_id) {
        let mut new_style = style.clone();
        new_style.size = Size {
            width: length(width),
            height: length(height),
        };
        taffy.set_style(root_id, new_style)?;
    }

    // Compute layout with measure function for paragraphs
    taffy.compute_layout_with_measure(
        root_id,
        Size::MAX_CONTENT,
        |known_dimensions, available_space, _node_id, node_context, _style| {
            // If dimensions are already known, use them
            if let (Some(w), Some(h)) = (known_dimensions.width, known_dimensions.height) {
                return Size {
                    width: w,
                    height: h,
                };
            }

            // Measure paragraphs and notifications based on available width
            if let Some(ctx) = node_context
                && let Some(ref para) = ctx.paragraph
            {
                // Use known width from Taffy if available, else fall back to available_space
                let available_width = known_dimensions.width.or(match available_space.width {
                    AvailableSpace::Definite(w) => Some(w),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });

                // Use explicit max_width if set, otherwise use available width
                let max_width = if para.base_style.max_width > 0 {
                    Some(
                        (para.base_style.max_width as f32).min(available_width.unwrap_or(f32::MAX)),
                    )
                } else {
                    available_width
                };

                let (w, h) = renderer.measure_paragraph(&para.base_style, &para.spans, max_width);
                return Size {
                    width: known_dimensions.width.unwrap_or(w),
                    height: known_dimensions.height.unwrap_or(h),
                };
            }

            // Default for non-paragraph nodes without explicit size
            Size::ZERO
        },
    )?;

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
        &taffy,
        root_id,
        0.0,
        0.0,
        renderer,
        interaction,
        &mut result,
        &mut anim_ctx,
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
            delta_ms,
            &mut result,
        );
    }

    // GC: remove animation/transition states not seen this frame
    anim_ctx
        .animation_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);
    anim_ctx
        .transition_states
        .retain(|_, s| s.last_seen_frame >= frame_counter);

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
                align_items: if is_center {
                    Some(AlignItems::Center)
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
                flex_grow: if is_center && props.flex == 0.0 {
                    1.0
                } else {
                    props.flex
                },
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            if props.background != 0 {
                taffy.set_node_context(
                    id,
                    Some(NodeContext {
                        background: props.background,
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
            icon_id,
        } => {
            let id_num = *button_id;
            *button_id += 1;
            result.clicks.push(false);

            let has_icon = *icon_id != 0;
            let has_label = !label.is_empty();

            let (width, height) = if has_icon && !has_label {
                // Icon-only: square button
                (48.0, 48.0)
            } else if has_icon && has_label {
                // Icon + text: icon 16px + 8px gap + text width + padding
                let text_w = (label.len() as f32 * 10.0).max(40.0);
                (16.0 + 8.0 + text_w + 32.0, 48.0)
            } else {
                // Text-only: unchanged
                ((label.len() as f32 * 10.0).max(120.0) + 32.0, 48.0)
            };

            let style = Style {
                size: Size {
                    width: length(width),
                    height: length(height),
                },
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    button: Some((id_num, label.clone(), *btn_style, *icon_id)),
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

        TreeNode::Canvas(props, draws) => {
            let style = Style {
                size: size_from_props(props),
                flex_grow: props.flex,
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    background: props.background,
                    draws: draws.clone(),
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
    }
}

#[expect(clippy::too_many_arguments)]
fn render_taffy_node(
    taffy: &TaffyTree<NodeContext>,
    node_id: taffy::NodeId,
    parent_x: f32,
    parent_y: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
) {
    let layout = taffy.layout(node_id).unwrap();
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;
    let w = layout.size.width;
    let h = layout.size.height;

    if let Some(ctx) = taffy.get_node_context(node_id) {
        if ctx.background != 0 {
            renderer.fill_rect(x, y, w, h, ctx.background);
        }

        if let Some(ref para) = ctx.paragraph {
            renderer.draw_paragraph(&para.base_style, &para.spans, x, y, w);
        }

        if let Some((btn_id, ref label, style, icon_id)) = ctx.button {
            let mut key_buf = [0_u8; 16];
            let key = format_btn_key(btn_id, &mut key_buf);
            let clicked = draw_button(
                renderer,
                interaction,
                key,
                label,
                x,
                y,
                w,
                h,
                ButtonStyle::from(style as u32),
                icon_id,
            );
            if (btn_id as usize) < result.clicks.len() {
                result.clicks[btn_id as usize] = clicked;
            }
        }

        // Render canvas draw commands with local coordinates
        if !ctx.draws.is_empty() {
            anim_ctx.draw_in_canvas = 0;
            for draw in &ctx.draws {
                render_draw_command(renderer, draw, x, y, w, h, anim_ctx);
                anim_ctx.draw_in_canvas += 1;
            }
            anim_ctx.canvas_index += 1;
        }

        if let Some(ref notif) = ctx.notification {
            render_notification(notif, x, y, w, h, renderer);
        }
    }

    for child_id in taffy.children(node_id).unwrap() {
        render_taffy_node(
            taffy,
            child_id,
            x,
            y,
            renderer,
            interaction,
            result,
            anim_ctx,
        );
    }
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
        | DrawCommand::Bitmap { w, h, .. } => (*w, *h),
        DrawCommand::Circle { r, .. } => (*r * 2.0, *r * 2.0),
        DrawCommand::Centered { inner }
        | DrawCommand::Rotated { inner, .. }
        | DrawCommand::Modified { inner, .. }
        | DrawCommand::Orbit { inner, .. } => get_draw_bounds(inner),
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
                renderer.draw_icon(rx, ry, ew, eh, final_color, *icon_id);
            } else {
                let pivot_x = cx + cw / 2.0;
                let pivot_y = cy + ch / 2.0;
                renderer.save();
                renderer.translate(pivot_x, pivot_y);
                renderer.rotate(rotation);
                renderer.draw_icon(rx - pivot_x, ry - pivot_y, ew, eh, final_color, *icon_id);
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

            // Process animations
            for anim_def in animations {
                let key = animation_key(anim_def, anim_ctx.draw_counter);
                let state =
                    anim_ctx
                        .animation_states
                        .entry(key)
                        .or_insert_with(|| AnimationState {
                            elapsed_ms: 0,
                            forward: true,
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
                }
            }

            anim_ctx.draw_counter += 1;

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
        DrawCommand::Bitmap { x, y, w, h, .. } => PrevDrawValues {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
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
    }
}

/// Count buttons in a tree node (for modal body button allocation)
fn count_tree_buttons(node: &TreeNode, button_id: &mut u32, result: &mut TreeResult) {
    match node {
        TreeNode::Column(_, children)
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
        | TreeNode::Canvas(..)
        | TreeNode::Notification { .. } => {}
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
    delta_ms: u32,
    result: &mut TreeResult,
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
    let modal_bg = GRAY_90;
    renderer.fill_rect(modal_x, modal_y, modal_width, modal_height, modal_bg);

    // Draw header background
    let header_bg = GRAY_100;
    renderer.fill_rect(
        modal_x,
        modal_y,
        modal_width,
        MODAL_HEADER_HEIGHT,
        header_bg,
    );

    // Draw header title
    let title_style = TextStyle {
        size: 16,
        weight: 600,
        color: GRAY_10,
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
        ButtonStyle::Secondary,
        ICON_CLOSE,
    );

    if close_clicked && (close_btn_id as usize) < result.clicks.len() {
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

    // Render body content with scroll offset
    // For now, render body children in a column layout within the body area
    let content_y = body_y + body_padding - state.scroll_offset;
    let clip_top = body_y + body_padding;
    let clip_bottom = body_y + body_padding + body_height;
    render_modal_body(
        &modal.body,
        body_x + body_padding,
        content_y,
        modal_width - body_padding * 2.0,
        body_height,
        renderer,
        interaction,
        result,
        modal.button_index_start,
        clip_top,
        clip_bottom,
    );

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
        TreeNode::Column(_, children)
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
        | TreeNode::Canvas(..)
        | TreeNode::Notification { .. } => 0,
    }
}

/// Render modal body children with clipping
#[expect(clippy::too_many_arguments)]
fn render_modal_body(
    body: &[TreeNode],
    x: f32,
    mut y: f32,
    width: f32,
    _available_height: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
    button_index_start: u32,
    clip_top: f32,
    clip_bottom: f32,
) {
    let mut button_idx = button_index_start;

    for node in body {
        let node_height = render_modal_body_node(
            node,
            x,
            y,
            width,
            renderer,
            interaction,
            result,
            &mut button_idx,
            clip_top,
            clip_bottom,
        );
        y += node_height + 8.0; // 8px gap between items
    }
}

/// Render a single modal body node, returning its height
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_modal_body_node(
    node: &TreeNode,
    x: f32,
    y: f32,
    width: f32,
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
    button_idx: &mut u32,
    clip_top: f32,
    clip_bottom: f32,
) -> f32 {
    match node {
        TreeNode::Paragraph {
            base_style, spans, ..
        } => {
            // Measure first to get actual height (cached)
            let (_, h) = renderer.measure_paragraph(base_style, spans, Some(width));

            // Only render if at least partially visible
            if y < clip_bottom && y + h > clip_top {
                renderer.draw_paragraph_clipped(
                    base_style,
                    spans,
                    x,
                    y,
                    width,
                    clip_top,
                    clip_bottom,
                );
            }
            h
        }
        TreeNode::Button {
            label,
            style,
            icon_id,
        } => {
            let btn_id = *button_idx;
            *button_idx += 1;

            let has_icon = *icon_id != 0;
            let has_label = !label.is_empty();
            let btn_width = if has_icon && !has_label {
                48.0
            } else if has_icon && has_label {
                let text_w = (label.len() as f32 * 10.0).max(40.0);
                16.0 + 8.0 + text_w + 32.0
            } else {
                (label.len() as f32 * 10.0).max(120.0) + 32.0
            };
            let btn_height = 48.0;

            if y < clip_bottom && y + btn_height > clip_top {
                let mut key_buf = [0_u8; 16];
                let key = format_btn_key(btn_id, &mut key_buf);
                let clicked = draw_button(
                    renderer,
                    interaction,
                    key,
                    label,
                    x,
                    y,
                    btn_width,
                    btn_height,
                    ButtonStyle::from(*style as u32),
                    *icon_id,
                );
                if clicked && (btn_id as usize) < result.clicks.len() {
                    result.clicks[btn_id as usize] = true;
                }
            }

            btn_height
        }
        TreeNode::Column(_, children) => {
            let mut total_height = 0.0_f32;
            let mut child_y = y;
            for child in children {
                let h = render_modal_body_node(
                    child,
                    x,
                    child_y,
                    width,
                    renderer,
                    interaction,
                    result,
                    button_idx,
                    clip_top,
                    clip_bottom,
                );
                child_y += h + 8.0;
                total_height += h + 8.0;
            }
            (total_height - 8.0).max(0.0) // Remove last gap
        }
        TreeNode::Row(_, children) => {
            let child_count = children.len().max(1) as f32;
            let child_width = width / child_count;
            let mut max_height = 0.0_f32;
            let mut child_x = x;
            for child in children {
                let h = render_modal_body_node(
                    child,
                    child_x,
                    y,
                    (child_width - 8.0).max(0.0),
                    renderer,
                    interaction,
                    result,
                    button_idx,
                    clip_top,
                    clip_bottom,
                );
                child_x += child_width;
                max_height = max_height.max(h);
            }
            max_height
        }
        TreeNode::Center(..)
        | TreeNode::Spacer { .. }
        | TreeNode::Canvas(..)
        | TreeNode::Modal { .. }
        | TreeNode::Notification { .. } => 0.0,
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

#[expect(dead_code)]
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

    // Background
    renderer.fill_rect(x, y, w, h, GRAY_90);

    // Left accent border
    renderer.fill_rect(x, y, NOTIF_BORDER_W, h, accent);

    // Icon (top-aligned with text)
    let icon_x = x + NOTIF_BORDER_W + NOTIF_PAD;
    let icon_y = y + NOTIF_PAD;
    renderer.draw_icon(
        icon_x,
        icon_y,
        NOTIF_ICON_SIZE,
        NOTIF_ICON_SIZE,
        accent,
        icon_id,
    );

    // Text
    let text_x = x + NOTIF_TEXT_LEFT;
    let text_w = (w - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0);
    let mut text_y = y + NOTIF_PAD;

    if !notif.title.is_empty() {
        let style = notification_title_style();
        let spans = plain_spans(&notif.title);
        let (_, th) = renderer.measure_paragraph(&style, &spans, Some(text_w));
        renderer.draw_paragraph(&style, &spans, text_x, text_y, text_w);
        text_y += th + 2.0;
    }
    if !notif.subtitle.is_empty() {
        let style = notification_subtitle_style();
        let spans = plain_spans(&notif.subtitle);
        renderer.draw_paragraph(&style, &spans, text_x, text_y, text_w);
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

fn format_btn_key(id: u32, buf: &mut [u8; 16]) -> &str {
    buf[0..4].copy_from_slice(b"btn_");
    if id == 0 {
        buf[4] = b'0';
        return core::str::from_utf8(&buf[0..5]).unwrap();
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
    core::str::from_utf8(&buf[0..4 + num_len]).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_props_size() {
        assert_eq!(std::mem::size_of::<PropsData>(), PropsData::SIZE);
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Tree deserialization and layout computation.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

use anyhow::{Result, bail};
use bmc_wasm_protocol::{
    DRAW_CENTERED, DRAW_ORBIT, DRAW_RECT, DRAW_ROTATED, GRAY_10, GRAY_50, GRAY_70, GRAY_90,
    GRAY_100, NODE_BUTTON, NODE_CANVAS, NODE_CENTER, NODE_COLUMN, NODE_MODAL, NODE_PARAGRAPH,
    NODE_ROW, NODE_SPACER,
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
    Rotated {
        angle: f32,
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
    },
    Spacer {
        flex: f32,
    },
    Canvas(PropsData, Vec<DrawCommand>),
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
                let len = self.read_u16()?;
                let label = self.read_string(len)?;
                Ok(TreeNode::Button { label, style })
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
            _ => bail!("unknown node type: {}", node_type),
        }
    }

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

use cosmic_text::{FontSystem, SwashCache};
use taffy::prelude::*;
use tiny_skia::Pixmap;

use crate::components::{ButtonStyle, draw_button};
use crate::drawing::shapes::{fill_rect, fill_rotated_rect};
use crate::drawing::text::{
    ShapedTextCache, measure_paragraph, render_paragraph, render_paragraph_clipped,
};
use crate::host_api::ModalState;
use crate::interaction::InteractionState;

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

/// Node data attached to taffy nodes
#[derive(Clone, Default)]
struct NodeContext {
    background: u32,
    paragraph: Option<ParagraphData>,
    button: Option<(u32, String, u8)>, // id, label, style
    draws: Vec<DrawCommand>,           // canvas draw commands
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

/// Process a tree: deserialize, layout, render
#[expect(clippy::too_many_arguments, clippy::implicit_hasher)]
pub fn process_tree(
    data: &[u8],
    width: u32,
    height: u32,
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<u16, ModalState>,
    delta_ms: u32,
) -> Result<TreeResult> {
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
            width: length(width as f32),
            height: length(height as f32),
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

            // Measure paragraphs based on available width
            if let Some(ctx) = node_context {
                if let Some(ref para) = ctx.paragraph {
                    let available_width = match available_space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        AvailableSpace::MinContent => Some(0.0),
                        AvailableSpace::MaxContent => None,
                    };

                    // Use explicit max_width if set, otherwise use available width
                    let max_width = if para.base_style.max_width > 0 {
                        Some(
                            (para.base_style.max_width as f32)
                                .min(available_width.unwrap_or(f32::MAX)),
                        )
                    } else {
                        available_width
                    };

                    let (w, h) =
                        measure_paragraph(font_system, &para.base_style, &para.spans, max_width);
                    return Size {
                        width: w,
                        height: h,
                    };
                }
            }

            // Default for non-paragraph nodes without explicit size
            Size::ZERO
        },
    )?;

    // Render main tree
    render_taffy_node(
        &taffy,
        root_id,
        0,
        0,
        pixmap,
        font_system,
        swash_cache,
        text_cache,
        interaction,
        &mut result,
    );

    // Render modal overlays
    for modal in &modals {
        render_modal(
            modal,
            width,
            height,
            pixmap,
            font_system,
            swash_cache,
            text_cache,
            interaction,
            modal_states,
            delta_ms,
            &mut result,
        );
    }

    Ok(result)
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
        } => {
            let id_num = *button_id;
            *button_id += 1;
            result.clicks.push(false);

            let width = (label.len() as f32 * 10.0).max(120.0) + 32.0;
            let height = 48.0;

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
                    button: Some((id_num, label.clone(), *btn_style)),
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
    parent_x: i32,
    parent_y: i32,
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
) {
    let layout = taffy.layout(node_id).unwrap();
    let x = parent_x + layout.location.x as i32;
    let y = parent_y + layout.location.y as i32;
    let w = layout.size.width as u32;
    let h = layout.size.height as u32;

    if let Some(ctx) = taffy.get_node_context(node_id) {
        if ctx.background != 0 {
            fill_rect(pixmap, x, y, w, h, ctx.background);
        }

        if let Some(ref para) = ctx.paragraph {
            render_paragraph(
                pixmap,
                font_system,
                swash_cache,
                &para.base_style,
                &para.spans,
                x,
                y,
                w,
            );
        }

        if let Some((btn_id, ref label, style)) = ctx.button {
            let mut key_buf = [0_u8; 16];
            let key = format_btn_key(btn_id, &mut key_buf);
            let clicked = draw_button(
                pixmap,
                font_system,
                swash_cache,
                text_cache,
                interaction,
                key,
                label,
                x,
                y,
                w,
                h,
                ButtonStyle::from(style as u32),
            );
            if (btn_id as usize) < result.clicks.len() {
                result.clicks[btn_id as usize] = clicked;
            }
        }

        // Render canvas draw commands with local coordinates
        for draw in &ctx.draws {
            render_draw_command(pixmap, draw, x, y, w, h);
        }
    }

    for child_id in taffy.children(node_id).unwrap() {
        render_taffy_node(
            taffy,
            child_id,
            x,
            y,
            pixmap,
            font_system,
            swash_cache,
            text_cache,
            interaction,
            result,
        );
    }
}

/// Render a draw command with canvas-local coordinates
fn render_draw_command(
    pixmap: &mut Pixmap,
    draw: &DrawCommand,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
) {
    render_draw_inner(pixmap, draw, cx, cy, cw, ch, 0.0, 0.0, 0.0);
}

/// Get the bounds (width, height) of a draw command
fn get_draw_bounds(draw: &DrawCommand) -> (f32, f32) {
    match draw {
        DrawCommand::Rect { w, h, .. } => (*w, *h),
        DrawCommand::Centered { inner } | DrawCommand::Rotated { inner, .. } => {
            get_draw_bounds(inner)
        }
        DrawCommand::Orbit { inner, .. } => get_draw_bounds(inner),
    }
}

/// Render a draw command with accumulated transforms
#[expect(clippy::too_many_arguments)]
fn render_draw_inner(
    pixmap: &mut Pixmap,
    draw: &DrawCommand,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    offset_x: f32,
    offset_y: f32,
    rotation: f32,
) {
    match draw {
        DrawCommand::Rect { x, y, w, h, color } => {
            let rx = cx + (*x + offset_x) as i32;
            let ry = cy + (*y + offset_y) as i32;
            if rotation == 0.0 {
                fill_rect(pixmap, rx, ry, *w as u32, *h as u32, *color);
            } else {
                fill_rotated_rect(pixmap, rx, ry, *w as u32, *h as u32, rotation, *color);
            }
        }
        DrawCommand::Centered { inner } => {
            // Center the inner command in canvas
            let (iw, ih) = get_draw_bounds(inner);
            let new_offset_x = (cw as f32 - iw) / 2.0;
            let new_offset_y = (ch as f32 - ih) / 2.0;
            render_draw_inner(
                pixmap,
                inner,
                cx,
                cy,
                cw,
                ch,
                new_offset_x,
                new_offset_y,
                rotation,
            );
        }
        DrawCommand::Orbit {
            radius,
            angle,
            inner,
        } => {
            // Position at orbit around canvas center
            let center_offset_x = cw as f32 / 2.0;
            let center_offset_y = ch as f32 / 2.0;
            let (iw, ih) = get_draw_bounds(inner);
            let new_offset_x = center_offset_x + radius * angle.cos() - iw / 2.0;
            let new_offset_y = center_offset_y + radius * angle.sin() - ih / 2.0;
            render_draw_inner(
                pixmap,
                inner,
                cx,
                cy,
                cw,
                ch,
                new_offset_x,
                new_offset_y,
                rotation,
            );
        }
        DrawCommand::Rotated { angle, inner } => {
            // Accumulate rotation
            render_draw_inner(
                pixmap,
                inner,
                cx,
                cy,
                cw,
                ch,
                offset_x,
                offset_y,
                rotation + angle,
            );
        }
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
        TreeNode::Paragraph { .. } | TreeNode::Spacer { .. } | TreeNode::Canvas(..) => {}
    }
}

// Modal rendering constants
const MODAL_HEADER_HEIGHT: u32 = 48;
const MODAL_ANIMATION_OPEN_MS: f32 = 250.0;
const MODAL_ANIMATION_CLOSE_MS: f32 = 180.0;

/// Render a modal overlay
#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cast_possible_wrap
)]
fn render_modal(
    modal: &ModalInfo,
    width: u32,
    height: u32,
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
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
    let padding = u32::from(modal.padding);
    let backdrop_alpha = ((f32::from(modal.backdrop_alpha) / 255.0) * progress * 255.0) as u8;
    let backdrop_color = u32::from_be_bytes([0, 0, 0, backdrop_alpha]);
    fill_rect(pixmap, 0, 0, width, height, backdrop_color);

    // Modal content dimensions
    let modal_width = width.saturating_sub(padding * 2);
    let modal_height = height.saturating_sub(padding * 2);
    // Body height = modal height - header - internal padding (16px top + 16px bottom)
    let body_padding = 16_u32;
    let body_height = modal_height.saturating_sub(MODAL_HEADER_HEIGHT + body_padding * 2);
    state.viewport_height = body_height as f32;

    // Animate content position (slide down from -100px)
    let slide_offset = ((1.0 - progress) * -100.0) as i32;
    let modal_x = padding as i32;
    let modal_y = padding as i32 + slide_offset;

    // Content is always fully opaque - only backdrop animates opacity
    // This prevents ugly alpha blending of text over background content

    // Draw modal background
    let modal_bg = GRAY_90;
    fill_rect(
        pixmap,
        modal_x,
        modal_y,
        modal_width,
        modal_height,
        modal_bg,
    );

    // Draw header background
    let header_bg = GRAY_100;
    fill_rect(
        pixmap,
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
    render_paragraph(
        pixmap,
        font_system,
        swash_cache,
        &title_style,
        &title_spans,
        modal_x + 16,
        modal_y + 12,
        modal_width - 64, // Leave space for close button
    );

    // Draw close button (X icon in top-right)
    let close_btn_x = modal_x + modal_width as i32 - 48;
    let close_btn_y = modal_y;
    let close_btn_size = MODAL_HEADER_HEIGHT;

    // Close button uses the last button index for this modal
    let close_btn_id = modal.button_index_start + count_modal_body_buttons(&modal.body);
    let mut key_buf = [0_u8; 16];
    let close_key = format_btn_key(close_btn_id, &mut key_buf);

    let close_clicked = draw_button(
        pixmap,
        font_system,
        swash_cache,
        text_cache,
        interaction,
        close_key,
        "✕",
        close_btn_x,
        close_btn_y,
        close_btn_size,
        close_btn_size,
        ButtonStyle::Secondary,
    );

    if close_clicked && (close_btn_id as usize) < result.clicks.len() {
        result.clicks[close_btn_id as usize] = true;
    }

    // Body area with scrolling
    let body_x = modal_x;
    let body_y = modal_y + MODAL_HEADER_HEIGHT as i32;

    // Register scroll region for touch handling
    let body_key = format!("modal_{}_body", modal.modal_id);
    let scroll_region = crate::interaction::Rect::new(body_x, body_y, modal_width, body_height);
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
    let max_scroll = (modal.content_height - body_height as f32).max(0.0);
    state.scroll_offset = state.scroll_offset.clamp(0.0, max_scroll);

    // Render body content with scroll offset
    // For now, render body children in a column layout within the body area
    let content_y = body_y + body_padding as i32 - state.scroll_offset as i32;
    let clip_top = body_y + body_padding as i32;
    let clip_bottom = body_y + body_padding as i32 + body_height as i32;
    render_modal_body(
        &modal.body,
        body_x + body_padding as i32,
        content_y,
        modal_width - body_padding * 2,
        body_height,
        pixmap,
        font_system,
        swash_cache,
        text_cache,
        interaction,
        result,
        modal.button_index_start,
        clip_top,
        clip_bottom,
    );

    // Draw scrollbar if content exceeds viewport
    if modal.content_height > body_height as f32 {
        let scrollbar_width = 4_u32;
        let scrollbar_x = modal_x + modal_width as i32 - scrollbar_width as i32 - 4;
        let scrollbar_track_y = body_y + 4;
        let scrollbar_track_height = body_height - 8;

        // Track
        let track_color = GRAY_70;
        fill_rect(
            pixmap,
            scrollbar_x,
            scrollbar_track_y,
            scrollbar_width,
            scrollbar_track_height,
            track_color,
        );

        // Thumb
        let thumb_ratio = body_height as f32 / modal.content_height;
        let thumb_height = (scrollbar_track_height as f32 * thumb_ratio).max(20.0) as u32;
        let scroll_ratio = state.scroll_offset / max_scroll.max(1.0);
        let thumb_y = scrollbar_track_y
            + (scroll_ratio * (scrollbar_track_height - thumb_height) as f32) as i32;

        let thumb_color = GRAY_50;
        fill_rect(
            pixmap,
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
        TreeNode::Paragraph { .. } | TreeNode::Spacer { .. } | TreeNode::Canvas(..) => 0,
    }
}

/// Render modal body children with clipping
#[expect(clippy::too_many_arguments, clippy::cast_possible_wrap)]
fn render_modal_body(
    body: &[TreeNode],
    x: i32,
    mut y: i32,
    width: u32,
    _available_height: u32,
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
    button_index_start: u32,
    clip_top: i32,
    clip_bottom: i32,
) {
    let mut button_idx = button_index_start;

    for node in body {
        let node_height = render_modal_body_node(
            node,
            x,
            y,
            width,
            pixmap,
            font_system,
            swash_cache,
            text_cache,
            interaction,
            result,
            &mut button_idx,
            clip_top,
            clip_bottom,
        );
        y += node_height as i32 + 8; // 8px gap between items
    }
}

/// Render a single modal body node, returning its height
#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    clippy::integer_division
)]
fn render_modal_body_node(
    node: &TreeNode,
    x: i32,
    y: i32,
    width: u32,
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text_cache: &mut ShapedTextCache,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
    button_idx: &mut u32,
    clip_top: i32,
    clip_bottom: i32,
) -> u32 {
    // Skip if completely outside clip region
    // (would need height first, but we'll render and let pixel-level clipping handle it)

    match node {
        TreeNode::Paragraph {
            base_style, spans, ..
        } => {
            // Measure first to get actual height
            let (_, h) = measure_paragraph(font_system, base_style, spans, Some(width as f32));
            let h_i32 = h as i32;

            // Only render if at least partially visible
            if y < clip_bottom && y + h_i32 > clip_top {
                render_paragraph_clipped(
                    pixmap,
                    font_system,
                    swash_cache,
                    base_style,
                    spans,
                    x,
                    y,
                    width,
                    clip_top,
                    clip_bottom,
                );
            }
            h as u32
        }
        TreeNode::Button { label, style } => {
            let btn_id = *button_idx;
            *button_idx += 1;

            let btn_width = (label.len() as u32 * 10).max(120) + 32;
            let btn_height = 48_u32;

            if y < clip_bottom && y + btn_height as i32 > clip_top {
                let mut key_buf = [0_u8; 16];
                let key = format_btn_key(btn_id, &mut key_buf);
                let clicked = draw_button(
                    pixmap,
                    font_system,
                    swash_cache,
                    text_cache,
                    interaction,
                    key,
                    label,
                    x,
                    y,
                    btn_width,
                    btn_height,
                    ButtonStyle::from(*style as u32),
                );
                if clicked && (btn_id as usize) < result.clicks.len() {
                    result.clicks[btn_id as usize] = true;
                }
            }

            btn_height
        }
        TreeNode::Column(_, children) => {
            let mut total_height = 0_u32;
            let mut child_y = y;
            for child in children {
                let h = render_modal_body_node(
                    child,
                    x,
                    child_y,
                    width,
                    pixmap,
                    font_system,
                    swash_cache,
                    text_cache,
                    interaction,
                    result,
                    button_idx,
                    clip_top,
                    clip_bottom,
                );
                child_y += h as i32 + 8;
                total_height += h + 8;
            }
            total_height.saturating_sub(8) // Remove last gap
        }
        TreeNode::Row(_, children) => {
            let child_count = children.len().max(1);
            let child_width = width / child_count as u32;
            let mut max_height = 0_u32;
            let mut child_x = x;
            for child in children {
                let h = render_modal_body_node(
                    child,
                    child_x,
                    y,
                    child_width.saturating_sub(8),
                    pixmap,
                    font_system,
                    swash_cache,
                    text_cache,
                    interaction,
                    result,
                    button_idx,
                    clip_top,
                    clip_bottom,
                );
                child_x += child_width as i32;
                max_height = max_height.max(h);
            }
            max_height
        }
        TreeNode::Center(..)
        | TreeNode::Spacer { .. }
        | TreeNode::Canvas(..)
        | TreeNode::Modal { .. } => 0,
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

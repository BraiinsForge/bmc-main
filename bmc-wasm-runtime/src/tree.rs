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
    DRAW_CENTERED, DRAW_ORBIT, DRAW_RECT, DRAW_ROTATED, NODE_BUTTON, NODE_CANVAS, NODE_CENTER,
    NODE_COLUMN, NODE_PARAGRAPH, NODE_ROW, NODE_SPACER,
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
#[derive(Debug)]
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

use cosmic_text::{FontSystem, SwashCache};
use taffy::prelude::*;
use tiny_skia::Pixmap;

use crate::components::{ButtonStyle, draw_button};
use crate::drawing::shapes::{fill_rect, fill_rotated_rect};
use crate::drawing::text::{measure_paragraph, render_paragraph};
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

/// Process a tree: deserialize, layout, render
pub fn process_tree(
    data: &[u8],
    width: u32,
    height: u32,
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    interaction: &mut InteractionState,
) -> Result<TreeResult> {
    let tree_node = deserialize_tree(data)?;
    let mut result = TreeResult::default();
    let mut button_id: u32 = 0;

    // Build taffy tree
    let mut taffy: TaffyTree<NodeContext> = TaffyTree::new();
    let root_id = build_taffy_node(&mut taffy, &tree_node, &mut result, &mut button_id)?;

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

    // Render
    render_taffy_node(
        &taffy,
        root_id,
        0,
        0,
        pixmap,
        font_system,
        swash_cache,
        interaction,
        &mut result,
    );

    Ok(result)
}

#[expect(clippy::too_many_lines)]
fn build_taffy_node(
    taffy: &mut TaffyTree<NodeContext>,
    node: &TreeNode,
    result: &mut TreeResult,
    button_id: &mut u32,
) -> Result<taffy::NodeId> {
    match node {
        TreeNode::Column(props, children)
        | TreeNode::Row(props, children)
        | TreeNode::Center(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, result, button_id))
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

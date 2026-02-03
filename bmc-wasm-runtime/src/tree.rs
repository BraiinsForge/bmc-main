// Copyright (C) 2025  Braiins Systems s.r.o.

//! Tree deserialization and layout computation.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

use anyhow::{Result, bail};

/// Node type tags (must match SDK)
pub const NODE_COLUMN: u8 = 0;
pub const NODE_ROW: u8 = 1;
pub const NODE_CENTER: u8 = 2;
pub const NODE_TEXT: u8 = 3;
pub const NODE_BUTTON: u8 = 4;
pub const NODE_SPACER: u8 = 5;
pub const NODE_CANVAS: u8 = 6;

// Draw command tags (must match SDK)
pub const DRAW_RECT: u8 = 16;
pub const DRAW_CENTERED: u8 = 17;
pub const DRAW_ORBIT: u8 = 18;
pub const DRAW_ROTATED: u8 = 19;

/// Fixed-size props (32 bytes, must match SDK)
#[derive(Clone, Copy, Default, Debug)]
pub struct PropsData {
    pub padding: f32,
    pub margin: f32,
    pub gap: f32,
    pub background: u32,
    pub width: f32,
    pub height: f32,
    pub flex: f32,
    pub color: u32,
}

impl PropsData {
    pub const SIZE: usize = 32;

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            padding: f32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            margin: f32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            gap: f32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            background: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
            width: f32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            height: f32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            flex: f32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            color: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
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
    Text {
        content: String,
        size: u32,
        color: u32,
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
            NODE_TEXT => {
                let size = self.read_u32()?;
                let color = self.read_u32()?;
                let len = self.read_u16()?;
                let content = self.read_string(len)?;
                Ok(TreeNode::Text {
                    content,
                    size,
                    color,
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

use crate::colors::GRAY_10;
use crate::components::{ButtonStyle, draw_button};
use crate::drawing::shapes::{fill_rect, fill_rotated_rect};
use crate::drawing::text::draw_text;
use crate::interaction::InteractionState;

/// Result from processing a tree
#[derive(Debug, Default)]
pub struct TreeResult {
    /// Click state for each button (in order of appearance)
    pub clicks: Vec<bool>,
}

/// Node data attached to taffy nodes
#[derive(Clone, Default)]
struct NodeContext {
    background: u32,
    text: Option<(String, u32, u32)>,  // content, size, color
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

    // Compute layout
    taffy.compute_layout(root_id, Size::MAX_CONTENT)?;

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

        TreeNode::Text {
            content,
            size,
            color,
        } => {
            let approx_width = content.len() as f32 * (*size as f32 * 0.6);
            let approx_height = *size as f32 * 1.2;
            let style = Style {
                size: Size {
                    width: length(approx_width),
                    height: length(approx_height),
                },
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    text: Some((content.clone(), *size, *color)),
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

        if let Some((ref content, size, color)) = ctx.text {
            let color = if color == 0 { GRAY_10 } else { color };
            draw_text(pixmap, font_system, swash_cache, content, x, y, size, color);
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

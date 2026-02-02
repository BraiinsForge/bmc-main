// Copyright (C) 2025  Braiins Systems s.r.o.

//! Declarative UI primitives for widget development.

use crate::colors::*;
use crate::host::{self, ButtonStyle};
use core::cell::Cell;
use std::string::String;
use std::vec::Vec;
use taffy::prelude::*;

// Frame-local button counter for auto-generated IDs
thread_local! {
    static BUTTON_ID: Cell<u32> = const { Cell::new(0) };
}

fn begin_frame() {
    BUTTON_ID.with(|id| id.set(0));
}

fn next_button_id() -> u32 {
    BUTTON_ID.with(|id| {
        let current = id.get();
        id.set(current + 1);
        current
    })
}

/// Style properties for layout nodes.
/// Use 0 for "unset" on numeric fields (auto size, no background, default color).
#[derive(Clone)]
pub struct Props {
    pub padding: f32,
    pub margin: f32,
    pub gap: f32,
    pub background: u32,
    pub width: f32,
    pub height: f32,
    pub flex: f32,
    pub color: u32,
}

impl Default for Props {
    fn default() -> Self {
        Self {
            padding: 0.0,
            margin: 0.0,
            gap: 0.0,
            background: 0,
            width: 0.0,  // 0 = auto
            height: 0.0, // 0 = auto
            flex: 0.0,
            color: 0, // 0 = default (GRAY_10)
        }
    }
}

/// A UI node in the tree.
pub enum Node {
    Column(Props, Vec<Node>),
    Row(Props, Vec<Node>),
    Center(Props, Vec<Node>),
    Text(String, u32, u32),
    Button(String, ButtonStyle),
    Spacer(f32),
}

/// Column layout (vertical stack).
pub fn col(props: Props, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Column(props, children.into_iter().collect())
}

/// Row layout (horizontal stack).
pub fn row(props: Props, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Row(props, children.into_iter().collect())
}

/// Centered container.
pub fn center(props: Props, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Center(props, children.into_iter().collect())
}

/// Text node.
pub fn text(content: impl Into<String>, size: u32, props: Props) -> Node {
    Node::Text(
        content.into(),
        size,
        if props.color == 0 {
            GRAY_10
        } else {
            props.color
        },
    )
}

/// Button node.
pub fn button(style: ButtonStyle, label: impl Into<String>) -> Node {
    Node::Button(label.into(), style)
}

/// Flexible spacer.
pub fn spacer(flex: f32) -> Node {
    Node::Spacer(flex)
}

/// Render the UI tree to screen.
pub fn render(width: u32, height: u32, root: Node) -> Vec<bool> {
    begin_frame();

    let mut tree: TaffyTree<NodeData> = TaffyTree::new();
    let mut button_clicks = Vec::new();

    let root_id = build_node(&mut tree, &root, &mut button_clicks);

    if let Ok(style) = tree.style(root_id) {
        let mut new_style = style.clone();
        new_style.size = Size {
            width: length(width as f32),
            height: length(height as f32),
        };
        let _ = tree.set_style(root_id, new_style);
    }

    tree.compute_layout(root_id, Size::MAX_CONTENT).unwrap();
    draw_node(&tree, root_id, 0, 0, &mut button_clicks);

    button_clicks
}

#[derive(Clone, Default)]
struct NodeData {
    background: u32, // 0 = no background
    text: Option<(String, u32, u32)>,
    button: Option<(u32, String, ButtonStyle)>,
}

fn build_node(
    tree: &mut TaffyTree<NodeData>,
    node: &Node,
    clicks: &mut Vec<bool>,
) -> taffy::NodeId {
    match node {
        Node::Column(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_node(tree, c, clicks))
                .collect();
            let style = Style {
                flex_direction: FlexDirection::Column,
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                flex_grow: props.flex,
                ..Default::default()
            };
            let id = tree.new_with_children(style, &child_ids).unwrap();
            if props.background != 0 {
                tree.set_node_context(
                    id,
                    Some(NodeData {
                        background: props.background,
                        ..Default::default()
                    }),
                )
                .unwrap();
            }
            id
        }
        Node::Row(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_node(tree, c, clicks))
                .collect();
            let style = Style {
                flex_direction: FlexDirection::Row,
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                flex_grow: props.flex,
                ..Default::default()
            };
            let id = tree.new_with_children(style, &child_ids).unwrap();
            if props.background != 0 {
                tree.set_node_context(
                    id,
                    Some(NodeData {
                        background: props.background,
                        ..Default::default()
                    }),
                )
                .unwrap();
            }
            id
        }
        Node::Center(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_node(tree, c, clicks))
                .collect();
            let style = Style {
                flex_direction: FlexDirection::Column,
                justify_content: Some(JustifyContent::Center),
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                flex_grow: if props.flex == 0.0 { 1.0 } else { props.flex },
                ..Default::default()
            };
            let id = tree.new_with_children(style, &child_ids).unwrap();
            if props.background != 0 {
                tree.set_node_context(
                    id,
                    Some(NodeData {
                        background: props.background,
                        ..Default::default()
                    }),
                )
                .unwrap();
            }
            id
        }
        Node::Text(content, size, color) => {
            let approx_width = content.len() as f32 * (*size as f32 * 0.6);
            let approx_height = *size as f32 * 1.2;
            let style = Style {
                size: Size {
                    width: length(approx_width),
                    height: length(approx_height),
                },
                ..Default::default()
            };
            let id = tree.new_leaf(style).unwrap();
            tree.set_node_context(
                id,
                Some(NodeData {
                    text: Some((content.clone(), *size, *color)),
                    ..Default::default()
                }),
            )
            .unwrap();
            id
        }
        Node::Button(label, style_variant) => {
            let btn_id = next_button_id();
            clicks.push(false);

            let width = (label.len() as f32 * 10.0).max(120.0) + 32.0;
            let height = 48.0;

            let style = Style {
                size: Size {
                    width: length(width),
                    height: length(height),
                },
                ..Default::default()
            };
            let id = tree.new_leaf(style).unwrap();
            tree.set_node_context(
                id,
                Some(NodeData {
                    button: Some((btn_id, label.clone(), *style_variant)),
                    ..Default::default()
                }),
            )
            .unwrap();
            id
        }
        Node::Spacer(flex) => {
            let style = Style {
                flex_grow: *flex,
                ..Default::default()
            };
            tree.new_leaf(style).unwrap()
        }
    }
}

fn draw_node(
    tree: &TaffyTree<NodeData>,
    node_id: taffy::NodeId,
    parent_x: i32,
    parent_y: i32,
    clicks: &mut Vec<bool>,
) {
    let layout = tree.layout(node_id).unwrap();
    let x = parent_x + layout.location.x as i32;
    let y = parent_y + layout.location.y as i32;
    let w = layout.size.width as u32;
    let h = layout.size.height as u32;

    if let Some(data) = tree.get_node_context(node_id) {
        if data.background != 0 {
            host::fill_rect(x, y, w, h, data.background);
        }
        if let Some((ref content, size, color)) = data.text {
            host::draw_text(content.as_bytes(), x, y, size, color);
        }
        if let Some((btn_id, ref label, style)) = data.button {
            let mut key_buf = [0u8; 16];
            let key = format_btn_key(btn_id, &mut key_buf);
            let clicked = host::button(key, label.as_bytes(), x, y, w, h, style);
            if (btn_id as usize) < clicks.len() {
                clicks[btn_id as usize] = clicked;
            }
        }
    }

    for child_id in tree.children(node_id).unwrap() {
        draw_node(tree, child_id, x, y, clicks);
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

fn size_from_props(props: &Props) -> Size<Dimension> {
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

fn format_btn_key(id: u32, buf: &mut [u8; 16]) -> &[u8] {
    buf[0..4].copy_from_slice(b"btn_");
    if id == 0 {
        buf[4] = b'0';
        return &buf[0..5];
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
    &buf[0..4 + num_len]
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Binary tree serialization for host-side layout.
//!
//! Format: Each node is [type:u8][data...]
//! Container nodes: [type][props:32B][child_count:u16][children...]
//! Text: [type][size:u32][color:u32][len:u16][bytes...]
//! Button: [type][style:u8][len:u16][bytes...]
//! Spacer: [type][flex:f32]
//! Canvas: [type][props:32B]

use std::string::String;
use std::vec::Vec;

use crate::host::ButtonStyle;

/// Node type tags
pub const NODE_COLUMN: u8 = 0;
pub const NODE_ROW: u8 = 1;
pub const NODE_CENTER: u8 = 2;
pub const NODE_TEXT: u8 = 3;
pub const NODE_BUTTON: u8 = 4;
pub const NODE_SPACER: u8 = 5;
pub const NODE_CANVAS: u8 = 6;

// Draw command tags (children of canvas)
pub const DRAW_RECT: u8 = 16;
pub const DRAW_CENTERED: u8 = 17;
pub const DRAW_ORBIT: u8 = 18;
pub const DRAW_ROTATED: u8 = 19;

/// Fixed-size props structure (32 bytes)
#[derive(Clone, Copy, Default)]
#[repr(C)]
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

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.padding.to_le_bytes());
        buf[4..8].copy_from_slice(&self.margin.to_le_bytes());
        buf[8..12].copy_from_slice(&self.gap.to_le_bytes());
        buf[12..16].copy_from_slice(&self.background.to_le_bytes());
        buf[16..20].copy_from_slice(&self.width.to_le_bytes());
        buf[20..24].copy_from_slice(&self.height.to_le_bytes());
        buf[24..28].copy_from_slice(&self.flex.to_le_bytes());
        buf[28..32].copy_from_slice(&self.color.to_le_bytes());
        buf
    }
}

/// Tree buffer for serialization
pub struct TreeBuffer {
    data: Vec<u8>,
}

impl TreeBuffer {
    pub fn new() -> Self {
        Self { data: Vec::with_capacity(4096) }
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Get pointer and length for passing to host
    pub fn as_ptr_len(&self) -> (u32, u32) {
        (self.data.as_ptr() as u32, self.data.len() as u32)
    }

    fn write_u8(&mut self, v: u8) {
        self.data.push(v);
    }

    fn write_u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u32(&mut self, v: u32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    fn write_f32(&mut self, v: f32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    fn write_props(&mut self, props: &PropsData) {
        self.data.extend_from_slice(&props.to_bytes());
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    /// Write a column container
    pub fn write_column(&mut self, props: &PropsData, child_count: u16) {
        self.write_u8(NODE_COLUMN);
        self.write_props(props);
        self.write_u16(child_count);
    }

    /// Write a row container
    pub fn write_row(&mut self, props: &PropsData, child_count: u16) {
        self.write_u8(NODE_ROW);
        self.write_props(props);
        self.write_u16(child_count);
    }

    /// Write a center container
    pub fn write_center(&mut self, props: &PropsData, child_count: u16) {
        self.write_u8(NODE_CENTER);
        self.write_props(props);
        self.write_u16(child_count);
    }

    /// Write a text node
    pub fn write_text(&mut self, content: &str, size: u32, color: u32) {
        self.write_u8(NODE_TEXT);
        self.write_u32(size);
        self.write_u32(color);
        let bytes = content.as_bytes();
        self.write_u16(bytes.len() as u16);
        self.write_bytes(bytes);
    }

    /// Write a button node
    pub fn write_button(&mut self, label: &str, style: ButtonStyle) {
        self.write_u8(NODE_BUTTON);
        self.write_u8(style as u8);
        let bytes = label.as_bytes();
        self.write_u16(bytes.len() as u16);
        self.write_bytes(bytes);
    }

    /// Write a spacer node
    pub fn write_spacer(&mut self, flex: f32) {
        self.write_u8(NODE_SPACER);
        self.write_f32(flex);
    }

    /// Write a canvas node with draw children
    pub fn write_canvas(&mut self, props: &PropsData, draw_count: u16) {
        self.write_u8(NODE_CANVAS);
        self.write_props(props);
        self.write_u16(draw_count);
    }

    /// Write a rect draw command (local coords)
    pub fn write_draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        self.write_u8(DRAW_RECT);
        self.write_f32(x);
        self.write_f32(y);
        self.write_f32(w);
        self.write_f32(h);
        self.write_u32(color);
    }
}

impl Default for TreeBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// Global tree buffer for the current frame
std::thread_local! {
    static TREE_BUFFER: std::cell::RefCell<TreeBuffer> = std::cell::RefCell::new(TreeBuffer::new());
}

/// Begin building a tree (clears buffer)
pub fn begin_tree() {
    TREE_BUFFER.with(|buf| buf.borrow_mut().clear());
}

/// Get the serialized tree pointer and length
pub fn finish_tree() -> (u32, u32) {
    TREE_BUFFER.with(|buf| buf.borrow().as_ptr_len())
}

/// Access the tree buffer for writing
pub fn with_buffer<F, R>(f: F) -> R
where
    F: FnOnce(&mut TreeBuffer) -> R,
{
    TREE_BUFFER.with(|buf| f(&mut buf.borrow_mut()))
}

// ============================================================================
// High-level Node API (mirrors ui.rs but serializes to tree buffer)
// ============================================================================

use crate::colors::GRAY_10;
use crate::host;

/// Draw command for canvas children (local coordinates)
#[derive(Clone)]
pub enum Draw {
    /// Rectangle at absolute local position
    Rect { x: f32, y: f32, w: f32, h: f32, color: u32 },
    /// Center any draw command in canvas
    Centered { inner: Box<Draw> },
    /// Position any draw command at orbit around canvas center
    Orbit { radius: f32, angle: f32, inner: Box<Draw> },
    /// Rotate any draw command around its center
    Rotated { angle: f32, inner: Box<Draw> },
}

/// A UI node in the tree (for building before serialization)
pub enum Node {
    Column(PropsData, Vec<Node>),
    Row(PropsData, Vec<Node>),
    Center(PropsData, Vec<Node>),
    Text { content: String, size: u32, color: u32 },
    Button { label: String, style: ButtonStyle },
    Spacer { flex: f32 },
    Canvas(PropsData, Vec<Draw>),
}

/// Result from tree rendering
#[derive(Default)]
pub struct TreeRenderResult {
    /// Click state for each button (in order of appearance)
    pub clicks: Vec<bool>,
}

/// Column layout
pub fn col(props: PropsData, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Column(props, children.into_iter().collect())
}

/// Row layout
pub fn row(props: PropsData, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Row(props, children.into_iter().collect())
}

/// Centered container
pub fn center(props: PropsData, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Center(props, children.into_iter().collect())
}

/// Text node
pub fn text(content: impl Into<String>, size: u32, props: PropsData) -> Node {
    Node::Text {
        content: content.into(),
        size,
        color: if props.color == 0 { GRAY_10 } else { props.color },
    }
}

/// Button node
pub fn button(style: ButtonStyle, label: impl Into<String>) -> Node {
    Node::Button { label: label.into(), style }
}

/// Flexible spacer
pub fn spacer(flex: f32) -> Node {
    Node::Spacer { flex }
}

/// Canvas for custom drawing with draw commands as children
pub fn canvas(props: PropsData, draws: impl IntoIterator<Item = Draw>) -> Node {
    Node::Canvas(props, draws.into_iter().collect())
}

/// Rectangle at local position within canvas
pub fn rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> Draw {
    Draw::Rect { x, y, w, h, color }
}

/// Center any draw command in canvas
pub fn centered(inner: Draw) -> Draw {
    Draw::Centered { inner: Box::new(inner) }
}

/// Position any draw command at orbit around canvas center
pub fn orbit(radius: f32, angle: f32, inner: Draw) -> Draw {
    Draw::Orbit { radius, angle, inner: Box::new(inner) }
}

/// Rotate any draw command around its center
pub fn rotated(angle: f32, inner: Draw) -> Draw {
    Draw::Rotated { angle, inner: Box::new(inner) }
}

/// Serialize a node tree to the buffer
fn serialize_node(buf: &mut TreeBuffer, node: &Node) {
    match node {
        Node::Column(props, children) => {
            buf.write_column(props, children.len() as u16);
            for child in children {
                serialize_node(buf, child);
            }
        }
        Node::Row(props, children) => {
            buf.write_row(props, children.len() as u16);
            for child in children {
                serialize_node(buf, child);
            }
        }
        Node::Center(props, children) => {
            buf.write_center(props, children.len() as u16);
            for child in children {
                serialize_node(buf, child);
            }
        }
        Node::Text { content, size, color } => {
            buf.write_text(content, *size, *color);
        }
        Node::Button { label, style } => {
            buf.write_button(label, *style);
        }
        Node::Spacer { flex } => {
            buf.write_spacer(*flex);
        }
        Node::Canvas(props, draws) => {
            buf.write_canvas(props, draws.len() as u16);
            for draw in draws {
                serialize_draw(buf, draw);
            }
        }
    }
}

/// Serialize a draw command to the buffer
fn serialize_draw(buf: &mut TreeBuffer, draw: &Draw) {
    match draw {
        Draw::Rect { x, y, w, h, color } => {
            buf.write_draw_rect(*x, *y, *w, *h, *color);
        }
        Draw::Centered { inner } => {
            buf.write_u8(DRAW_CENTERED);
            serialize_draw(buf, inner);
        }
        Draw::Orbit { radius, angle, inner } => {
            buf.write_u8(DRAW_ORBIT);
            buf.write_f32(*radius);
            buf.write_f32(*angle);
            serialize_draw(buf, inner);
        }
        Draw::Rotated { angle, inner } => {
            buf.write_u8(DRAW_ROTATED);
            buf.write_f32(*angle);
            serialize_draw(buf, inner);
        }
    }
}

/// Count buttons in the tree
fn count_buttons(node: &Node) -> u32 {
    match node {
        Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
            children.iter().map(count_buttons).sum()
        }
        Node::Button { .. } => 1,
        _ => 0,
    }
}

/// Render UI tree using host-side layout.
/// Returns button clicks.
pub fn render_ui(width: u32, height: u32, root: Node) -> TreeRenderResult {
    let button_count = count_buttons(&root);

    // Serialize tree to buffer
    begin_tree();
    with_buffer(|buf| serialize_node(buf, &root));
    let (ptr, len) = finish_tree();

    // Submit to host for layout and rendering
    host::submit_tree(
        unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) },
        width,
        height,
    );

    // Collect button click results
    let mut result = TreeRenderResult::default();
    for i in 0..button_count {
        result.clicks.push(host::get_click(i));
    }

    result
}
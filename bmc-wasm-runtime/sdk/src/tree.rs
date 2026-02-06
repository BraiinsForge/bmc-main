// Copyright (C) 2026  Braiins Systems s.r.o.

//! Binary tree serialization for host-side layout.
//!
//! Format: Each node is [type:u8][data...]
//! Container nodes: [type][props:32B][child_count:u16][children...]
//! Paragraph: [type][props:32B][text_style:16B][span_count:u16][spans...]
//! Button: [type][style:u8][len:u16][bytes...]
//! Spacer: [type][flex:f32]
//! Canvas: [type][props:32B]

use std::string::String;
use std::vec::Vec;

use bmc_wasm_protocol::{
    AnimProperty, ColorSpace, DRAW_CENTERED, DRAW_MODIFIED, DRAW_ORBIT, DRAW_RECT, DRAW_ROTATED,
    Easing, GRAY_10, LoopMode, NODE_BUTTON, NODE_CANVAS, NODE_CENTER, NODE_COLUMN, NODE_MODAL,
    NODE_PARAGRAPH, NODE_ROW, NODE_SPACER,
};

// Re-export for macro paths
pub use bmc_wasm_protocol::{PropsData, TextStyle};

use crate::host::ButtonStyle;

/// Definition of a single animation (serialized to host).
#[derive(Clone, Debug)]
pub struct AnimationDef {
    pub property: AnimProperty,
    pub from: f32,
    pub to: f32,
    pub duration_ms: u32,
    pub delay_ms: u16,
    pub easing: Easing,
    pub loop_mode: LoopMode,
}

/// Definition of a transition (serialized to host).
#[derive(Clone, Debug)]
pub struct TransitionDef {
    pub duration_ms: u32,
    pub easing: Easing,
}

/// A text span with optional style overrides
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub weight: Option<u16>,
    pub color: Option<u32>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Span {
    /// Serialize span style flags (u16) and optional color
    /// flags bits:
    ///   0-11:  weight (if has_weight)
    ///   12:    has_weight
    ///   13:    has_color (color u32 follows after text)
    ///   14:    italic
    ///   15:    underline
    /// Note: strikethrough is in the extra byte if needed
    pub fn flags(&self) -> u16 {
        let weight_bits = self.weight.unwrap_or(0) & 0xFFF;
        let has_weight = if self.weight.is_some() { 1 << 12 } else { 0 };
        let has_color = if self.color.is_some() { 1 << 13 } else { 0 };
        let italic_bit = if self.italic { 1 << 14 } else { 0 };
        let underline_bit = if self.underline { 1 << 15 } else { 0 };
        weight_bits | has_weight | has_color | italic_bit | underline_bit
    }

    /// Extra flags byte for strikethrough (separate to fit in u16)
    pub fn extra_flags(&self) -> u8 {
        if self.strikethrough { 1 } else { 0 }
    }
}

/// Trait for optional style argument in span()
pub trait IntoSpanStyle {
    fn apply(self, span: &mut Span);
}

impl IntoSpanStyle for () {
    fn apply(self, _span: &mut Span) {}
}

impl IntoSpanStyle for StyleResult {
    fn apply(self, span: &mut Span) {
        let ts = self.0;
        if ts.weight != 400 {
            span.weight = Some(ts.weight);
        }
        if ts.color != GRAY_10 {
            span.color = Some(ts.color);
        }
        span.italic = ts.italic;
        span.underline = ts.underline;
        span.strikethrough = ts.strikethrough;
    }
}

/// Create a text span, optionally with style overrides
///
/// # Examples
/// ```ignore
/// span("plain text", ())
/// span("bold", style!(weight: 700))
/// span("colored", style!(color: RED_50))
/// ```
pub fn span(text: impl Into<String>, style: impl IntoSpanStyle) -> Span {
    let mut s = Span {
        text: text.into(),
        weight: None,
        color: None,
        italic: false,
        underline: false,
        strikethrough: false,
    };
    style.apply(&mut s);
    s
}

/// Tree buffer for serialization
pub struct TreeBuffer {
    data: Vec<u8>,
}

impl TreeBuffer {
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(4096),
        }
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

    /// Write a paragraph node
    /// Format: [NODE_PARAGRAPH][props:32B][text_style:16B][span_count:u16][spans...]
    /// Each span: [flags:u16][extra_flags:u8][len:u16][text bytes...][color:u32 if has_color]
    pub fn write_paragraph(&mut self, props: &PropsData, base_style: &TextStyle, spans: &[Span]) {
        self.write_u8(NODE_PARAGRAPH);
        self.write_props(props);
        self.write_bytes(&base_style.to_bytes());
        self.write_u16(spans.len() as u16);

        for span in spans {
            self.write_u16(span.flags());
            self.write_u8(span.extra_flags());
            let bytes = span.text.as_bytes();
            self.write_u16(bytes.len() as u16);
            self.write_bytes(bytes);
            if let Some(color) = span.color {
                self.write_u32(color);
            }
        }
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

    /// Write a modal node
    /// Format: [NODE_MODAL][modal_id:u16][is_open:u8][padding:u16][backdrop_alpha:u8][title_len:u16][title_bytes...][content_height:f32][child_count:u16][children...]
    pub fn write_modal(
        &mut self,
        modal_id: u16,
        is_open: bool,
        padding: u16,
        backdrop_alpha: u8,
        title: &str,
        content_height: f32,
        child_count: u16,
    ) {
        self.write_u8(NODE_MODAL);
        self.write_u16(modal_id);
        self.write_u8(if is_open { 1 } else { 0 });
        self.write_u16(padding);
        self.write_u8(backdrop_alpha);
        let title_bytes = title.as_bytes();
        self.write_u16(title_bytes.len() as u16);
        self.write_bytes(title_bytes);
        self.write_f32(content_height);
        self.write_u16(child_count);
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

use crate::host;

/// Draw command for canvas children (local coordinates)
#[derive(Clone)]
pub enum Draw {
    /// Rectangle at absolute local position
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
    },
    /// Center any draw command in canvas
    Centered { inner: Box<Draw> },
    /// Position any draw command at orbit around canvas center
    Orbit {
        radius: f32,
        angle: f32,
        inner: Box<Draw>,
    },
    /// Rotate any draw command around its center
    Rotated { angle: f32, inner: Box<Draw> },
    /// Draw with host-computed animations and/or transitions
    Modified {
        animations: Vec<AnimationDef>,
        transition: Option<TransitionDef>,
        color_space: ColorSpace,
        inner: Box<Draw>,
    },
}

impl Draw {
    /// Add a repeating animation to this draw command.
    #[must_use]
    pub fn animate(
        self,
        property: AnimProperty,
        from: f32,
        to: f32,
        duration_ms: u32,
        easing: Easing,
        loop_mode: LoopMode,
    ) -> Self {
        self.animate_delayed(property, from, to, duration_ms, 0, easing, loop_mode)
    }

    /// Add a repeating animation with a start delay.
    #[must_use]
    pub fn animate_delayed(
        self,
        property: AnimProperty,
        from: f32,
        to: f32,
        duration_ms: u32,
        delay_ms: u16,
        easing: Easing,
        loop_mode: LoopMode,
    ) -> Self {
        let def = AnimationDef {
            property,
            from,
            to,
            duration_ms,
            delay_ms,
            easing,
            loop_mode,
        };
        match self {
            Draw::Modified {
                mut animations,
                transition,
                color_space,
                inner,
            } => {
                animations.push(def);
                Draw::Modified {
                    animations,
                    transition,
                    color_space,
                    inner,
                }
            }
            other => Draw::Modified {
                animations: vec![def],
                transition: None,
                color_space: ColorSpace::default(),
                inner: Box::new(other),
            },
        }
    }

    /// Animate between two colors.
    #[must_use]
    pub fn animate_color(
        self,
        from_color: u32,
        to_color: u32,
        duration_ms: u32,
        easing: Easing,
        loop_mode: LoopMode,
    ) -> Self {
        self.animate(
            AnimProperty::Color,
            f32::from_bits(from_color),
            f32::from_bits(to_color),
            duration_ms,
            easing,
            loop_mode,
        )
    }

    /// Add a transition — host smoothly interpolates when static values change.
    #[must_use]
    pub fn transition(self, duration_ms: u32, easing: Easing) -> Self {
        self.transition_with_color_space(duration_ms, easing, ColorSpace::default())
    }

    /// Add a transition with explicit color interpolation space.
    #[must_use]
    pub fn transition_with_color_space(
        self,
        duration_ms: u32,
        easing: Easing,
        color_space: ColorSpace,
    ) -> Self {
        let transition = Some(TransitionDef {
            duration_ms,
            easing,
        });
        match self {
            Draw::Modified {
                animations,
                transition: _,
                color_space: cs,
                inner,
            } => Draw::Modified {
                animations,
                transition,
                color_space: if color_space != ColorSpace::default() {
                    color_space
                } else {
                    cs
                },
                inner,
            },
            other => Draw::Modified {
                animations: Vec::new(),
                transition,
                color_space,
                inner: Box::new(other),
            },
        }
    }
}

/// A UI node in the tree (for building before serialization)
pub enum Node {
    Column(PropsData, Vec<Node>),
    Row(PropsData, Vec<Node>),
    Center(PropsData, Vec<Node>),
    Paragraph {
        props: PropsData,
        base_style: TextStyle,
        spans: Vec<Span>,
    },
    Button {
        label: String,
        style: ButtonStyle,
    },
    Spacer {
        flex: f32,
    },
    Canvas(PropsData, Vec<Draw>),
    /// Modal dialog overlay with title, close button, and scrollable body
    Modal {
        modal_id: u16,
        is_open: bool,
        title: String,
        content_height: f32,
        padding: u16,
        backdrop_alpha: u8,
        body: Vec<Node>,
    },
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

/// Combined text style and layout props for the style!() macro
pub struct StyleResult(pub TextStyle, pub PropsData);

impl From<StyleResult> for TextStyle {
    fn from(sr: StyleResult) -> Self {
        sr.0
    }
}

impl From<StyleResult> for PropsData {
    fn from(sr: StyleResult) -> Self {
        sr.1
    }
}

/// Simple text node with unified styling
pub fn text(content: impl Into<String>, style: StyleResult) -> Node {
    Node::Paragraph {
        props: style.1,
        base_style: style.0,
        spans: vec![span(content, ())],
    }
}

/// Rich paragraph with multiple styled spans
pub fn paragraph(style: StyleResult, spans: impl IntoIterator<Item = Span>) -> Node {
    Node::Paragraph {
        props: style.1,
        base_style: style.0,
        spans: spans.into_iter().map(Into::into).collect(),
    }
}

/// Button node
pub fn button(style: ButtonStyle, label: impl Into<String>) -> Node {
    Node::Button {
        label: label.into(),
        style,
    }
}

/// Flexible spacer
pub fn spacer(flex: f32) -> Node {
    Node::Spacer { flex }
}

/// Canvas for custom drawing with draw commands as children
pub fn canvas(props: PropsData, draws: impl IntoIterator<Item = Draw>) -> Node {
    Node::Canvas(props, draws.into_iter().collect())
}

/// Modal dialog configuration
#[derive(Clone, Default)]
pub struct ModalProps {
    /// Margin around the modal content area in pixels.
    /// This creates space between the modal and screen edges where the
    /// semi-transparent backdrop is visible.
    /// Default: 48 pixels
    pub padding: u16,

    /// Backdrop opacity as 0-255 value (0 = fully transparent, 255 = fully opaque).
    /// The backdrop is the dark overlay behind the modal that dims the background content.
    /// Lower values make more of the background visible through the overlay.
    /// Default: 128 (50% opacity)
    pub backdrop_alpha: u8,
}

impl ModalProps {
    /// Default margin around modal content (48 pixels)
    pub const DEFAULT_PADDING: u16 = 48;
    /// Default backdrop opacity (128 = 50%)
    pub const DEFAULT_BACKDROP_ALPHA: u8 = 128;
}

/// Modal dialog overlay with title, close button, and scrollable body.
///
/// - `modal_id`: Unique ID for state tracking (must be unique per modal instance)
/// - `is_open`: Whether the modal is visible
/// - `title`: Header title text
/// - `content_height`: Estimated total height of body content (for scroll sizing)
/// - `body`: Child nodes for the modal body
///
/// The close button is automatically included in the header. It uses the next
/// available button index after any buttons in the body.
pub fn modal(
    modal_id: u16,
    is_open: bool,
    title: impl Into<String>,
    content_height: f32,
    body: impl IntoIterator<Item = Node>,
) -> Node {
    modal_styled(
        modal_id,
        is_open,
        title,
        content_height,
        ModalProps::default(),
        body,
    )
}

/// Modal dialog with custom styling props.
pub fn modal_styled(
    modal_id: u16,
    is_open: bool,
    title: impl Into<String>,
    content_height: f32,
    props: ModalProps,
    body: impl IntoIterator<Item = Node>,
) -> Node {
    Node::Modal {
        modal_id,
        is_open,
        title: title.into(),
        content_height,
        padding: if props.padding == 0 {
            ModalProps::DEFAULT_PADDING
        } else {
            props.padding
        },
        backdrop_alpha: if props.backdrop_alpha == 0 {
            ModalProps::DEFAULT_BACKDROP_ALPHA
        } else {
            props.backdrop_alpha
        },
        body: body.into_iter().collect(),
    }
}

/// Rectangle at local position within canvas
pub fn rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> Draw {
    Draw::Rect { x, y, w, h, color }
}

/// Center any draw command in canvas
pub fn centered(inner: Draw) -> Draw {
    Draw::Centered {
        inner: Box::new(inner),
    }
}

/// Position any draw command at orbit around canvas center
pub fn orbit(radius: f32, angle: f32, inner: Draw) -> Draw {
    Draw::Orbit {
        radius,
        angle,
        inner: Box::new(inner),
    }
}

/// Rotate any draw command around its center
pub fn rotated(angle: f32, inner: Draw) -> Draw {
    Draw::Rotated {
        angle,
        inner: Box::new(inner),
    }
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
        Node::Paragraph {
            props,
            base_style,
            spans,
        } => {
            buf.write_paragraph(props, base_style, spans);
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
        Node::Modal {
            modal_id,
            is_open,
            title,
            content_height,
            padding,
            backdrop_alpha,
            body,
        } => {
            buf.write_modal(
                *modal_id,
                *is_open,
                *padding,
                *backdrop_alpha,
                title,
                *content_height,
                body.len() as u16,
            );
            for child in body {
                serialize_node(buf, child);
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
        Draw::Orbit {
            radius,
            angle,
            inner,
        } => {
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
        Draw::Modified {
            animations,
            transition,
            color_space,
            inner,
        } => {
            buf.write_u8(DRAW_MODIFIED);
            let mut flags: u8 = 0;
            if !animations.is_empty() {
                flags |= 0x01;
            }
            if transition.is_some() {
                flags |= 0x02;
            }
            flags |= (*color_space as u8) << 2;
            buf.write_u8(flags);

            if !animations.is_empty() {
                buf.write_u8(animations.len() as u8);
                for anim in animations {
                    buf.write_u8(anim.property as u8);
                    buf.write_f32(anim.from);
                    buf.write_f32(anim.to);
                    buf.write_u32(anim.duration_ms);
                    buf.write_u16(anim.delay_ms);
                    buf.write_u8(anim.easing as u8);
                    buf.write_u8(anim.loop_mode as u8);
                }
            }

            if let Some(t) = transition {
                buf.write_u32(t.duration_ms);
                buf.write_u8(t.easing as u8);
            }

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
        // Modal has an implicit close button when open
        Node::Modal { is_open, body, .. } => {
            let body_buttons: u32 = body.iter().map(count_buttons).sum();
            if *is_open {
                body_buttons + 1 // +1 for close button
            } else {
                0 // closed modal contributes no buttons
            }
        }
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

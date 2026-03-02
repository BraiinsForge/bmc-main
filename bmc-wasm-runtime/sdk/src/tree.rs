// Copyright (C) 2026  Braiins Systems s.r.o.

//! Binary tree serialization for host-side layout.
//!
//! Format: Each node is [type:u8][data...]
//! Container nodes: [type][props:32B][child_count:u16][children...]
//! Paragraph: [type][props:32B][text_style:16B][span_count:u16][spans...]
//! Button: [type][style:u8][size:u8][icon_id:u16][len:u16][bytes...]
//! Spacer: [type][flex:f32]
//! Canvas: [type][props:32B]

use std::string::String;
use std::vec::Vec;

use std::cell::RefCell;

use bmc_wasm_protocol::{
    AnimProperty, ColorSpace, DRAW_BITMAP, DRAW_CENTERED, DRAW_CIRCLE, DRAW_ICON, DRAW_MODIFIED,
    DRAW_ORBIT, DRAW_PATH, DRAW_RECT, DRAW_ROTATED, DRAW_SPHERE, DRAW_TEXT, Easing, GRAY_10,
    LoopMode, NODE_BUTTON, NODE_CANVAS, NODE_CENTER, NODE_COLUMN, NODE_MODAL, NODE_NOTIFICATION,
    NODE_PARAGRAPH, NODE_ROW, NODE_SCROLL, NODE_SPACER,
};

// Re-export for macro paths
pub use bmc_wasm_protocol::{PropsData, TextStyle};

use crate::host::{ButtonSize, ButtonStyle};

/// Compiled icon data (output of `include_icon!` proc macro).
///
/// The `data` field contains the compact binary representation of SVG paths
/// produced at compile time. On first use, this data is sent to the host via
/// `host_register_icon()` which returns an opaque ID used for rendering.
pub struct Icon {
    pub data: &'static [u8],
}

// Icon registration — lazy, once per icon per runtime lifetime.
thread_local! {
    static ICON_IDS: RefCell<Vec<(usize, u16)>> = const { RefCell::new(Vec::new()) };
}

/// Register an icon with the host (if not already registered) and return its ID.
///
/// Useful when you need a raw icon ID for `button_with_icon` or `icon_button`.
#[must_use]
pub fn ensure_registered(icon: &Icon) -> u16 {
    ICON_IDS.with(|ids| {
        let mut ids = ids.borrow_mut();
        let key = icon.data.as_ptr() as usize;
        for &(k, id) in ids.iter() {
            if k == key {
                return id;
            }
        }
        let id = host::register_icon(icon.data);
        ids.push((key, id));
        id
    })
}

/// Embedded raster image data (output of `include_bitmap!` proc macro).
///
/// The `data` field contains raw PNG (or other image format) bytes embedded
/// at compile time. On first use, this data is sent to the host via
/// `host_register_bitmap()` which decodes it and uploads the texture to VRAM.
pub struct Bitmap {
    pub data: &'static [u8],
}

// Bitmap registration — lazy, once per bitmap per runtime lifetime.
thread_local! {
    static BITMAP_IDS: RefCell<Vec<(usize, u16)>> = const { RefCell::new(Vec::new()) };
}

/// Register a bitmap with the host (if not already registered) and return its ID.
#[must_use]
pub fn ensure_bitmap_registered(bmp: &Bitmap) -> u16 {
    BITMAP_IDS.with(|ids| {
        let mut ids = ids.borrow_mut();
        let key = bmp.data.as_ptr() as usize;
        for &(k, id) in ids.iter() {
            if k == key {
                return id;
            }
        }
        let id = host::register_bitmap(bmp.data);
        ids.push((key, id));
        id
    })
}

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

/// Path interpolation mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Interpolation {
    /// Straight line segments between points.
    #[default]
    Linear = 0,
    /// Smooth Catmull-Rom spline through all points (host converts to cubic Bézier).
    CatmullRom = 1,
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
    #[must_use]
    pub fn flags(&self) -> u16 {
        let weight_bits = self.weight.unwrap_or(0) & 0xFFF;
        let has_weight = if self.weight.is_some() { 1 << 12 } else { 0 };
        let has_color = if self.color.is_some() { 1 << 13 } else { 0 };
        let italic_bit = if self.italic { 1 << 14 } else { 0 };
        let underline_bit = if self.underline { 1 << 15 } else { 0 };
        weight_bits | has_weight | has_color | italic_bit | underline_bit
    }

    /// Extra flags byte for strikethrough (separate to fit in u16)
    #[must_use]
    pub fn extra_flags(&self) -> u8 {
        u8::from(self.strikethrough)
    }
}

/// Inline notification severity kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NotificationKind {
    Error = 0,
    Warning = 1,
    Success = 2,
    Info = 3,
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(4096),
        }
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Borrow the serialized bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.data
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
    pub fn write_button(
        &mut self,
        label: &str,
        style: ButtonStyle,
        size: ButtonSize,
        icon_id: u16,
    ) {
        self.write_u8(NODE_BUTTON);
        self.write_u8(style as u8);
        self.write_u8(size as u8);
        self.write_u16(icon_id);
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

    /// Write a scroll container
    /// Format: [NODE_SCROLL][scroll_id:u16][props:32B][child_count:u16][children...]
    pub fn write_scroll(&mut self, scroll_id: u16, props: &PropsData, child_count: u16) {
        self.write_u8(NODE_SCROLL);
        self.write_u16(scroll_id);
        self.write_props(props);
        self.write_u16(child_count);
    }

    /// Write a notification node
    /// Format: [NODE_NOTIFICATION][kind:u8][title_len:u16][title_bytes...][subtitle_len:u16][subtitle_bytes...]
    pub fn write_notification(&mut self, kind: NotificationKind, title: &str, subtitle: &str) {
        self.write_u8(NODE_NOTIFICATION);
        self.write_u8(kind as u8);
        let title_bytes = title.as_bytes();
        self.write_u16(title_bytes.len() as u16);
        self.write_bytes(title_bytes);
        let subtitle_bytes = subtitle.as_bytes();
        self.write_u16(subtitle_bytes.len() as u16);
        self.write_bytes(subtitle_bytes);
    }

    /// Write a modal node
    /// Format: [NODE_MODAL][modal_id:u16][is_open:u8][padding:u16][backdrop_alpha:u8][title_len:u16][title_bytes...][content_height:f32][child_count:u16][children...]
    #[expect(clippy::too_many_arguments)]
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
        self.write_u8(u8::from(is_open));
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
    static TREE_BUFFER: RefCell<TreeBuffer> = RefCell::new(TreeBuffer::new());
}

/// Begin building a tree (clears buffer)
pub fn begin_tree() {
    TREE_BUFFER.with(|buf| buf.borrow_mut().clear());
}

/// Submit the serialized tree to the host and clear the buffer.
pub fn submit_and_clear(width: u32, height: u32) {
    TREE_BUFFER.with(|buf| {
        let b = buf.borrow();
        host::submit_tree(b.as_slice(), width, height);
    });
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
    /// Filled circle at absolute local position (cx, cy = center)
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
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
    /// Icon at absolute local position
    Icon {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
        icon_id: u16,
    },
    /// Bitmap (raster image) at absolute local position
    Bitmap {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: u16,
    },
    /// Draw with host-computed animations and/or transitions
    Modified {
        animations: Vec<AnimationDef>,
        transition: Option<TransitionDef>,
        color_space: ColorSpace,
        inner: Box<Draw>,
    },
    /// Variable-length path: polyline (stroked) or polygon (filled),
    /// with optional Catmull-Rom smoothing.
    Path {
        points: Vec<(f32, f32)>,
        color: u32,
        stroke_width: f32,
        closed: bool,
        fill: bool,
        interpolation: Interpolation,
    },
    /// Styled text at an explicit canvas position.
    Text {
        x: f32,
        y: f32,
        text: String,
        style: TextStyle,
    },
    /// 3D sphere: equirectangular texture mapped onto a sphere with optional light shading.
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
}

impl Draw {
    // ── Constructors ────────────────────────────────────────────────────

    /// Rectangle at local position within canvas.
    #[must_use]
    pub fn rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> Self {
        Self::Rect { x, y, w, h, color }
    }

    /// Filled circle at local position within canvas.
    #[must_use]
    pub fn circle(cx: f32, cy: f32, r: f32, color: u32) -> Self {
        Self::Circle { cx, cy, r, color }
    }

    /// Center any draw command in canvas.
    #[must_use]
    pub fn centered(inner: Draw) -> Self {
        Self::Centered {
            inner: Box::new(inner),
        }
    }

    /// Position any draw command at orbit around canvas center.
    #[must_use]
    pub fn orbit(radius: f32, angle: f32, inner: Draw) -> Self {
        Self::Orbit {
            radius,
            angle,
            inner: Box::new(inner),
        }
    }

    /// Rotate any draw command around its center.
    #[must_use]
    pub fn rotated(angle: f32, inner: Draw) -> Self {
        Self::Rotated {
            angle,
            inner: Box::new(inner),
        }
    }

    /// Icon at local position within canvas.
    ///
    /// On first call for a given icon, registers its compiled data with the host.
    /// Subsequent calls reuse the cached host ID — zero per-frame overhead.
    ///
    /// Use `TRANSPARENT` (0) as color to render with original SVG colors,
    /// or pass a color to tint the entire icon.
    #[must_use]
    pub fn icon(x: f32, y: f32, w: f32, h: f32, icon_data: &Icon, color: u32) -> Self {
        let icon_id = ensure_registered(icon_data);
        Self::Icon {
            x,
            y,
            w,
            h,
            color,
            icon_id,
        }
    }

    /// Draw a built-in icon in canvas (no registration needed).
    ///
    /// Use `ICON_CLOSE` or other `ICON_*` constants from the protocol crate.
    #[must_use]
    pub fn icon_builtin(x: f32, y: f32, w: f32, h: f32, icon_id: u16, color: u32) -> Self {
        Self::Icon {
            x,
            y,
            w,
            h,
            color,
            icon_id,
        }
    }

    /// Bitmap (raster image) at local position within canvas.
    ///
    /// On first call for a given bitmap, registers its PNG data with the host
    /// which decodes and uploads the texture to VRAM. Subsequent calls reuse
    /// the cached texture — zero per-frame overhead.
    #[must_use]
    pub fn bitmap(x: f32, y: f32, w: f32, h: f32, bmp: &Bitmap) -> Self {
        let bitmap_id = ensure_bitmap_registered(bmp);
        Self::Bitmap {
            x,
            y,
            w,
            h,
            bitmap_id,
        }
    }

    /// 3D sphere at local position within canvas.
    ///
    /// Renders an equirectangular texture mapped onto a sphere with perspective
    /// projection, camera centered at (center_lat, center_lon), and optional
    /// directional light shading from (light_lat, light_lon).
    ///
    /// The texture **must** use standard equirectangular (PlateCarrée) layout:
    ///
    /// - `u = 0` → lon = -180°, `u = 0.5` → lon = 0° (prime meridian), `u = 1` → lon = +180°
    /// - `v = 0` → lat = +90° (north pole), `v = 1` → lat = -90° (south pole)
    ///
    /// The GPU shader samples using `atan(x,z)` for longitude and `asin(y)` for
    /// latitude, then maps to UV with `u = lon/(2π) + 0.5`, `v = 0.5 - lat/π`.
    /// Any texture that doesn't follow this convention will show misplaced geography.
    ///
    /// `zoom` is the camera distance from the sphere center in units of sphere
    /// radii (unitless). Values must be > 1.0; smaller values zoom in, larger
    /// values zoom out. Typical full-globe values are ~1.6–2.2. If you want a
    /// more intuitive "scale" parameter, remap it before calling `sphere!`.
    ///
    /// Transitions applied via `.transition(...)` will smoothly interpolate
    /// `center_lat`, `center_lon`, `zoom`, and light direction on the host.
    ///
    /// When `atmosphere` is true, adds limb darkening and bluish edge glow.
    ///
    /// Prefer the [`sphere!`] macro for ergonomic call sites.
    #[must_use]
    #[expect(clippy::too_many_arguments)]
    pub fn sphere(
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bmp: &Bitmap,
        center_lat: f32,
        center_lon: f32,
        zoom: f32,
        light: Option<(f32, f32)>,
        atmosphere: bool,
    ) -> Self {
        let bitmap_id = ensure_bitmap_registered(bmp);
        let (light_lat, light_lon) = light.unwrap_or((f32::NAN, f32::NAN));
        Self::Sphere {
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
        }
    }

    /// Styled text at an explicit canvas position.
    ///
    /// Alignment model: `x` is the anchor point.
    /// `Left` = text starts at x, `Center` = centered on x, `Right` = text ends at x.
    ///
    /// Uses the same font and rendering as the layout system's paragraphs.
    ///
    /// # Examples
    /// ```ignore
    /// Draw::text(10.0, 20.0, "Hello", style!(size: 14, color: WHITE))
    /// Draw::text(w / 2.0, 10.0, "Centered", style!(size: 12, color: GRAY_30, align: Center))
    /// ```
    #[must_use]
    pub fn text(x: f32, y: f32, content: impl Into<String>, style: impl Into<TextStyle>) -> Self {
        Self::Text {
            x,
            y,
            text: content.into(),
            style: style.into(),
        }
    }

    /// Path draw command — polyline or polygon with optional Catmull-Rom smoothing.
    ///
    /// Prefer the [`path!`] macro for ergonomic call sites.
    #[must_use]
    pub fn path(
        points: Vec<(f32, f32)>,
        stroke_width: f32,
        color: u32,
        closed: bool,
        fill: bool,
        interpolation: Interpolation,
    ) -> Self {
        Self::Path {
            points,
            color,
            stroke_width,
            closed,
            fill,
            interpolation,
        }
    }

    // ── Modifiers ───────────────────────────────────────────────────────

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
    #[expect(clippy::too_many_arguments)]
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
                color_space: if color_space == ColorSpace::default() {
                    cs
                } else {
                    color_space
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
        size: ButtonSize,
        icon_id: u16,
    },
    Spacer {
        flex: f32,
    },
    Canvas(PropsData, Vec<Draw>),
    /// Inline notification (error/warning/success/info banner)
    Notification {
        kind: NotificationKind,
        title: String,
        subtitle: String,
    },
    /// Scrollable container — clips children and allows vertical scrolling.
    Scroll {
        scroll_id: u16,
        props: PropsData,
        children: Vec<Node>,
    },
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
#[derive(Clone, Copy)]
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
        spans: spans.into_iter().collect(),
    }
}

/// Create a button node (used by the `button!` macro).
pub fn make_button(label: String, style: ButtonStyle, size: ButtonSize, icon_id: u16) -> Node {
    Node::Button {
        label,
        style,
        size,
        icon_id,
    }
}

/// Inline notification banner
pub fn notification(
    kind: NotificationKind,
    title: impl Into<String>,
    subtitle: impl Into<String>,
) -> Node {
    Node::Notification {
        kind,
        title: title.into(),
        subtitle: subtitle.into(),
    }
}

/// Flexible spacer
#[must_use]
pub fn spacer(flex: f32) -> Node {
    Node::Spacer { flex }
}

/// Scrollable container — clips children and allows vertical scrolling.
///
/// - `scroll_id`: Unique ID for state tracking (must be unique per scroll instance)
/// - `props`: Layout props — **must set `height`** for the viewport
/// - `children`: Child nodes (laid out as a column)
pub fn scroll(scroll_id: u16, props: PropsData, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Scroll {
        scroll_id,
        props,
        children: children.into_iter().collect(),
    }
}

/// Canvas for custom drawing with draw commands as children
pub fn canvas(props: PropsData, draws: impl IntoIterator<Item = Draw>) -> Node {
    Node::Canvas(props, draws.into_iter().collect())
}

/// Modal dialog configuration
#[derive(Clone, Copy, Default)]
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

/// Ergonomic sphere construction for canvas draw commands.
///
/// # Without light (full brightness)
/// ```ignore
/// sphere!(&TEXTURE, at: (0.0, 0.0, 400.0, 400.0), center: (lat, lon), zoom: 1.8)
/// ```
///
/// # With directional light
/// ```ignore
/// sphere!(&TEXTURE, at: (0.0, 0.0, 400.0, 400.0), center: (lat, lon), zoom: 1.8,
///     light: (slat, slon))
/// ```
///
/// # With atmosphere (limb darkening + edge glow)
/// ```ignore
/// sphere!(&TEXTURE, at: (0.0, 0.0, 400.0, 400.0), center: (lat, lon), zoom: 1.8, atmosphere)
/// sphere!(&TEXTURE, at: (0.0, 0.0, 400.0, 400.0), center: (lat, lon), zoom: 1.8,
///     light: (slat, slon), atmosphere)
/// ```
#[macro_export]
macro_rules! sphere {
    ($bmp:expr, at: ($x:expr, $y:expr, $w:expr, $h:expr),
     center: ($lat:expr, $lon:expr), zoom: $z:expr) => {
        $crate::Draw::sphere($x, $y, $w, $h, $bmp, $lat, $lon, $z, None, false)
    };
    ($bmp:expr, at: ($x:expr, $y:expr, $w:expr, $h:expr),
     center: ($lat:expr, $lon:expr), zoom: $z:expr, atmosphere) => {
        $crate::Draw::sphere($x, $y, $w, $h, $bmp, $lat, $lon, $z, None, true)
    };
    ($bmp:expr, at: ($x:expr, $y:expr, $w:expr, $h:expr),
     center: ($lat:expr, $lon:expr), zoom: $z:expr,
     light: ($slat:expr, $slon:expr)) => {
        $crate::Draw::sphere(
            $x,
            $y,
            $w,
            $h,
            $bmp,
            $lat,
            $lon,
            $z,
            Some(($slat, $slon)),
            false,
        )
    };
    ($bmp:expr, at: ($x:expr, $y:expr, $w:expr, $h:expr),
     center: ($lat:expr, $lon:expr), zoom: $z:expr,
     light: ($slat:expr, $slon:expr), atmosphere) => {
        $crate::Draw::sphere(
            $x,
            $y,
            $w,
            $h,
            $bmp,
            $lat,
            $lon,
            $z,
            Some(($slat, $slon)),
            true,
        )
    };
}

/// Ergonomic path construction for canvas draw commands.
///
/// # Stroked paths (polylines)
/// ```ignore
/// path!(points, stroke: 4.0, color: WHITE)                // open, linear
/// path!(points, stroke: 4.0, color: BLUE_50, smooth)      // open, Catmull-Rom
/// path!(points, stroke: 2.0, color: WHITE, closed)        // closed outline, linear
/// path!(points, stroke: 2.0, color: WHITE, closed, smooth) // closed outline, smooth
/// ```
///
/// # Filled paths (polygons)
/// ```ignore
/// path!(points, fill, color: SHADE_BLACK)          // filled polygon, linear
/// path!(points, fill, color: SHADE_BLACK, smooth)  // filled polygon, smooth
/// ```
#[macro_export]
macro_rules! path {
    ($pts:expr, stroke: $w:expr, color: $c:expr) => {
        $crate::Draw::path($pts, $w, $c, false, false, $crate::Interpolation::Linear)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, smooth) => {
        $crate::Draw::path(
            $pts,
            $w,
            $c,
            false,
            false,
            $crate::Interpolation::CatmullRom,
        )
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, closed) => {
        $crate::Draw::path($pts, $w, $c, true, false, $crate::Interpolation::Linear)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, closed, smooth) => {
        $crate::Draw::path($pts, $w, $c, true, false, $crate::Interpolation::CatmullRom)
    };
    ($pts:expr, fill, color: $c:expr) => {
        $crate::Draw::path($pts, 0.0, $c, true, true, $crate::Interpolation::Linear)
    };
    ($pts:expr, fill, color: $c:expr, smooth) => {
        $crate::Draw::path($pts, 0.0, $c, true, true, $crate::Interpolation::CatmullRom)
    };
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
        Node::Button {
            label,
            style,
            size,
            icon_id,
        } => {
            buf.write_button(label, *style, *size, *icon_id);
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
        Node::Scroll {
            scroll_id,
            props,
            children,
        } => {
            buf.write_scroll(*scroll_id, props, children.len() as u16);
            for child in children {
                serialize_node(buf, child);
            }
        }
        Node::Notification {
            kind,
            title,
            subtitle,
        } => {
            buf.write_notification(*kind, title, subtitle);
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
        Draw::Circle { cx, cy, r, color } => {
            buf.write_u8(DRAW_CIRCLE);
            buf.write_f32(*cx);
            buf.write_f32(*cy);
            buf.write_f32(*r);
            buf.write_u32(*color);
        }
        Draw::Icon {
            x,
            y,
            w,
            h,
            color,
            icon_id,
        } => {
            buf.write_u8(DRAW_ICON);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*w);
            buf.write_f32(*h);
            buf.write_u32(*color);
            buf.write_u16(*icon_id);
        }
        Draw::Bitmap {
            x,
            y,
            w,
            h,
            bitmap_id,
        } => {
            buf.write_u8(DRAW_BITMAP);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*w);
            buf.write_f32(*h);
            buf.write_u16(*bitmap_id);
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
        Draw::Path {
            points,
            color,
            stroke_width,
            closed,
            fill,
            interpolation,
        } => {
            let mut flags: u8 = 0;
            if *closed {
                flags |= 0x01;
            }
            if *interpolation == Interpolation::CatmullRom {
                flags |= 0x02;
            }
            if *fill {
                flags |= 0x04;
            }
            buf.write_u8(DRAW_PATH);
            buf.write_u8(flags);
            buf.write_u16(points.len() as u16);
            for &(x, y) in points {
                buf.write_f32(x);
                buf.write_f32(y);
            }
            buf.write_u32(*color);
            if !fill {
                buf.write_f32(*stroke_width);
            }
        }
        Draw::Text { x, y, text, style } => {
            buf.write_u8(DRAW_TEXT);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_bytes(&style.to_bytes());
            let bytes = text.as_bytes();
            buf.write_u16(bytes.len() as u16);
            buf.write_bytes(bytes);
        }
        Draw::Sphere {
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
            buf.write_u8(DRAW_SPHERE);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*w);
            buf.write_f32(*h);
            buf.write_u16(*bitmap_id);
            let flags: u8 = u8::from(*atmosphere);
            buf.write_u8(flags);
            buf.write_f32(*center_lat);
            buf.write_f32(*center_lon);
            buf.write_f32(*zoom);
            buf.write_f32(*light_lat);
            buf.write_f32(*light_lon);
        }
    }
}

/// Count buttons in the tree
fn count_buttons(node: &Node) -> u32 {
    match node {
        Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
            children.iter().map(count_buttons).sum()
        }
        Node::Scroll { children, .. } => children.iter().map(count_buttons).sum(),
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
#[must_use]
#[expect(clippy::needless_pass_by_value)] // Node is consumed by serialization
pub fn render_ui(width: u32, height: u32, root: Node) -> TreeRenderResult {
    let button_count = count_buttons(&root);

    // Serialize tree to buffer and submit to host for layout and rendering
    begin_tree();
    with_buffer(|buf| serialize_node(buf, &root));
    submit_and_clear(width, height);

    // Collect button click results
    let mut result = TreeRenderResult::default();
    for i in 0..button_count {
        result.clicks.push(host::get_click(i));
    }

    result
}

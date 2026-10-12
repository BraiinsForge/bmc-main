// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! The decoded tree's node and draw-command types, and builders for them.

use crate::components::skeleton::SkeletonData;
use crate::components::switcher::SwitcherTabData;
use crate::components::{ButtonSize, ButtonStyle};
use crate::gpu::mesh::MeshDrawArgs;
use bmc_wasm_protocol::{
    AnimProperty, ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, AutoFit, BitmapId, Color,
    ColorSpace, Easing, Fill, FontWeight, LoopMode, MeshId, PathPaint, ProgressKind, PropsData,
    RelTimeClamp, RelTimeFormat, SvgId, TagKind, TextStyle,
};

/// A text span with style overrides.
///
/// `Hash` is derived rather than written out, so a field
/// added here reaches the paragraph layout cache's key on its own.
#[derive(Clone, Debug, Hash)]
pub struct SpanData {
    pub text: String,
    pub weight: Option<FontWeight>,
    pub color: Option<Color>,
    /// Font size for this span alone. How it keeps the paragraph's baseline
    /// is `shape_paragraph`'s business, and written up there.
    pub size: Option<u32>,
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
            size: self.size.unwrap_or(base.size),
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
///
/// `id_hash` is the FNV1a-32 digest of the widget's
/// `Draw::transition` id argument.
///
/// The transition state map keys on `(canvas_index, id_hash)`
/// so transition state follows the logical draw across
/// tree-shape changes — adding or removing a sibling
/// does not reshuffle state into the wrong draws.
#[derive(Debug, Clone)]
pub struct HostTransitionDef {
    pub id_hash: u32,
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
        fill: Fill,
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
        fill: Fill,
    },
    Arc {
        cx: f32,
        cy: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        width: f32,
        fill: ArcFill,
        segments: ArcSegments,
        cap: ArcCap,
    },
    Svg {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        icon_id: Option<SvgId>,
        anti_alias: bool,
        /// Per-path fill overrides keyed by the SVG path `id`.
        /// Empty by default; populated by `Draw::svg(...).fill(id, color)`.
        fills: Vec<(String, Color)>,
    },
    Bitmap {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
    },
    Qr {
        x: f32,
        y: f32,
        size: f32,
        dark: Color,
        light: Color,
        quiet_zone: u8,
        text: String,
    },
    Rotated {
        angle: f32,
        inner: Box<DrawCommand>,
    },
    Shadow {
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
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
        paint: PathPaint,
        closed: bool,
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
    CurvedText {
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        text: String,
        style: TextStyle,
    },
    AutofitText {
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
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

/// Host-side slider skin data (deserialized from wire).
#[derive(Clone, Debug)]
pub struct SliderSkinData {
    pub(crate) track: NinePatchData,
    pub(crate) track_h: u16,
    pub(crate) thumb_id: Option<BitmapId>,
    pub(crate) thumb_w: u16,
    pub(crate) thumb_h: u16,
    #[expect(
        dead_code,
        reason = "the wire schema carries a pressed thumb, but host sliders have no pressed-state input"
    )]
    pub(crate) thumb_pressed_id: Option<BitmapId>,
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
        stretch: bool,
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
    /// Host-rendered relative-time label.
    RelTime {
        anchor: i64,
        format: RelTimeFormat,
        clamp: RelTimeClamp,
        style: TextStyle,
    },
    /// Carbon status pill — themed chrome around a content child.
    /// `icon` is resolved at decode (`None` = no icon).
    Tag {
        kind: TagKind,
        icon: Option<SvgId>,
        content: Box<TreeNode>,
    },
    /// Segmented view switcher — rounded pill of icon tabs, one active.
    Switcher {
        active: usize,
        disabled: bool,
        tabs: Vec<SwitcherTabData>,
    },
    /// Loading placeholder holding a value's slot while it loads.
    Skeleton(SkeletonData),
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
        mode: ProgressKind,
        fraction: f32,
        active: bool,
        fill_color: Color,
        track_color: Color,
        bg_color: Color,
        skin: Option<SliderSkinData>,
    },
    /// `child` drawn with every colour scaled by `brightness`,
    /// as [`crate::renderer::Renderer::set_brightness`] scales it, and nested ones multiply.
    /// No layout box of its own: `child` lays out as if unwrapped,
    /// so absolute insets still resolve against the same parent.
    /// A modal inside still draws lit: modals paint after the tree, outside it.
    /// Host-only: the wire format has no encoding for it.
    Dimmed {
        brightness: f32,
        child: Box<TreeNode>,
    },
}

/// `child`, drawn at `brightness`; see [`TreeNode::Dimmed`].
#[must_use]
pub fn dimmed(brightness: f32, child: TreeNode) -> TreeNode {
    TreeNode::Dimmed {
        brightness,
        child: Box::new(child),
    }
}

/// Column layout.
#[must_use]
pub fn col(props: PropsData, children: impl IntoIterator<Item = TreeNode>) -> TreeNode {
    TreeNode::Column(props, children.into_iter().collect())
}

/// Row layout.
#[must_use]
pub fn row(props: PropsData, children: impl IntoIterator<Item = TreeNode>) -> TreeNode {
    TreeNode::Row(props, children.into_iter().collect())
}

/// Centered container.
#[must_use]
pub fn center(props: PropsData, children: impl IntoIterator<Item = TreeNode>) -> TreeNode {
    TreeNode::Center(props, children.into_iter().collect())
}

/// Single-span paragraph.
#[must_use]
pub fn text(content: impl Into<String>, style: TextStyle) -> TreeNode {
    TreeNode::Paragraph {
        props: PropsData::default(),
        base_style: style,
        spans: vec![SpanData {
            text: content.into(),
            weight: None,
            color: None,
            size: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }],
    }
}

/// Create a button node.
#[must_use]
pub fn make_button(
    id: impl Into<String>,
    label: impl Into<String>,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<ButtonSkinData>,
) -> TreeNode {
    TreeNode::Button {
        id: id.into(),
        label: label.into(),
        style: style as u8,
        size: size as u8,
        icon_id,
        disabled,
        stretch: false,
        skin,
    }
}

/// Flexible spacer.
#[must_use]
pub fn spacer(flex: f32) -> TreeNode {
    TreeNode::Spacer { flex }
}

/// A fixed vertical gap: an empty column of the given height. The layout
/// engine has only uniform `padding`/`margin`, so one-sided vertical spacing
/// is added as a sized spacer between siblings.
#[must_use]
pub fn fixed_height(height: f32) -> TreeNode {
    col(
        PropsData {
            height,
            ..Default::default()
        },
        Vec::new(),
    )
}

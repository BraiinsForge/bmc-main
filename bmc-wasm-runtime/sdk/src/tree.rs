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

//! Binary tree serialization for host-side layout.
//!
//! Format: each node is `[type:u8][data...]`. Per-variant layout:
//!
//! ```text
//! Container: [type][props][child_count:u16][children...]
//! Paragraph: [type][props][text_style][span_count:u16][spans...]
//! Button:    [type][id_len:u16][id_bytes...][style:u8][size:u8]
//!            [icon_id:u16][label_len:u16][label_bytes...]
//! Spacer:    [type][flex:f32]
//! Canvas:    [type][props]
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::string::String;
use std::vec::Vec;

use bmc_wasm_protocol::{
    AnimProperty, ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, AutoFit, BLACK, BitmapId,
    Color, ColorSpace, DRAW_ARC, DRAW_AUTOFIT_TEXT, DRAW_BITMAP, DRAW_CENTERED, DRAW_CIRCLE,
    DRAW_CURVED_TEXT, DRAW_ICON, DRAW_MESH, DRAW_MODIFIED, DRAW_NINE_PATCH, DRAW_ORBIT, DRAW_PATH,
    DRAW_QR, DRAW_RECT, DRAW_ROTATED, DRAW_SHADOW, DRAW_SPHERE, DRAW_TEXT, DROP_SHADOW_BLUR_MAX,
    Dash, Easing, Fill, LoopMode, MeshId, NODE_BUTTON, NODE_CANVAS, NODE_CENTER, NODE_COLUMN,
    NODE_MODAL, NODE_NOTIFICATION, NODE_PARAGRAPH, NODE_PROGRESS_BAR, NODE_RELTIME, NODE_ROW,
    NODE_SCROLL, NODE_SPACER, NODE_SWITCHER, NODE_TAG, PathPaint, ProgressKind, RelTimeClamp,
    RelTimeFormat, SvgId, TagIconMode, TagKind, WHITE, encode_arc_cap, encode_arc_fill,
    encode_arc_segments, encode_fill,
};

use crate::PropsFieldValue;
use crate::mesh::{Mesh, MeshView};

// Re-export for macro paths
pub use bmc_wasm_protocol::{PropsData, TextStyle};

pub use bmc_render_skin::{
    ButtonSkin, NinePatch, NinePatchAsset, Skin, SkinAsset, SkinEntry, SliderSkin,
    ensure_nine_patch_registered,
};

use crate::host::{ButtonSize, ButtonStyle};

// Re-export extracted modules so `tree::*` still covers everything
pub use crate::assets::*;
pub use crate::modal::*;
pub use crate::notification::*;
pub use crate::progress_bar::*;
pub use crate::relative_time::*;
pub use crate::status_overlay::*;
pub use crate::switcher::*;
pub use crate::tag::*;
pub use crate::text::*;

/// Tree buffer for serialization
#[derive(Debug)]
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

    /// Consume the buffer and return the serialized bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
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

    fn write_i64(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    fn write_color(&mut self, c: Color) {
        self.data.extend_from_slice(&c.to_u32().to_le_bytes());
    }

    fn write_fill(&mut self, fill: &Fill) {
        encode_fill(&mut self.data, fill);
    }

    fn write_arc_fill(&mut self, fill: &ArcFill) {
        encode_arc_fill(&mut self.data, fill);
    }

    fn write_arc_segments(&mut self, segments: &ArcSegments) {
        encode_arc_segments(&mut self.data, segments);
    }

    fn write_arc_cap(&mut self, cap: ArcCap) {
        encode_arc_cap(&mut self.data, cap);
    }

    fn write_icon_id(&mut self, id: Option<SvgId>) {
        self.write_u16(id.map_or(0, SvgId::to_wire));
    }

    fn write_bitmap_id(&mut self, id: Option<BitmapId>) {
        self.write_u16(id.map_or(0, BitmapId::to_wire));
    }

    fn write_mesh_id(&mut self, id: Option<MeshId>) {
        self.write_u16(id.map_or(0, MeshId::to_wire));
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

    /// Write a paragraph node.
    ///
    /// ```text
    /// [NODE_PARAGRAPH][props][text_style][span_count:u16][spans...]
    /// each span: [flags:u16][extra_flags:u8][len:u16][text bytes...][color:u32 if has_color]
    /// ```
    pub fn write_paragraph(&mut self, props: &PropsData, base_style: &TextStyle, spans: &[Span]) {
        self.write_u8(NODE_PARAGRAPH);
        self.write_props(props);
        self.write_bytes(&base_style.to_bytes());
        self.write_u16(u16::try_from(spans.len()).expect("BUG: text spans exceed u16::MAX"));

        for span in spans {
            self.write_u16(span.flags());
            self.write_u8(span.extra_flags());
            let bytes = span.text.as_bytes();
            self.write_u16(
                u16::try_from(bytes.len()).expect("BUG: span text exceeds u16::MAX bytes"),
            );
            self.write_bytes(bytes);
            if let Some(color) = span.color {
                self.write_color(color);
            }
        }
    }

    /// Wire format: `[NODE_BUTTON][id_len:u16][id_bytes...][style:u8][size:u8][icon_id:u16][disabled:u8][stretch:u8][label_len:u16][label_bytes...]`
    #[expect(
        clippy::too_many_arguments,
        reason = "one positional arg per button wire field"
    )]
    pub fn write_button(
        &mut self,
        id: &str,
        label: &str,
        style: ButtonStyle,
        size: ButtonSize,
        icon_id: Option<SvgId>,
        disabled: bool,
        stretch: bool,
    ) {
        self.write_u8(NODE_BUTTON);
        let id_bytes = id.as_bytes();
        self.write_u16(u16::try_from(id_bytes.len()).expect("BUG: text id exceeds u16::MAX bytes"));
        self.write_bytes(id_bytes);
        self.write_u8(style as u8);
        self.write_u8(size as u8);
        self.write_icon_id(icon_id);
        self.write_u8(u8::from(disabled));
        self.write_u8(u8::from(stretch));
        let bytes = label.as_bytes();
        self.write_u16(u16::try_from(bytes.len()).expect("BUG: text exceeds u16::MAX bytes"));
        self.write_bytes(bytes);
    }

    /// Write a spacer node
    pub fn write_spacer(&mut self, flex: f32) {
        self.write_u8(NODE_SPACER);
        self.write_f32(flex);
    }

    /// Write a canvas node with draw children
    ///
    /// Wire format: `[NODE_CANVAS][props][key_len:u16][key_bytes...][draw_count:u16][draws...]`
    pub fn write_canvas(&mut self, props: &PropsData, touch_key: Option<&str>, draw_count: u16) {
        self.write_u8(NODE_CANVAS);
        self.write_props(props);
        if let Some(key) = touch_key {
            self.write_u16(
                u16::try_from(key.len()).expect("BUG: touch key exceeds u16::MAX bytes"),
            );
            self.data.extend_from_slice(key.as_bytes());
        } else {
            self.write_u16(0);
        }
        self.write_u16(draw_count);
    }

    /// Write a scroll container.
    ///
    /// ```text
    /// [NODE_SCROLL][key_len:u16][key_bytes...][props][child_count:u16][children...]
    /// ```
    pub fn write_scroll(&mut self, scroll_key: &str, props: &PropsData, child_count: u16) {
        self.write_u8(NODE_SCROLL);
        self.write_u16(
            u16::try_from(scroll_key.len()).expect("BUG: scroll key exceeds u16::MAX bytes"),
        );
        self.data.extend_from_slice(scroll_key.as_bytes());
        self.write_props(props);
        self.write_u16(child_count);
    }

    /// Write a notification node.
    ///
    /// ```text
    /// [NODE_NOTIFICATION][kind:u8][title_len:u16][title_bytes...]
    ///                    [subtitle_len:u16][subtitle_bytes...]
    /// ```
    pub fn write_notification(&mut self, kind: NotificationKind, title: &str, subtitle: &str) {
        self.write_u8(NODE_NOTIFICATION);
        self.write_u8(kind as u8);
        let title_bytes = title.as_bytes();
        self.write_u16(
            u16::try_from(title_bytes.len()).expect("BUG: title exceeds u16::MAX bytes"),
        );
        self.write_bytes(title_bytes);
        let subtitle_bytes = subtitle.as_bytes();
        self.write_u16(
            u16::try_from(subtitle_bytes.len()).expect("BUG: subtitle exceeds u16::MAX bytes"),
        );
        self.write_bytes(subtitle_bytes);
    }

    /// Write a relative-time node.
    ///
    /// ```text
    /// [NODE_RELTIME][anchor:i64][format:u8][clamp:u8][text_style bytes]
    /// ```
    pub fn write_relative_time(
        &mut self,
        anchor: i64,
        format: RelTimeFormat,
        clamp: RelTimeClamp,
        style: &TextStyle,
    ) {
        self.write_u8(NODE_RELTIME);
        self.write_i64(anchor);
        self.write_u8(u8::from(format));
        self.write_u8(clamp as u8);
        self.write_bytes(&style.to_bytes());
    }

    /// Write a tag node's chrome; the content child is serialized after it.
    ///
    /// ```text
    /// [NODE_TAG][kind:u8][icon_mode:u8][icon:u16][content node]
    /// ```
    pub fn write_tag(&mut self, kind: TagKind, icon: TagIcon) {
        self.write_u8(NODE_TAG);
        self.write_u8(kind as u8);
        let (mode, icon_id) = match icon {
            TagIcon::Default => (TagIconMode::Default, None),
            TagIcon::Hidden => (TagIconMode::Hidden, None),
            TagIcon::Custom(id) => (TagIconMode::Custom, Some(id)),
        };
        self.write_u8(mode as u8);
        self.write_icon_id(icon_id);
    }

    /// Write a view-switcher node.
    ///
    /// ```text
    /// [NODE_SWITCHER][active:u8][disabled:u8][tab_count:u8]
    ///   per tab: [icon:u16][click_id_len:u16][click_id_bytes...]
    /// ```
    pub fn write_switcher(&mut self, active: usize, disabled: bool, tabs: &[SwitcherTab]) {
        self.write_u8(NODE_SWITCHER);
        self.write_u8(u8::try_from(active).expect("BUG: active tab index exceeds u8::MAX"));
        self.write_u8(u8::from(disabled));
        self.write_u8(u8::try_from(tabs.len()).expect("BUG: switcher tabs exceed u8::MAX"));
        for tab in tabs {
            self.write_icon_id(tab.icon);
            let bytes = tab.click_id.as_bytes();
            self.write_u16(
                u16::try_from(bytes.len()).expect("BUG: tab click id exceeds u16::MAX bytes"),
            );
            self.write_bytes(bytes);
        }
    }

    /// Write a modal node.
    ///
    /// ```text
    /// [NODE_MODAL][id_len:u16][id_bytes...][is_open:u8][padding:u16]
    ///             [backdrop_alpha:u8][title_len:u16][title_bytes...]
    ///             [content_height:f32][child_count:u16][children...]
    /// ```
    #[expect(clippy::too_many_arguments)]
    pub fn write_modal(
        &mut self,
        modal_id: &str,
        is_open: bool,
        padding: u16,
        backdrop_alpha: u8,
        title: &str,
        content_height: f32,
        child_count: u16,
    ) {
        self.write_u8(NODE_MODAL);
        let id_bytes = modal_id.as_bytes();
        self.write_u16(
            u16::try_from(id_bytes.len()).expect("BUG: modal id exceeds u16::MAX bytes"),
        );
        self.write_bytes(id_bytes);
        self.write_u8(u8::from(is_open));
        self.write_u16(padding);
        self.write_u8(backdrop_alpha);
        let title_bytes = title.as_bytes();
        self.write_u16(
            u16::try_from(title_bytes.len()).expect("BUG: modal title exceeds u16::MAX bytes"),
        );
        self.write_bytes(title_bytes);
        self.write_f32(content_height);
        self.write_u16(child_count);
    }

    /// Write a progress bar node.
    ///
    /// ```text
    /// [NODE_PROGRESS_BAR][key_len:u16][key_bytes...][track_h:f32]
    ///                    [mode:u8][fraction:f32][active:u8]
    ///                    [fill_color:u32][track_color:u32][bg_color:u32]
    /// ```
    #[expect(clippy::too_many_arguments)]
    pub fn write_progress_bar(
        &mut self,
        touch_key: &str,
        track_h: f32,
        mode: &ProgressMode,
        active: bool,
        fill_color: Color,
        track_color: Color,
        bg_color: Color,
        skin: &Option<SliderSkin>,
    ) {
        self.write_u8(NODE_PROGRESS_BAR);
        let key_bytes = touch_key.as_bytes();
        self.write_u16(
            u16::try_from(key_bytes.len()).expect("BUG: progress touch key exceeds u16::MAX bytes"),
        );
        self.write_bytes(key_bytes);
        self.write_f32(track_h);
        match mode {
            ProgressMode::Slider(f) => {
                self.write_u8(ProgressKind::Slider.into());
                self.write_f32(*f);
            }
            ProgressMode::Indeterminate => {
                self.write_u8(ProgressKind::Indeterminate.into());
                self.write_f32(0.0); // unused, keeps format fixed-size
            }
            ProgressMode::Meter(f) => {
                self.write_u8(ProgressKind::Meter.into());
                self.write_f32(*f);
            }
        }
        self.write_u8(u8::from(active));
        self.write_color(fill_color);
        self.write_color(track_color);
        self.write_color(bg_color);
        // Optional slider skin
        if let Some(sk) = skin {
            self.write_u8(1);
            // Track 9-patch: bitmap_id + insets
            self.write_bitmap_id(sk.track.bitmap_id);
            self.write_u16(sk.track.left);
            self.write_u16(sk.track.top);
            self.write_u16(sk.track.right);
            self.write_u16(sk.track.bottom);
            self.write_u16(sk.track_h);
            // Thumb
            self.write_bitmap_id(sk.thumb_id);
            self.write_u16(sk.thumb_w);
            self.write_u16(sk.thumb_h);
            self.write_bitmap_id(sk.thumb_pressed_id);
        } else {
            self.write_u8(0);
        }
    }

    /// Write a rect draw command (local coords)
    pub fn write_draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &Fill) {
        self.write_u8(DRAW_RECT);
        self.write_f32(x);
        self.write_f32(y);
        self.write_f32(w);
        self.write_f32(h);
        self.write_fill(fill);
    }
}

impl Default for TreeBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// Global tree buffer for the current frame
thread_local! {
    static TREE_BUFFER: RefCell<TreeBuffer> = RefCell::new(TreeBuffer::new());
}

/// Begin building a tree (clears buffer)
#[cfg(target_arch = "wasm32")]
pub fn begin_tree() {
    use std::sync::Once;
    static SKIN_INIT: Once = Once::new();
    SKIN_INIT.call_once(|| bmc_render_skin::init(host::register_bitmap_nearest));

    TREE_BUFFER.with(|buf| buf.borrow_mut().clear());
}

/// Submit the serialized tree to the host and clear the buffer.
#[cfg(target_arch = "wasm32")]
pub fn submit_and_clear(width: u32, height: u32) {
    TREE_BUFFER.with(|buf| {
        let b = buf.borrow();
        host::submit_tree(b.as_slice(), width, height);
    });
}

/// Serialize a `Node` to its wire-format bytes (native path for storybook).
///
/// Returns the serialized tree buffer that can be passed to
/// `bmc_render::deserialize_tree()` on the host side.
#[must_use]
pub fn serialize_node_to_bytes(node: &Node) -> Vec<u8> {
    let mut buf = TreeBuffer::new();
    serialize_node(&mut buf, node);
    buf.into_bytes()
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

use crate::TouchHit;
#[cfg(target_arch = "wasm32")]
use crate::host;

/// Parameters for `Draw::with_drop_shadow`.
/// Mirrors CSS `drop-shadow(dx dy blur color)`.
#[derive(Clone, Copy, Debug)]
pub struct DropShadow {
    /// Horizontal offset in screen pixels (positive = right).
    pub dx: f32,
    /// Vertical offset in screen pixels (positive = down).
    pub dy: f32,
    /// Gaussian-blur sigma in screen pixels. Capped at `DROP_SHADOW_BLUR_MAX`.
    pub blur: f32,
    /// Shadow colour, including alpha.
    pub color: Color,
}

/// Styling for a [`Draw::qr`] code.
#[derive(Clone, Copy, Debug)]
pub struct QrStyle {
    /// Colour of the "on" modules.
    pub dark: Color,
    /// Colour of the "off" modules and the quiet-zone margin.
    pub light: Color,
    /// Blank margin around the code, in modules (spec minimum is 4).
    pub quiet_zone: u8,
}

impl Default for QrStyle {
    fn default() -> Self {
        Self {
            dark: BLACK,
            light: WHITE,
            quiet_zone: 2,
        }
    }
}

/// Draw command for canvas children (local coordinates)
#[derive(Clone, Debug)]
pub enum Draw {
    /// Rectangle at absolute local position
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Fill,
    },
    /// Filled circle at absolute local position (cx, cy = center)
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Fill,
    },
    /// Stroked circular arc at absolute local position.
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
    /// Drop shadow behind any draw command — see [`DropShadow`].
    Shadow {
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: Box<Draw>,
    },
    /// Svg at absolute local position
    Svg {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        icon_id: Option<SvgId>,
        anti_alias: bool,
        /// Per-path fill colour overrides, keyed by the path's `id`
        /// attribute. Built via the chained `.fill(id, color)` setter.
        /// Empty `Vec` is the default (no overrides) so the underlying
        /// SVG colours flow through.
        fills: Vec<(String, Color)>,
    },
    /// Bitmap (raster image) at absolute local position
    Bitmap {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
    },
    /// QR code encoding `text`, rasterised host-side into a `size`×`size`
    /// square. The wasm side sends only the text + style; the host owns the
    /// encoding (`qrcodegen`) and the rendering, so no matrix rides the wire.
    Qr {
        x: f32,
        y: f32,
        /// Full footprint in px, quiet zone included; square.
        size: f32,
        style: QrStyle,
        text: String,
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
        paint: PathPaint,
        closed: bool,
        interpolation: Interpolation,
    },
    /// Styled text at an explicit canvas position.
    Text {
        x: f32,
        y: f32,
        text: String,
        style: TextStyle,
    },
    /// Styled text laid out on a circular arc.
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
    /// Text scaled to fit an explicit `(box_width, box_height)` rectangle.
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
    /// 9-patch bitmap: sliced into 9 quads, corners stay fixed, edges stretch.
    NinePatch {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        nine_patch: NinePatch,
    },
    /// 3D sphere: equirectangular texture mapped onto a sphere with optional light shading.
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
    /// 3D mesh rendered via GPU with quaternion-based orientation.
    Mesh {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mesh_id: Option<MeshId>,
        fov: f32,
        distance: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        px: f32,
        py: f32,
        pz: f32,
        scale: f32,
        light_pitch: f32,
        light_yaw: f32,
        ambient: f32,
        specular: f32,
        // UV-rect highlight (NaN = disabled)
        hl_u_min: f32,
        hl_v_min: f32,
        hl_u_max: f32,
        hl_v_max: f32,
        hl_r: f32,
        hl_g: f32,
        hl_b: f32,
    },
}

impl Draw {
    // ── Constructors ────────────────────────────────────────────────────

    /// Rectangle at local position within canvas.
    #[must_use]
    pub fn rect(x: f32, y: f32, w: f32, h: f32, fill: impl Into<Fill>) -> Self {
        Self::Rect {
            x,
            y,
            w,
            h,
            fill: fill.into(),
        }
    }

    /// Filled circle at local position within canvas.
    #[must_use]
    pub fn circle(cx: f32, cy: f32, r: f32, fill: impl Into<Fill>) -> Self {
        Self::Circle {
            cx,
            cy,
            r,
            fill: fill.into(),
        }
    }

    /// Stroked circular arc at local position within canvas.
    #[must_use]
    #[expect(clippy::too_many_arguments, reason = "arc geometry is irreducible")]
    pub fn arc(
        cx: f32,
        cy: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        width: f32,
        fill: impl Into<ArcFill>,
        segments: ArcSegments,
        cap: ArcCap,
    ) -> Self {
        Self::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill: fill.into(),
            segments,
            cap,
        }
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

    /// Wrap this draw in a screen-space drop shadow, replacing
    /// any prior shadow (shadows do not stack).
    /// `blur` is clamped to `[0.0, DROP_SHADOW_BLUR_MAX]`;
    /// over-cap values also fire a debug assert.
    #[must_use]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "any non-Shadow Draw is wrapped identically; new variants don't change the wrap"
    )]
    pub fn with_drop_shadow(self, shadow: DropShadow) -> Self {
        debug_assert!(
            shadow.blur <= DROP_SHADOW_BLUR_MAX,
            "BUG: drop-shadow blur {} exceeds cap {DROP_SHADOW_BLUR_MAX}",
            shadow.blur,
        );
        let blur = shadow.blur.clamp(0.0, DROP_SHADOW_BLUR_MAX);
        let inner = match self {
            Self::Shadow { inner, .. } => inner,
            other => Box::new(other),
        };
        Self::Shadow {
            dx: shadow.dx,
            dy: shadow.dy,
            blur,
            color: shadow.color,
            inner,
        }
    }

    /// Svg at local position within canvas.
    ///
    /// On first call for a given icon, registers its compiled data with the host.
    /// Subsequent calls reuse the cached host ID — zero per-frame overhead.
    ///
    /// Use `TRANSPARENT` (0) as color to render with original SVG colors,
    /// or pass a color to tint the entire icon.
    #[must_use]
    pub fn svg(x: f32, y: f32, w: f32, h: f32, icon_data: &Svg, color: Color) -> Self {
        let icon_id = ensure_registered(icon_data);
        Self::Svg {
            x,
            y,
            w,
            h,
            color,
            icon_id,
            anti_alias: false,
            fills: Vec::new(),
        }
    }

    /// Draw a built-in icon in canvas (no registration needed).
    ///
    /// Use `ICON_CLOSE` or other `ICON_*` constants from the protocol crate.
    #[must_use]
    pub fn svg_builtin(
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        icon_id: impl Into<Option<SvgId>>,
        color: Color,
    ) -> Self {
        Self::Svg {
            x,
            y,
            w,
            h,
            color,
            icon_id: icon_id.into(),
            anti_alias: false,
            fills: Vec::new(),
        }
    }

    /// Fit `svg` inside a `size`×`size` box preserving its aspect ratio and
    /// centering it — matching a browser's default `xMidYMid meet`. Because the
    /// host scales X and Y independently, drawing a non-square glyph straight
    /// into a square box via [`Draw::svg`] would stretch it; this reads the
    /// glyph's [`Svg::viewbox`] and shrinks the larger axis to fit.
    ///
    /// Use `TRANSPARENT` (0) as color to render with original SVG colors,
    /// or pass a color to tint the entire icon.
    #[must_use]
    pub fn svg_contain(svg: &Svg, size: f32, color: Color) -> Self {
        let (vw, vh) = svg.viewbox();
        let scale = (size / vw).min(size / vh);
        let w = vw * scale;
        let h = vh * scale;
        Self::svg((size - w) / 2.0, (size - h) / 2.0, w, h, svg, color)
    }

    /// Enable anti-aliasing on this draw command (currently only affects icons).
    #[must_use]
    pub fn with_anti_alias(mut self) -> Self {
        if let Self::Svg {
            ref mut anti_alias, ..
        } = self
        {
            *anti_alias = true;
        }
        self
    }

    /// Override the fill colour of the SVG path whose `id` attribute
    /// matches `id`. Chainable: multiple `.fill(...)` calls layer
    /// overrides over the same draw.
    ///
    /// ```ignore
    /// Draw::svg(0.0, 0.0, 390.0, 390.0, &DIAL_ROUND, TRANSPARENT)
    ///     .fill("ticks-large", WHITE)
    ///     .fill("rim-outer", GRAY_60)
    /// ```
    ///
    /// Paths with no matching id fall through to the whole-icon
    /// `color` tint (when non-`TRANSPARENT`) or the SVG's own
    /// fill colour. No-op on non-SVG draw kinds.
    #[must_use]
    pub fn fill(mut self, id: impl Into<String>, color: Color) -> Self {
        if let Self::Svg { ref mut fills, .. } = self {
            fills.push((id.into(), color));
        }
        self
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

    /// QR code encoding `text`, rendered into a `size`×`size` square (quiet
    /// zone included). The host encodes and rasterises it — the wasm side only
    /// carries the text, so an over-long `text` that exceeds QR capacity is
    /// dropped host-side (nothing drawn) rather than failing here.
    #[must_use]
    pub fn qr(x: f32, y: f32, size: f32, text: &str, style: QrStyle) -> Self {
        Self::Qr {
            x,
            y,
            size,
            style,
            text: text.to_owned(),
        }
    }

    /// Bitmap from a pre-registered ID (for dynamically fetched images).
    ///
    /// Use `host::register_bitmap()` to register image data and get an ID,
    /// then pass it here to render. Useful for album art and other runtime images.
    #[must_use]
    pub fn bitmap_id(x: f32, y: f32, w: f32, h: f32, bitmap_id: Option<BitmapId>) -> Self {
        Self::Bitmap {
            x,
            y,
            w,
            h,
            bitmap_id,
        }
    }

    /// 9-patch bitmap at local position within canvas.
    ///
    /// On first call for a given asset, registers its bitmap with the host.
    /// Subsequent calls reuse the cached ID — zero per-frame overhead.
    #[must_use]
    pub fn nine_patch(
        x: impl PropsFieldValue<f32>,
        y: impl PropsFieldValue<f32>,
        w: impl PropsFieldValue<f32>,
        h: impl PropsFieldValue<f32>,
        asset: &NinePatchAsset,
    ) -> Self {
        let np = ensure_nine_patch_registered(asset);
        Self::NinePatch {
            x: x.into_field(),
            y: y.into_field(),
            w: w.into_field(),
            h: h.into_field(),
            nine_patch: np,
        }
    }

    /// 9-patch from a pre-registered [`NinePatch`] (for dynamically created 9-patches).
    #[must_use]
    pub fn nine_patch_id(
        x: impl PropsFieldValue<f32>,
        y: impl PropsFieldValue<f32>,
        w: impl PropsFieldValue<f32>,
        h: impl PropsFieldValue<f32>,
        np: NinePatch,
    ) -> Self {
        Self::NinePatch {
            x: x.into_field(),
            y: y.into_field(),
            w: w.into_field(),
            h: h.into_field(),
            nine_patch: np,
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
    /// Prefer the [`crate::sphere!`] macro for ergonomic call sites.
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

    /// 3D mesh at local position within canvas.
    ///
    /// Renders an arbitrary mesh with quaternion-based orientation, perspective
    /// camera, and optional directional lighting. The mesh is rendered to an
    /// offscreen FBO on the GPU and composited into the 2D scene by femtovg.
    ///
    /// # Examples
    /// ```ignore
    /// Draw::mesh(0.0, 0.0, 200.0, 200.0, &SUZANNE, MeshView {
    ///     orientation: Orientation::from_euler(30.0, 45.0, 0.0),
    ///     ..Default::default()
    /// })
    /// ```
    #[must_use]
    pub fn mesh(x: f32, y: f32, w: f32, h: f32, mdl: &Mesh, view: MeshView) -> Self {
        let mesh_id = ensure_mesh_registered(mdl);
        let light = view.light.unwrap_or(crate::mesh::LightAngles {
            pitch: f32::NAN,
            yaw: f32::NAN,
        });
        let highlight = view.highlight.unwrap_or(crate::mesh::Highlight {
            uv_rect: [f32::NAN, 0.0, 0.0, 0.0],
            color: [0.0, 0.0, 0.0],
        });
        Self::Mesh {
            x,
            y,
            w,
            h,
            mesh_id,
            fov: view.fov,
            distance: view.distance,
            qx: view.orientation.x,
            qy: view.orientation.y,
            qz: view.orientation.z,
            qw: view.orientation.w,
            px: view.position[0],
            py: view.position[1],
            pz: view.position[2],
            scale: view.scale,
            light_pitch: light.pitch,
            light_yaw: light.yaw,
            ambient: view.ambient,
            specular: view.specular,
            hl_u_min: highlight.uv_rect[0],
            hl_v_min: highlight.uv_rect[1],
            hl_u_max: highlight.uv_rect[2],
            hl_v_max: highlight.uv_rect[3],
            hl_r: highlight.color[0],
            hl_g: highlight.color[1],
            hl_b: highlight.color[2],
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

    /// Styled text laid out on a circular arc.
    ///
    /// Angles are radians, with `0.0` at 12 o'clock and increasing clockwise.
    /// `radius` is the circle followed by glyph centers.
    #[must_use]
    #[expect(clippy::too_many_arguments)]
    pub fn curved_text(
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        content: impl Into<String>,
        style: impl Into<TextStyle>,
    ) -> Self {
        Self::CurvedText {
            cx,
            cy,
            radius,
            angle,
            anchor,
            facing,
            text: content.into(),
            style: style.into(),
        }
    }

    /// Autofit text that only shrinks to fit (floor 12 px, no growth).
    #[must_use]
    pub fn autofit_text(
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        content: impl Into<String>,
        style: impl Into<TextStyle>,
    ) -> Self {
        Self::autofit_text_ranged(
            x,
            y,
            box_width,
            box_height,
            content,
            style,
            AutoFit::Shrink,
            0,
            0,
        )
    }

    /// Autofit text with explicit mode and size bounds (`0` = default floor / box-bounded).
    #[must_use]
    #[expect(clippy::too_many_arguments)]
    pub fn autofit_text_ranged(
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        content: impl Into<String>,
        style: impl Into<TextStyle>,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
    ) -> Self {
        Self::AutofitText {
            x,
            y,
            box_width,
            box_height,
            mode,
            min_size,
            max_size,
            text: content.into(),
            style: style.into(),
        }
    }

    /// Path draw command — polyline or polygon with optional Catmull-Rom smoothing.
    ///
    /// Prefer the [`crate::path!`] macro for ergonomic call sites.
    #[must_use]
    pub fn path(
        points: Vec<(f32, f32)>,
        stroke_width: f32,
        color: Color,
        closed: bool,
        interpolation: Interpolation,
    ) -> Self {
        Self::Path {
            points,
            paint: PathPaint::Stroke {
                color,
                width: stroke_width,
                dash: None,
            },
            closed,
            interpolation,
        }
    }

    /// Dashed polyline; the host splits it by arc length.
    /// Prefer `path!`'s `dashed:`.
    #[must_use]
    pub fn dashed_path(
        points: Vec<(f32, f32)>,
        stroke_width: f32,
        color: Color,
        dash_on: f32,
        dash_off: f32,
    ) -> Self {
        Self::Path {
            points,
            paint: PathPaint::Stroke {
                color,
                width: stroke_width,
                dash: Some(Dash {
                    on: dash_on,
                    off: dash_off,
                }),
            },
            closed: false,
            interpolation: Interpolation::Linear,
        }
    }

    /// Filled polygon with a [`Fill`] paint. Prefer the [`crate::fill!`] macro.
    #[must_use]
    pub fn fill_path(points: Vec<(f32, f32)>, paint: impl Into<Fill>, smooth: bool) -> Self {
        Self::Path {
            points,
            paint: PathPaint::Fill(paint.into()),
            closed: true,
            interpolation: if smooth {
                Interpolation::CatmullRom
            } else {
                Interpolation::Linear
            },
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
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "any non-Modified Draw is wrapped identically; new variants don't change the wrap"
    )]
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
        from_color: Color,
        to_color: Color,
        duration_ms: u32,
        easing: Easing,
        loop_mode: LoopMode,
    ) -> Self {
        self.animate(
            AnimProperty::Color,
            f32::from_bits(from_color.to_u32()),
            f32::from_bits(to_color.to_u32()),
            duration_ms,
            easing,
            loop_mode,
        )
    }

    /// Add a transition — host smoothly interpolates when static values change.
    ///
    /// `id` is a widget-supplied stable identifier
    /// for this draw (e.g. `"hour-hand"`).
    ///
    /// The host keys transition state on `(canvas_index, fnv1a_32(id))`,
    /// so interpolation tracks the logical draw regardless of sibling
    /// order in the tree.
    ///
    /// Use distinct ids per draw within the same canvas;
    /// reusing an id silently aliases two transitions
    /// to the same state slot.
    #[must_use]
    pub fn transition(self, id: &str, duration_ms: u32, easing: Easing) -> Self {
        self.transition_with_color_space(id, duration_ms, easing, ColorSpace::default())
    }

    /// Add a transition with explicit color interpolation space.
    /// See [`Draw::transition`] for the `id` semantics.
    #[must_use]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "any non-Modified Draw is wrapped identically; new variants don't change the wrap"
    )]
    pub fn transition_with_color_space(
        self,
        id: &str,
        duration_ms: u32,
        easing: Easing,
        color_space: ColorSpace,
    ) -> Self {
        let transition = Some(TransitionDef {
            id_hash: fnv1a_32(id),
            duration_ms,
            easing,
        });
        match self {
            Draw::Modified {
                animations,
                color_space: cs,
                inner,
                ..
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
#[derive(Clone, Debug)]
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
        id: String,
        label: String,
        style: ButtonStyle,
        size: ButtonSize,
        icon_id: Option<SvgId>,
        disabled: bool,
        /// Fill the container's cross axis instead of sizing to content.
        stretch: bool,
        skin: Option<ButtonSkin>,
    },
    Spacer {
        flex: f32,
    },
    Canvas {
        props: PropsData,
        touch_key: Option<String>,
        draws: Vec<Draw>,
    },
    /// Inline notification (error/warning/success/info banner)
    Notification {
        kind: NotificationKind,
        title: String,
        subtitle: String,
    },
    /// Host-rendered relative-time label — self-updating "N ago" / "in N".
    RelTime {
        anchor: i64,
        format: RelTimeFormat,
        clamp: RelTimeClamp,
        style: TextStyle,
    },
    /// Carbon status pill — host-rendered chrome around an embedder content child.
    Tag {
        kind: TagKind,
        icon: TagIcon,
        content: Box<Node>,
    },
    /// Segmented view switcher — host-rendered rounded pill of icon tabs.
    Switcher {
        active: usize,
        disabled: bool,
        tabs: Vec<SwitcherTab>,
    },
    /// Scrollable container — clips children and allows vertical scrolling.
    Scroll {
        scroll_key: String,
        props: PropsData,
        children: Vec<Node>,
    },
    /// Modal dialog overlay with title, close button, scrollable body, and optional footer
    Modal {
        modal_id: String,
        is_open: bool,
        title: String,
        content_height: f32,
        padding: u16,
        backdrop_alpha: u8,
        /// Modal body background color. `0` = default (`GRAY_90`).
        bg_color: Color,
        /// Modal header background color. `0` = default (`GRAY_100`).
        header_color: Color,
        /// Modal title text color. `0` = default (`GRAY_10`).
        title_color: Color,
        /// Maximum modal width. `0` = no limit.
        max_width: u16,
        body: Vec<Node>,
        /// Optional footer — primary key, primary label, secondary key, secondary label, danger flag.
        /// Empty strings = no secondary / no footer.
        footer_primary_key: String,
        footer_primary_label: String,
        footer_secondary_key: String,
        footer_secondary_label: String,
        footer_danger: bool,
    },
    /// Host-rendered progress bar — slider, meter, or indeterminate.
    ///
    /// Rendered entirely host-side: track, fill, animated squiggle, drag thumb.
    /// Uses `flex: 1.0` layout. Touch interaction via `touch_key`.
    ProgressBar {
        touch_key: String,
        track_h: f32,
        mode: ProgressMode,
        active: bool,
        fill_color: Color,
        track_color: Color,
        bg_color: Color,
        skin: Option<SliderSkin>,
    },
}

/// Result from tree rendering
#[derive(Debug, Default)]
pub struct TreeRenderResult {
    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub clicks: HashMap<String, TouchHit>,
    /// Active drag positions on interactive canvases (while finger is down)
    pub drags: HashMap<String, TouchHit>,
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

/// Create a button node (used by the `button!` macro).
#[must_use]
pub fn make_button(
    id: String,
    label: String,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<ButtonSkin>,
) -> Node {
    Node::Button {
        id,
        label,
        style,
        size,
        icon_id,
        disabled,
        stretch: false,
        skin,
    }
}

impl Node {
    /// Tell a button to fill its container's cross axis (no-op for other nodes).
    #[must_use]
    pub fn stretch(mut self) -> Self {
        if let Node::Button { stretch, .. } = &mut self {
            *stretch = true;
        }
        self
    }
}

/// Flexible spacer
#[must_use]
pub fn spacer(flex: impl crate::PropsFieldValue<f32>) -> Node {
    Node::Spacer {
        flex: flex.into_field(),
    }
}

/// Scrollable container — clips children and allows vertical scrolling.
///
/// - `key`: Unique string ID for state tracking and interaction targeting
/// - `props`: Layout props — **must set `height`** for the viewport
/// - `children`: Child nodes (laid out as a column)
pub fn scroll(key: &str, props: PropsData, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Scroll {
        scroll_key: String::from(key),
        props,
        children: children.into_iter().collect(),
    }
}

/// Canvas for custom drawing with draw commands as children
pub fn canvas(props: PropsData, draws: impl IntoIterator<Item = Draw>) -> Node {
    Node::Canvas {
        props,
        touch_key: None,
        draws: draws.into_iter().collect(),
    }
}

/// Interactive canvas with click detection and position reporting.
///
/// Works like `canvas()` but registers a touch region. When tapped, the click
/// position (local to the canvas bounds) is available via `TreeRenderResult::touch`.
pub fn touchable(key: &str, props: PropsData, draws: impl IntoIterator<Item = Draw>) -> Node {
    Node::Canvas {
        props,
        touch_key: Some(String::from(key)),
        draws: draws.into_iter().collect(),
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
/// Use the [`crate::fill!`] macro for filled polygons.
#[macro_export]
macro_rules! path {
    ($pts:expr, stroke: $w:expr, color: $c:expr) => {
        $crate::Draw::path($pts, $w, $c, false, $crate::Interpolation::Linear)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, smooth) => {
        $crate::Draw::path($pts, $w, $c, false, $crate::Interpolation::CatmullRom)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, closed) => {
        $crate::Draw::path($pts, $w, $c, true, $crate::Interpolation::Linear)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, closed, smooth) => {
        $crate::Draw::path($pts, $w, $c, true, $crate::Interpolation::CatmullRom)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, dashed: ($on:expr, $off:expr)) => {
        $crate::Draw::dashed_path($pts, $w, $c, $on, $off)
    };
}

/// Build a filled polygon ([`Draw::fill_path`]) with a solid colour or gradient.
///
/// ```ignore
/// fill!(pts, color: c)
/// fill!(pts, color: c, smooth)
/// fill!(pts, linear: (top, bottom))
/// fill!(pts, linear: (a, b), angle: 90.0)
/// fill!(pts, radial: (inner, outer))
/// ```
#[macro_export]
macro_rules! fill {
    ($pts:expr, color: $c:expr) => {
        $crate::Draw::fill_path($pts, $c, false)
    };
    ($pts:expr, color: $c:expr, smooth) => {
        $crate::Draw::fill_path($pts, $c, true)
    };
    ($pts:expr, linear: ($a:expr, $b:expr)) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear(0.0, $a, $b), false)
    };
    ($pts:expr, linear: ($a:expr, $b:expr), smooth) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear(0.0, $a, $b), true)
    };
    ($pts:expr, linear: ($a:expr, $b:expr), angle: $ang:expr) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear($ang, $a, $b), false)
    };
    ($pts:expr, linear: ($a:expr, $b:expr), angle: $ang:expr, smooth) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear($ang, $a, $b), true)
    };
    ($pts:expr, radial: ($inner:expr, $outer:expr)) => {
        $crate::Draw::fill_path($pts, $crate::Fill::radial($inner, $outer), false)
    };
    ($pts:expr, radial: ($inner:expr, $outer:expr), smooth) => {
        $crate::Draw::fill_path($pts, $crate::Fill::radial($inner, $outer), true)
    };
}

/// Serialize a node tree to the buffer
#[expect(
    clippy::too_many_lines,
    reason = "tree serialization keeps the wire format in one linear encoder"
)]
fn serialize_node(buf: &mut TreeBuffer, node: &Node) {
    match node {
        Node::Column(props, children) => {
            buf.write_column(
                props,
                u16::try_from(children.len()).expect("BUG: column children exceed u16::MAX"),
            );
            for child in children {
                serialize_node(buf, child);
            }
        }
        Node::Row(props, children) => {
            buf.write_row(
                props,
                u16::try_from(children.len()).expect("BUG: row children exceed u16::MAX"),
            );
            for child in children {
                serialize_node(buf, child);
            }
        }
        Node::Center(props, children) => {
            buf.write_center(
                props,
                u16::try_from(children.len()).expect("BUG: center children exceed u16::MAX"),
            );
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
        Node::RelTime {
            anchor,
            format,
            clamp,
            style,
        } => {
            buf.write_relative_time(*anchor, *format, *clamp, style);
        }
        Node::Tag {
            kind,
            icon,
            content,
        } => {
            buf.write_tag(*kind, *icon);
            serialize_node(buf, content);
        }
        Node::Button {
            id,
            label,
            style,
            size,
            icon_id,
            disabled,
            stretch,
            skin,
        } => {
            buf.write_button(id, label, *style, *size, *icon_id, *disabled, *stretch);
            // Trailing optional skin payload
            if let Some(s) = skin {
                buf.write_u8(1); // has_skin
                // Normal 9-patch
                buf.write_bitmap_id(s.normal.bitmap_id);
                buf.write_u16(s.normal.left);
                buf.write_u16(s.normal.top);
                buf.write_u16(s.normal.right);
                buf.write_u16(s.normal.bottom);
                // Pressed 9-patch (optional)
                if let Some(p) = &s.pressed {
                    buf.write_u8(1); // has_pressed
                    buf.write_bitmap_id(p.bitmap_id);
                    buf.write_u16(p.left);
                    buf.write_u16(p.top);
                    buf.write_u16(p.right);
                    buf.write_u16(p.bottom);
                } else {
                    buf.write_u8(0);
                }
                buf.write_color(s.text_color);
                buf.write_color(s.pressed_text_color);
                buf.write_u8(u8::from(s.opaque));
            } else {
                buf.write_u8(0); // no skin
            }
        }
        Node::Spacer { flex } => {
            buf.write_spacer(*flex);
        }
        Node::Canvas {
            props,
            touch_key,
            draws,
        } => {
            buf.write_canvas(
                props,
                touch_key.as_deref(),
                u16::try_from(draws.len()).expect("BUG: canvas draws exceed u16::MAX"),
            );
            for draw in draws {
                serialize_draw(buf, draw);
            }
        }
        Node::Scroll {
            scroll_key,
            props,
            children,
        } => {
            buf.write_scroll(
                scroll_key,
                props,
                u16::try_from(children.len()).expect("BUG: scroll children exceed u16::MAX"),
            );
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
        Node::Switcher {
            active,
            disabled,
            tabs,
        } => {
            buf.write_switcher(*active, *disabled, tabs);
        }
        Node::Modal {
            modal_id,
            is_open,
            title,
            content_height,
            padding,
            backdrop_alpha,
            bg_color,
            header_color,
            title_color,
            max_width,
            body,
            footer_primary_key,
            footer_primary_label,
            footer_secondary_key,
            footer_secondary_label,
            footer_danger,
        } => {
            buf.write_modal(
                modal_id,
                *is_open,
                *padding,
                *backdrop_alpha,
                title,
                *content_height,
                u16::try_from(body.len()).expect("BUG: modal body children exceed u16::MAX"),
            );
            buf.write_color(*bg_color);
            buf.write_color(*header_color);
            buf.write_color(*title_color);
            buf.write_u16(*max_width);
            for child in body {
                serialize_node(buf, child);
            }
            // Footer descriptor: [pk_len][pk][pl_len][pl][sk_len][sk][sl_len][sl][danger:u8]
            buf.write_u16(
                u16::try_from(footer_primary_key.len())
                    .expect("BUG: footer primary key exceeds u16::MAX bytes"),
            );
            buf.write_bytes(footer_primary_key.as_bytes());
            buf.write_u16(
                u16::try_from(footer_primary_label.len())
                    .expect("BUG: footer primary label exceeds u16::MAX bytes"),
            );
            buf.write_bytes(footer_primary_label.as_bytes());
            buf.write_u16(
                u16::try_from(footer_secondary_key.len())
                    .expect("BUG: footer secondary key exceeds u16::MAX bytes"),
            );
            buf.write_bytes(footer_secondary_key.as_bytes());
            buf.write_u16(
                u16::try_from(footer_secondary_label.len())
                    .expect("BUG: footer secondary label exceeds u16::MAX bytes"),
            );
            buf.write_bytes(footer_secondary_label.as_bytes());
            buf.write_u8(u8::from(*footer_danger));
        }
        Node::ProgressBar {
            touch_key,
            track_h,
            mode,
            active,
            fill_color,
            track_color,
            bg_color,
            skin,
        } => {
            buf.write_progress_bar(
                touch_key,
                *track_h,
                mode,
                *active,
                *fill_color,
                *track_color,
                *bg_color,
                skin,
            );
        }
    }
}

/// Serialize a draw command to the buffer
#[expect(
    clippy::too_many_lines,
    reason = "draw serialization keeps the wire format in one linear encoder"
)]
fn serialize_draw(buf: &mut TreeBuffer, draw: &Draw) {
    match draw {
        Draw::Rect { x, y, w, h, fill } => {
            buf.write_draw_rect(*x, *y, *w, *h, fill);
        }
        Draw::Circle { cx, cy, r, fill } => {
            buf.write_u8(DRAW_CIRCLE);
            buf.write_f32(*cx);
            buf.write_f32(*cy);
            buf.write_f32(*r);
            buf.write_fill(fill);
        }
        Draw::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            segments,
            cap,
        } => {
            buf.write_u8(DRAW_ARC);
            buf.write_f32(*cx);
            buf.write_f32(*cy);
            buf.write_f32(*radius);
            buf.write_f32(*start_angle);
            buf.write_f32(*end_angle);
            buf.write_f32(*width);
            buf.write_arc_fill(fill);
            buf.write_arc_segments(segments);
            buf.write_arc_cap(*cap);
        }
        Draw::Svg {
            x,
            y,
            w,
            h,
            color,
            icon_id,
            anti_alias,
            fills,
        } => {
            buf.write_u8(DRAW_ICON);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*w);
            buf.write_f32(*h);
            buf.write_color(*color);
            buf.write_icon_id(*icon_id);
            buf.write_u8(u8::from(*anti_alias));
            let fill_count = u16::try_from(fills.len())
                .expect("BUG: Draw::Svg has more than u16::MAX fill overrides");
            buf.write_u16(fill_count);
            for (id, color) in fills {
                let id_len =
                    u16::try_from(id.len()).expect("BUG: SVG path id exceeds u16::MAX bytes");
                buf.write_u16(id_len);
                buf.write_bytes(id.as_bytes());
                buf.write_color(*color);
            }
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
            buf.write_bitmap_id(*bitmap_id);
        }
        Draw::Qr {
            x,
            y,
            size,
            style,
            text,
        } => {
            buf.write_u8(DRAW_QR);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*size);
            buf.write_color(style.dark);
            buf.write_color(style.light);
            buf.write_u8(style.quiet_zone);
            let text_len = u16::try_from(text.len()).expect("BUG: QR text exceeds u16::MAX bytes");
            buf.write_u16(text_len);
            buf.write_bytes(text.as_bytes());
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
        Draw::Shadow {
            dx,
            dy,
            blur,
            color,
            inner,
        } => {
            buf.write_u8(DRAW_SHADOW);
            buf.write_f32(*dx);
            buf.write_f32(*dy);
            buf.write_f32(*blur);
            buf.write_color(*color);
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
                buf.write_u8(
                    u8::try_from(animations.len()).expect("BUG: node animations exceed u8::MAX"),
                );
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
                buf.write_u32(t.id_hash);
                buf.write_u32(t.duration_ms);
                buf.write_u8(t.easing as u8);
            }

            serialize_draw(buf, inner);
        }
        Draw::Path {
            points,
            paint,
            closed,
            interpolation,
        } => {
            let mut flags: u8 = 0;
            if *closed {
                flags |= 0x01;
            }
            if *interpolation == Interpolation::CatmullRom {
                flags |= 0x02;
            }
            if matches!(paint, PathPaint::Fill(_)) {
                flags |= 0x04;
            }
            if matches!(paint, PathPaint::Stroke { dash: Some(_), .. }) {
                flags |= 0x08;
            }
            buf.write_u8(DRAW_PATH);
            buf.write_u8(flags);
            buf.write_u16(u16::try_from(points.len()).expect("BUG: path points exceed u16::MAX"));
            for &(x, y) in points {
                buf.write_f32(x);
                buf.write_f32(y);
            }
            match paint {
                PathPaint::Fill(fill) => buf.write_fill(fill),
                PathPaint::Stroke { color, width, dash } => {
                    buf.write_color(*color);
                    buf.write_f32(*width);
                    if let Some(d) = dash {
                        buf.write_f32(d.on);
                        buf.write_f32(d.off);
                    }
                }
            }
        }
        Draw::Text { x, y, text, style } => {
            buf.write_u8(DRAW_TEXT);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_bytes(&style.to_bytes());
            let bytes = text.as_bytes();
            buf.write_u16(
                u16::try_from(bytes.len()).expect("BUG: draw text exceeds u16::MAX bytes"),
            );
            buf.write_bytes(bytes);
        }
        Draw::CurvedText {
            cx,
            cy,
            radius,
            angle,
            anchor,
            facing,
            text,
            style,
        } => {
            buf.write_u8(DRAW_CURVED_TEXT);
            buf.write_f32(*cx);
            buf.write_f32(*cy);
            buf.write_f32(*radius);
            buf.write_f32(*angle);
            buf.write_u8(u8::from(*anchor));
            buf.write_u8(u8::from(*facing));
            buf.write_bytes(&style.to_bytes());
            let bytes = text.as_bytes();
            let len = u16::try_from(bytes.len())
                .expect("BUG: Draw::CurvedText text exceeds u16::MAX bytes");
            buf.write_u16(len);
            buf.write_bytes(bytes);
        }
        Draw::AutofitText {
            x,
            y,
            box_width,
            box_height,
            mode,
            min_size,
            max_size,
            text,
            style,
        } => {
            buf.write_u8(DRAW_AUTOFIT_TEXT);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*box_width);
            buf.write_f32(*box_height);
            buf.write_u8(*mode as u8);
            buf.write_u16(*min_size);
            buf.write_u16(*max_size);
            buf.write_bytes(&style.to_bytes());
            let bytes = text.as_bytes();
            let len = u16::try_from(bytes.len())
                .expect("BUG: Draw::AutofitText text exceeds u16::MAX bytes");
            buf.write_u16(len);
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
            buf.write_bitmap_id(*bitmap_id);
            let flags: u8 = u8::from(*atmosphere);
            buf.write_u8(flags);
            buf.write_f32(*center_lat);
            buf.write_f32(*center_lon);
            buf.write_f32(*zoom);
            buf.write_f32(*light_lat);
            buf.write_f32(*light_lon);
        }
        Draw::Mesh {
            x,
            y,
            w,
            h,
            mesh_id,
            fov,
            distance,
            qx,
            qy,
            qz,
            qw,
            px,
            py,
            pz,
            scale,
            light_pitch,
            light_yaw,
            ambient,
            specular,
            hl_u_min,
            hl_v_min,
            hl_u_max,
            hl_v_max,
            hl_r,
            hl_g,
            hl_b,
        } => {
            buf.write_u8(DRAW_MESH);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*w);
            buf.write_f32(*h);
            buf.write_mesh_id(*mesh_id);
            buf.write_f32(*fov);
            buf.write_f32(*distance);
            buf.write_f32(*qx);
            buf.write_f32(*qy);
            buf.write_f32(*qz);
            buf.write_f32(*qw);
            buf.write_f32(*px);
            buf.write_f32(*py);
            buf.write_f32(*pz);
            buf.write_f32(*scale);
            buf.write_f32(*light_pitch);
            buf.write_f32(*light_yaw);
            buf.write_f32(*ambient);
            buf.write_f32(*specular);
            buf.write_f32(*hl_u_min);
            buf.write_f32(*hl_v_min);
            buf.write_f32(*hl_u_max);
            buf.write_f32(*hl_v_max);
            buf.write_f32(*hl_r);
            buf.write_f32(*hl_g);
            buf.write_f32(*hl_b);
        }
        Draw::NinePatch {
            x,
            y,
            w,
            h,
            nine_patch: np,
        } => {
            buf.write_u8(DRAW_NINE_PATCH);
            buf.write_f32(*x);
            buf.write_f32(*y);
            buf.write_f32(*w);
            buf.write_f32(*h);
            buf.write_bitmap_id(np.bitmap_id);
            buf.write_u16(np.left);
            buf.write_u16(np.top);
            buf.write_u16(np.right);
            buf.write_u16(np.bottom);
        }
    }
}

/// Collect interaction keys from the tree (buttons, touchable canvases, progress bars).
#[cfg(target_arch = "wasm32")]
fn collect_interaction_keys(node: &Node, keys: &mut Vec<String>) {
    match node {
        Node::Button { id, .. } => keys.push(id.clone()),
        Node::Canvas {
            touch_key: Some(key),
            ..
        } => keys.push(key.clone()),
        Node::ProgressBar { touch_key, .. } if !touch_key.is_empty() => {
            keys.push(touch_key.clone());
        }
        Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
            for child in children {
                collect_interaction_keys(child, keys);
            }
        }
        Node::Scroll {
            scroll_key,
            children,
            ..
        } => {
            keys.push(scroll_key.clone());
            for child in children {
                collect_interaction_keys(child, keys);
            }
        }
        Node::Modal {
            is_open,
            modal_id,
            body,
            footer_primary_key,
            footer_secondary_key,
            ..
        } => {
            if *is_open {
                for child in body {
                    collect_interaction_keys(child, keys);
                }
                if !footer_primary_key.is_empty() {
                    keys.push(footer_primary_key.clone());
                }
                if !footer_secondary_key.is_empty() {
                    keys.push(footer_secondary_key.clone());
                }
                keys.push(std::format!("{modal_id}::close"));
            }
        }
        Node::Switcher {
            tabs,
            disabled: false,
            ..
        } => {
            for tab in tabs {
                keys.push(tab.click_id.clone());
            }
        }
        _ => {}
    }
}

/// Render UI tree using host-side layout.
/// Returns click and drag interactions keyed by string IDs.
#[must_use]
#[expect(clippy::needless_pass_by_value)] // Node is consumed by serialization
#[cfg(target_arch = "wasm32")]
pub fn render_ui(width: u32, height: u32, root: Node) -> TreeRenderResult {
    let mut keys = Vec::new();
    collect_interaction_keys(&root, &mut keys);

    // Serialize tree to buffer and submit to host for layout and rendering
    begin_tree();
    with_buffer(|buf| serialize_node(buf, &root));
    submit_and_clear(width, height);

    // Collect all interactions (clicks and active drags)
    let mut result = TreeRenderResult::default();
    for key in &keys {
        if let Some(hit) = host::get_touch_click(key) {
            result.clicks.insert(key.clone(), hit);
        }
        if let Some(hit) = host::get_touch_drag(key) {
            result.drags.insert(key.clone(), hit);
        }
    }

    result
}

#[cfg(test)]
mod drop_shadow_tests {
    use super::*;

    fn rect() -> Draw {
        Draw::rect(0.0, 0.0, 10.0, 10.0, Color::from_rgb(0xFF, 0xFF, 0xFF))
    }

    #[test]
    fn with_drop_shadow_wraps_the_draw() {
        let shadowed = rect().with_drop_shadow(DropShadow {
            dx: 1.0,
            dy: 2.0,
            blur: 4.0,
            color: Color::from_rgba(0, 0, 0, 0x80),
        });
        let Draw::Shadow {
            dx,
            dy,
            blur,
            inner,
            ..
        } = shadowed
        else {
            panic!("with_drop_shadow should produce Draw::Shadow");
        };
        assert_eq!((dx, dy, blur), (1.0, 2.0, 4.0));
        assert!(matches!(*inner, Draw::Rect { .. }));
    }

    #[test]
    fn with_drop_shadow_does_not_stack() {
        let restacked = rect()
            .with_drop_shadow(DropShadow {
                dx: 1.0,
                dy: 1.0,
                blur: 4.0,
                color: Color::from_rgba(0, 0, 0, 0x80),
            })
            .with_drop_shadow(DropShadow {
                dx: 9.0,
                dy: 9.0,
                blur: 8.0,
                color: Color::from_rgba(0, 0, 0, 0xC0),
            });
        let Draw::Shadow {
            dx, blur, inner, ..
        } = restacked
        else {
            panic!("expected Draw::Shadow");
        };
        // Second call replaces the first — params are the latest.
        assert_eq!((dx, blur), (9.0, 8.0));
        // The inner stays the original Rect, not a nested Shadow.
        assert!(matches!(*inner, Draw::Rect { .. }), "shadows must not nest");
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "clamp output is an exact bound")]
    fn with_drop_shadow_clamps_blur_to_range() {
        // Negative blur clamps to 0 without tripping the debug assertion
        // (the assertion only fires for values *above* the cap).
        let shadowed = rect().with_drop_shadow(DropShadow {
            dx: 0.0,
            dy: 0.0,
            blur: -5.0,
            color: Color::from_rgb(0, 0, 0),
        });
        let Draw::Shadow { blur, .. } = shadowed else {
            panic!("expected Draw::Shadow");
        };
        assert_eq!(blur, 0.0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "drop-shadow blur")]
    fn with_drop_shadow_asserts_on_over_cap_blur() {
        let _ = rect().with_drop_shadow(DropShadow {
            dx: 0.0,
            dy: 0.0,
            blur: DROP_SHADOW_BLUR_MAX + 50.0,
            color: Color::from_rgb(0, 0, 0),
        });
    }

    #[test]
    fn serialize_draw_emits_shadow_then_inner() {
        let shadowed = rect().with_drop_shadow(DropShadow {
            dx: 1.0,
            dy: 2.0,
            blur: 4.0,
            color: Color::from_rgba(0, 0, 0, 0x80),
        });
        let mut buf = TreeBuffer::new();
        serialize_draw(&mut buf, &shadowed);
        let bytes = buf.into_bytes();
        // [DRAW_SHADOW][dx:4][dy:4][blur:4][color:4][inner...]
        assert_eq!(bytes[0], DRAW_SHADOW);
        assert_eq!(
            bytes[17], DRAW_RECT,
            "the wrapped draw must follow the 16-byte shadow header",
        );
    }
}

#[cfg(test)]
mod curved_text_tests {
    use super::*;
    use bmc_wasm_protocol::FontWeight;

    #[test]
    fn curved_text_serializes_geometry_enums_style_and_text() {
        let style = TextStyle {
            size: 22,
            color: Color::from_rgb(1, 2, 3),
            weight: FontWeight::BOLD,
            ..TextStyle::default()
        };
        let draw = Draw::curved_text(
            10.0,
            20.0,
            30.0,
            1.25,
            ArcAnchor::End,
            ArcTextFacing::Inward,
            "MINING",
            style,
        );

        let mut buf = TreeBuffer::new();
        serialize_draw(&mut buf, &draw);
        let bytes = buf.into_bytes();

        assert_eq!(bytes[0], DRAW_CURVED_TEXT);
        assert_eq!(&bytes[1..5], &10.0_f32.to_le_bytes());
        assert_eq!(&bytes[5..9], &20.0_f32.to_le_bytes());
        assert_eq!(&bytes[9..13], &30.0_f32.to_le_bytes());
        assert_eq!(&bytes[13..17], &1.25_f32.to_le_bytes());
        assert_eq!(bytes[17], u8::from(ArcAnchor::End));
        assert_eq!(bytes[18], u8::from(ArcTextFacing::Inward));
        assert_eq!(&bytes[19..47], &style.to_bytes());
        assert_eq!(
            u16::from_le_bytes(bytes[47..49].try_into().expect("BUG: len bytes")),
            6
        );
        assert_eq!(&bytes[49..], b"MINING");
    }

    #[test]
    #[should_panic(expected = "BUG: Draw::CurvedText text exceeds u16::MAX bytes")]
    fn curved_text_panics_when_text_exceeds_wire_length() {
        let text = "x".repeat(usize::from(u16::MAX) + 1);
        let draw = Draw::curved_text(
            0.0,
            0.0,
            10.0,
            0.0,
            ArcAnchor::Center,
            ArcTextFacing::Outward,
            text,
            TextStyle::default(),
        );
        let mut buf = TreeBuffer::new();
        serialize_draw(&mut buf, &draw);
    }
}

#[cfg(test)]
mod fill_wire_tests {
    use super::*;
    use bmc_wasm_protocol::{ArcCap, ArcFill, ArcSegments, FILL_LINEAR, Fill};

    #[test]
    fn arc_constructor_builds_variant() {
        let fill = ArcFill::gradient(Color::from_rgb(1, 2, 3), Color::from_rgb(4, 5, 6));
        let segments = ArcSegments::Explicit(vec![(0.0, 0.25), (0.5, 1.0)]);
        let draw = Draw::arc(
            1.0,
            2.0,
            3.0,
            0.25,
            1.5,
            4.0,
            fill,
            segments.clone(),
            ArcCap::Butt,
        );
        let Draw::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill: got_fill,
            segments: got_segments,
            cap,
        } = draw
        else {
            panic!("BUG: Draw::arc must build Draw::Arc");
        };
        assert_eq!(
            (cx, cy, radius, start_angle, end_angle, width),
            (1.0, 2.0, 3.0, 0.25, 1.5, 4.0)
        );
        assert_eq!(got_fill, fill);
        assert_eq!(got_segments, segments);
        assert_eq!(cap, ArcCap::Butt);
    }

    #[test]
    fn arc_serializes_with_opcode_first() {
        let mut buf = TreeBuffer::new();
        let draw = Draw::arc(
            1.0,
            2.0,
            3.0,
            0.25,
            1.5,
            4.0,
            Color::from_rgb(1, 2, 3),
            ArcSegments::Continuous,
            ArcCap::Round,
        );
        serialize_draw(&mut buf, &draw);
        assert_eq!(buf.data[0], DRAW_ARC);
    }

    #[test]
    fn qr_constructor_keeps_text_and_style() {
        let draw = Draw::qr(1.0, 2.0, 40.0, "hi", QrStyle::default());
        let Draw::Qr {
            x,
            y,
            size,
            style,
            text,
        } = draw
        else {
            panic!("BUG: Draw::qr must build Draw::Qr");
        };
        assert_eq!((x, y, size), (1.0, 2.0, 40.0));
        assert_eq!(style.quiet_zone, 2);
        assert_eq!(text, "hi");
    }

    #[test]
    fn qr_serializes_opcode_then_geometry_then_text() {
        let draw = Draw::qr(0.0, 0.0, 40.0, "hi", QrStyle::default());
        let mut buf = TreeBuffer::new();
        serialize_draw(&mut buf, &draw);
        assert_eq!(buf.data[0], DRAW_QR);
        assert!(buf.data.ends_with(b"hi"), "text rides at the tail");
    }

    #[test]
    fn rect_serializes_linear_fill_after_geometry() {
        let red = Color::from_rgb(0xFF, 0, 0);
        let blue = Color::from_rgb(0, 0, 0xFF);
        let draw = Draw::rect(1.0, 2.0, 3.0, 4.0, Fill::linear(90.0, red, blue));
        let mut buf = TreeBuffer::new();
        serialize_draw(&mut buf, &draw);
        let bytes = buf.into_bytes();
        // [DRAW_RECT][x:4][y:4][w:4][h:4][fill...]
        assert_eq!(bytes[0], DRAW_RECT);
        assert_eq!(
            bytes[17], FILL_LINEAR,
            "fill block must follow 16 bytes of geometry"
        );
    }

    #[test]
    fn rect_solid_call_site_still_compiles() {
        let _ = Draw::rect(0.0, 0.0, 1.0, 1.0, Color::from_rgb(1, 2, 3));
    }

    #[test]
    fn fill_macro_builds_a_filled_path() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)];
        let red = Color::from_rgb(0xFF, 0, 0);
        let draw = fill!(pts.clone(), linear: (red, red), angle: 90.0);
        let Draw::Path { paint, .. } = draw else {
            panic!("fill! must build a Draw::Path");
        };
        assert_eq!(paint, PathPaint::Fill(Fill::linear(90.0, red, red)));
    }

    #[test]
    fn path_macro_stroke_stays_solid() {
        let pts = vec![(0.0, 0.0), (10.0, 10.0)];
        let red = Color::from_rgb(0xFF, 0, 0);
        let draw = path!(pts.clone(), stroke: 2.0, color: red);
        let Draw::Path { paint, .. } = draw else {
            panic!("path! must build a Draw::Path");
        };
        assert_eq!(
            paint,
            PathPaint::Stroke {
                color: red,
                width: 2.0,
                dash: None,
            }
        );
    }

    #[test]
    fn path_macro_dashed_carries_the_pattern() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0)];
        let red = Color::from_rgb(0xFF, 0, 0);
        let draw = path!(pts, stroke: 1.0, color: red, dashed: (6.0, 4.0));
        let Draw::Path { paint, .. } = draw else {
            panic!("path! must build a Draw::Path");
        };
        assert_eq!(
            paint,
            PathPaint::Stroke {
                color: red,
                width: 1.0,
                dash: Some(Dash { on: 6.0, off: 4.0 }),
            }
        );
    }
}

#[cfg(test)]
mod autofit_text_tests {
    use super::*;
    use bmc_wasm_protocol::AutoFit;

    #[test]
    fn autofit_text_serializes_geometry_mode_bounds_style_and_text() {
        let style = TextStyle {
            size: 40,
            ..TextStyle::default()
        };
        let draw = Draw::autofit_text_ranged(
            10.0,
            20.0,
            100.0,
            50.0,
            "HELLO",
            style,
            AutoFit::ShrinkAndGrow,
            14,
            64,
        );
        let mut buf = TreeBuffer::new();
        serialize_draw(&mut buf, &draw);
        let bytes = buf.into_bytes();

        assert_eq!(bytes[0], DRAW_AUTOFIT_TEXT);
        assert_eq!(&bytes[1..5], &10.0_f32.to_le_bytes());
        assert_eq!(&bytes[5..9], &20.0_f32.to_le_bytes());
        assert_eq!(&bytes[9..13], &100.0_f32.to_le_bytes());
        assert_eq!(&bytes[13..17], &50.0_f32.to_le_bytes());
        assert_eq!(bytes[17], AutoFit::ShrinkAndGrow as u8);
        assert_eq!(&bytes[18..20], &14_u16.to_le_bytes());
        assert_eq!(&bytes[20..22], &64_u16.to_le_bytes());
        let style_end = 22 + TextStyle::SIZE;
        assert_eq!(&bytes[22..style_end], &style.to_bytes());
        assert_eq!(
            u16::from_le_bytes(
                bytes[style_end..style_end + 2]
                    .try_into()
                    .expect("BUG: len")
            ),
            5
        );
        assert_eq!(&bytes[style_end + 2..], b"HELLO");
    }

    #[test]
    fn autofit_text_defaults_to_shrink() {
        let draw = Draw::autofit_text(0.0, 0.0, 10.0, 10.0, "x", TextStyle::default());
        let Draw::AutofitText {
            mode,
            min_size,
            max_size,
            ..
        } = draw
        else {
            panic!("BUG: expected AutofitText");
        };
        assert_eq!(mode, AutoFit::Shrink);
        assert_eq!((min_size, max_size), (0, 0));
    }
}

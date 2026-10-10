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

//! The widget tree: nodes, canvas draws and the builders that make them.
//!
//! One vocabulary for both sides of the wire: a widget builds a [`Node`]
//! and encodes it, the host decodes the very same type and lays it out.
//! The bytes between them are [`crate::codec`]'s business.

use std::collections::HashMap;

use crate::animation::{AnimProperty, ColorSpace, Easing, LoopMode};
use crate::arc::{ArcCap, ArcFill, ArcSegments};
use crate::colors::{BLACK, Color, WHITE};
use crate::fill::{Dash, Fill, PathPaint};
use crate::ids::{BitmapId, MeshId, SvgId};
use crate::mesh::{MeshDrawArgs, MeshView};
use crate::nodes::{ButtonSize, ButtonStyle, DROP_SHADOW_BLUR_MAX, TagKind};
use crate::progress::ProgressKind;
use crate::relative_time::{RelTimeClamp, RelTimeFormat};
use crate::skeleton::SkeletonKind;
use crate::skin::{ButtonSkin, NinePatch, SliderSkin};
use crate::text::{ArcAnchor, ArcTextFacing, AutoFit, FontWeight, PropsData, SpanFlags, TextStyle};

// ── Draw-time descriptors ───────────────────────────────────────────

/// Definition of a single animation the host runs on a draw.
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

/// Definition of a transition the host interpolates when a draw's values change.
///
/// The host keys transition state on `(canvas_index, id_hash)`
/// so the interpolation follows the *logical* draw across tree-shape
/// changes (e.g. an optional sibling appearing or disappearing).
///
/// Without an id the host would key on draw position, which silently
/// swaps state between draws when their relative order shifts.
#[derive(Clone, Debug)]
pub struct TransitionDef {
    /// FNV1a-32 digest of the widget's [`Draw::transition`] id argument.
    pub id_hash: u32,
    pub duration_ms: u32,
    pub easing: Easing,
}

/// FNV1a-32 hash of `s`, deriving a stable per-draw key
/// from the widget-supplied id string at call time.
///
/// Const so `Draw::transition("hour-hand", …)` const-folds
/// the hash at compile time for `&'static str` ids.
#[must_use]
pub const fn fnv1a_32(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut hash: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
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

/// A text span with optional style overrides.
///
/// `Hash` is derived rather than written out, so a field
/// added here reaches the paragraph layout cache's key on its own.
#[derive(Clone, Debug, Hash)]
pub struct Span {
    pub text: String,
    pub weight: Option<FontWeight>,
    pub color: Option<Color>,
    /// Font size for this span alone; it keeps the paragraph's baseline.
    pub size: Option<u32>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Span {
    /// A span of `text` in the paragraph's own style.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            weight: None,
            color: None,
            size: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    /// This span's two wire flag words, packed by [`SpanFlags`]
    /// — the one place the bit positions are defined, host side included.
    #[must_use]
    pub fn wire_flags(&self) -> (u16, u8) {
        SpanFlags {
            weight: self.weight,
            has_color: self.color.is_some(),
            has_size: self.size.is_some(),
            italic: self.italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
        }
        .pack()
    }

    /// This span's effective style over the paragraph's `base` style.
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

/// Parameters for [`Draw::with_drop_shadow`].
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

// ── Node payloads ───────────────────────────────────────────────────

/// Leading icon for a [`Node::Tag`]. `Default` uses the per-kind theme icon,
/// `Hidden` draws none, `Custom` overrides with an explicit glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagIcon {
    Default,
    Hidden,
    Custom(SvgId),
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

impl From<u8> for NotificationKind {
    /// An unknown byte reads as `Info`, the least alarming banner.
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Error,
            1 => Self::Warning,
            2 => Self::Success,
            _ => Self::Info,
        }
    }
}

/// Progress bar mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressMode {
    /// A draggable slider at a fraction (0.0–1.0): fill plus drag thumb,
    /// the HTML `<input type=range>` of the family.
    Slider(f32),
    /// Unknown duration — animated indicator across full width.
    Indeterminate,
    /// A passive meter at a fraction (0.0–1.0): plain bar,
    /// no drag thumb or squiggle — the HTML `<progress>` of the family.
    Meter(f32),
}

impl ProgressMode {
    #[must_use]
    pub fn kind(self) -> ProgressKind {
        match self {
            Self::Slider(_) => ProgressKind::Slider,
            Self::Indeterminate => ProgressKind::Indeterminate,
            Self::Meter(_) => ProgressKind::Meter,
        }
    }

    /// The fraction the wire carries; `Indeterminate` has none and writes `0.0`.
    #[must_use]
    pub fn fraction(self) -> f32 {
        match self {
            Self::Slider(fraction) | Self::Meter(fraction) => fraction,
            Self::Indeterminate => 0.0,
        }
    }

    #[must_use]
    pub fn from_wire(kind: ProgressKind, fraction: f32) -> Self {
        match kind {
            ProgressKind::Slider => Self::Slider(fraction),
            ProgressKind::Indeterminate => Self::Indeterminate,
            ProgressKind::Meter => Self::Meter(fraction),
        }
    }
}

/// One tab of a [`Node::Switcher`]: its icon and the click id it emits when tapped.
#[derive(Clone, Debug)]
pub struct SwitcherTab {
    pub icon: Option<SvgId>,
    pub click_id: String,
}

/// A loading placeholder's role and the metrics of the value it stands in for.
#[derive(Clone, Copy, Default, Debug)]
pub struct SkeletonData {
    pub kind: SkeletonKind,
    /// Glyph count the slot would have held (`Text` / `Heading`).
    pub chars: f32,
    /// Font size of the text the slot stands in for (`Text` / `Heading`).
    pub font_size: f32,
    /// Explicit box for `Placeholder`, and the bar height for `Fill`.
    pub width: f32,
    pub height: f32,
    pub color: Color,
}

// ── Interaction readback ────────────────────────────────────────────

/// Touch interaction result (click or drag position + element dimensions).
#[derive(Debug, Clone, Copy)]
pub struct TouchHit {
    /// Local x position (relative to element left edge)
    pub x: f32,
    /// Local y position (relative to element top edge)
    pub y: f32,
    /// Element layout width
    pub width: f32,
    /// Element layout height
    pub height: f32,
}

impl TouchHit {
    /// Horizontal fraction (0.0 = left edge, 1.0 = right edge), clamped.
    #[must_use]
    pub fn frac_x(&self) -> f32 {
        if self.width > 0.0 {
            (self.x / self.width).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// The interactions a rendered tree reports back, keyed by element id.
#[derive(Debug, Default)]
pub struct TreeRenderResult {
    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub clicks: HashMap<String, TouchHit>,
    /// Active drag positions on interactive canvases (while finger is down)
    pub drags: HashMap<String, TouchHit>,
}

// ── Props field coercion ────────────────────────────────────────────

/// Trait that allows integer literals to be used for `f32` fields in `props!`.
///
/// Identity impl covers all types (f32→f32, bool→bool, Color→Color, etc.).
/// Additional impls convert integer types to f32 so you can write
/// `props!(gap: 12, padding: 16)` instead of `props!(gap: 12.0, padding: 16.0)`.
#[doc(hidden)]
pub trait PropsFieldValue<T> {
    fn into_field(self) -> T;
}

impl<T> PropsFieldValue<T> for T {
    fn into_field(self) -> T {
        self
    }
}

macro_rules! impl_int_to_f32_lossless {
    ($($t:ty),*) => {
        $(impl PropsFieldValue<f32> for $t {
            fn into_field(self) -> f32 { f32::from(self) }
        })*
    };
}

macro_rules! impl_int_to_f32_lossy {
    ($($t:ty),*) => {
        $(impl PropsFieldValue<f32> for $t {
            #[expect(clippy::cast_precision_loss)]
            fn into_field(self) -> f32 { self as f32 }
        })*
    };
}

impl_int_to_f32_lossless!(i8, i16, u8, u16);
impl_int_to_f32_lossy!(i32, i64, u32, u64, isize, usize);

// ── Draws ───────────────────────────────────────────────────────────

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
        args: MeshDrawArgs,
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
    ///
    /// Placement is read off the canvas rather than off an enclosing `centered`
    /// or `orbit`; a translate applied outside still moves the result.
    #[must_use]
    pub fn centered(inner: Draw) -> Self {
        Self::Centered {
            inner: Box::new(inner),
        }
    }

    /// Position any draw command at orbit around canvas center.
    ///
    /// Places from the canvas centre like [`Draw::centered`], with the same
    /// translate behaviour.
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

    /// Svg from a registered id at local position within canvas.
    ///
    /// Use `TRANSPARENT` (0) as color to render with original SVG colors,
    /// or pass a color to tint the entire icon.
    #[must_use]
    pub fn svg_id(x: f32, y: f32, w: f32, h: f32, icon_id: Option<SvgId>, color: Color) -> Self {
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
    /// Use `ICON_CLOSE` or other `ICON_*` constants.
    #[must_use]
    pub fn svg_builtin(
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        icon_id: impl Into<Option<SvgId>>,
        color: Color,
    ) -> Self {
        Self::svg_id(x, y, w, h, icon_id.into(), color)
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

    /// 3D sphere at local position within canvas, textured by a pre-registered bitmap.
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
    /// values zoom out. Typical full-globe values are ~1.6–2.2.
    ///
    /// Transitions applied via `.transition(...)` will smoothly interpolate
    /// `center_lat`, `center_lon`, `zoom`, and light direction on the host.
    ///
    /// When `atmosphere` is true, adds limb darkening and bluish edge glow.
    #[must_use]
    #[expect(clippy::too_many_arguments)]
    pub fn sphere_id(
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: Option<BitmapId>,
        center_lat: f32,
        center_lon: f32,
        zoom: f32,
        light: Option<(f32, f32)>,
        atmosphere: bool,
    ) -> Self {
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

    /// 3D mesh at local position within canvas, from a pre-registered mesh id.
    ///
    /// Renders an arbitrary mesh with quaternion-based orientation, perspective
    /// camera, and optional directional lighting. The mesh is rendered to an
    /// offscreen FBO on the GPU and composited into the 2D scene by femtovg.
    #[must_use]
    pub fn mesh_id(
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mesh_id: Option<MeshId>,
        view: MeshView,
    ) -> Self {
        Self::Mesh {
            x,
            y,
            w,
            h,
            mesh_id,
            args: MeshDrawArgs::from(view),
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

    /// Filled polygon with a [`Fill`] paint.
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
    ///
    /// A `duration_ms` of 0 snaps: the host settles
    /// the stored positions at the new values,
    /// so a later non-zero duration animates from there
    /// rather than from the pre-snap position.
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

// ── Nodes ───────────────────────────────────────────────────────────

/// A UI node in the tree
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
    /// Canvas with optional touch interaction key.
    /// When `touch_key` is `Some`, the canvas registers a hit region
    /// and reports the click position via the render result.
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
    /// Host-rendered loading placeholder holding a value's slot while it loads.
    Skeleton(SkeletonData),
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
        touch_key: Option<String>,
        track_h: f32,
        mode: ProgressMode,
        active: bool,
        fill_color: Color,
        track_color: Color,
        bg_color: Color,
        skin: Option<SliderSkin>,
    },
    /// `child` drawn with every colour scaled by `brightness`, and nested ones multiply.
    /// No layout box of its own: `child` lays out as if unwrapped,
    /// so absolute insets still resolve against the same parent.
    /// A modal inside still draws lit: modals paint after the tree, outside it.
    Dimmed {
        brightness: f32,
        child: Box<Node>,
    },
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

/// Create a button node.
#[must_use]
pub fn make_button(
    id: impl Into<String>,
    label: impl Into<String>,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<ButtonSkin>,
) -> Node {
    Node::Button {
        id: id.into(),
        label: label.into(),
        style,
        size,
        icon_id,
        disabled,
        stretch: false,
        skin,
    }
}

/// Flexible spacer
#[must_use]
pub fn spacer(flex: impl PropsFieldValue<f32>) -> Node {
    Node::Spacer {
        flex: flex.into_field(),
    }
}

/// A fixed vertical gap: an empty column of the given height. The layout
/// engine has only uniform `padding`/`margin`, so one-sided vertical spacing
/// is added as a sized spacer between siblings.
#[must_use]
pub fn fixed_height(height: f32) -> Node {
    col(
        PropsData {
            height,
            ..Default::default()
        },
        Vec::new(),
    )
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
/// Works like `canvas()` but registers a touch region.
/// When tapped, the click position (local to the canvas bounds) comes back under `key`.
pub fn touchable(key: &str, props: PropsData, draws: impl IntoIterator<Item = Draw>) -> Node {
    Node::Canvas {
        props,
        touch_key: Some(String::from(key)),
        draws: draws.into_iter().collect(),
    }
}

/// `child`, drawn at `brightness`; see [`Node::Dimmed`].
#[must_use]
pub fn dimmed(brightness: f32, child: Node) -> Node {
    Node::Dimmed {
        brightness,
        child: Box::new(child),
    }
}

/// Every key the host may report an interaction under:
/// buttons, touchable canvases, progress bars, scroll containers,
/// an open modal's footer and close button, an enabled switcher's tabs.
#[must_use]
pub fn interaction_keys(root: &Node) -> Vec<String> {
    let mut keys = Vec::new();
    collect_interaction_keys(root, &mut keys);
    keys
}

fn collect_interaction_keys(node: &Node, keys: &mut Vec<String>) {
    match node {
        Node::Button { id, .. } => keys.push(id.clone()),
        Node::Canvas {
            touch_key: Some(key),
            ..
        }
        | Node::ProgressBar {
            touch_key: Some(key),
            ..
        } if !key.is_empty() => keys.push(key.clone()),
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
                let mut close = modal_id.clone();
                close.push_str("::close");
                keys.push(close);
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
        Node::Dimmed { child, .. } => collect_interaction_keys(child, keys),
        Node::Paragraph { .. }
        | Node::Spacer { .. }
        | Node::Canvas { .. }
        | Node::Notification { .. }
        | Node::RelTime { .. }
        | Node::Tag { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_)
        | Node::ProgressBar { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::{ArcCap, ArcFill, ArcSegments};
    use crate::nodes::DROP_SHADOW_BLUR_MAX;

    fn rect() -> Draw {
        Draw::rect(0.0, 0.0, 10.0, 10.0, Color::from_rgb(0xFF, 0xFF, 0xFF))
    }

    #[test]
    fn the_guest_polls_exactly_the_keys_the_host_decodes() {
        let bar = |touch_key| {
            crate::progress_bar(
                touch_key,
                2.0,
                ProgressMode::Slider(0.5),
                false,
                Color::from_rgb(1, 1, 1),
                Color::from_rgb(2, 2, 2),
                Color::from_rgb(3, 3, 3),
                None,
            )
        };
        let nodes = [
            bar(Some("")),
            bar(Some("volume")),
            bar(None),
            touchable("", PropsData::default(), [rect()]),
            touchable("seek", PropsData::default(), [rect()]),
            canvas(PropsData::default(), [rect()]),
        ];

        for node in nodes {
            let decoded =
                crate::decode(&crate::encode(&node)).expect("BUG: an encoded tree decodes");
            assert_eq!(
                interaction_keys(&node),
                interaction_keys(&decoded.root),
                "{node:?}"
            );
        }
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
        assert_eq!((dx, blur), (9.0, 8.0), "the latest call's params win");
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
    fn rect_solid_call_site_still_compiles() {
        let _ = Draw::rect(0.0, 0.0, 1.0, 1.0, Color::from_rgb(1, 2, 3));
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

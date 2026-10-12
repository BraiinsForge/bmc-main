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

// Re-export the shared wire enums so widgets match on the SDK's own path.
pub use bmc_ui::{DisplayShape, ViewportShape};

/// The drawable rectangle assigned to this widget, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetViewport {
    pub width: u32,
    pub height: u32,
    pub shape: ViewportShape,
}

/// The logical display this widget's viewport lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
}

/// Known widget size variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeVariant {
    Full,   // 1280×480
    Large,  //  638×480
    Medium, //  638×238
    Small,  //  317×238
}

impl SizeVariant {
    /// Canonical pixel width for this variant.
    #[must_use]
    pub const fn width(self) -> u32 {
        match self {
            Self::Full => 1_280,
            Self::Large | Self::Medium => 638,
            Self::Small => 317,
        }
    }

    /// Canonical pixel height for this variant.
    #[must_use]
    pub const fn height(self) -> u32 {
        match self {
            Self::Full | Self::Large => 480,
            Self::Medium | Self::Small => 238,
        }
    }

    /// All variants. Order is irrelevant to the result — the comparator below
    /// gives a total order — but keeping it explicit documents the candidate set.
    const ALL: [SizeVariant; 4] = [
        SizeVariant::Small,
        SizeVariant::Medium,
        SizeVariant::Large,
        SizeVariant::Full,
    ];

    /// Pixel area of this variant's canonical dimensions.
    #[must_use]
    const fn area(self) -> u32 {
        self.width() * self.height()
    }

    /// Normalized distance from `(w, h)` to this variant's canonical size:
    /// `|w-vw|/vw + |h-vh|/vh`. Zero on an exact dimensional match.
    #[must_use]
    fn distance_from(self, w: u32, h: u32) -> f64 {
        let vw = f64::from(self.width());
        let vh = f64::from(self.height());
        (f64::from(w) - vw).abs() / vw + (f64::from(h) - vh).abs() / vh
    }

    /// Classify a viewport to the closest BMC100 variant.
    ///
    /// Minimize the normalized distance; on a genuine distance tie, prefer the
    /// larger-area variant so a non-BMC100 fullscreen display never collapses
    /// to a compact layout. An exact dimensional match has distance 0 and so
    /// always wins. Deterministic for identical inputs.
    #[must_use]
    pub fn closest(w: u32, h: u32) -> Self {
        // Destructuring the fixed-size array seeds the fold from a concrete
        // variant, so the result is `Self` rather than `Option<Self>` and the
        // never-empty invariant is enforced at compile time.
        let [first, rest @ ..] = Self::ALL;
        rest.into_iter().fold(first, |best, cand| {
            // `total_cmp` orders by distance without a float `==`
            // (which `clippy::float_cmp` rejects under -D warnings);
            // the `.then` tie-break makes the larger-area variant compare
            // as "less" so it wins an exact distance tie. Variant areas
            // are all distinct, so the comparator is never `Equal` and the
            // result is fully deterministic.
            if cand
                .distance_from(w, h)
                .total_cmp(&best.distance_from(w, h))
                .then(best.area().cmp(&cand.area()))
                .is_lt()
            {
                cand
            } else {
                best
            }
        })
    }
}

/// Diameter of the round BFM100 face, in pixels.
const ROUND_FACE_PX: f32 = 480.0;

/// Widget viewport dimensions and size variant.
///
/// Created from the raw `(width, height)` the host passes to `init()`.
/// Carries both the classified variant (for layout matching) and the
/// actual pixel dimensions (for `render_ui`).
#[derive(Debug, Clone, Copy)]
pub struct WidgetSize {
    pub variant: SizeVariant,
    pub width: u32,
    pub height: u32,
}

impl WidgetSize {
    #[must_use]
    pub fn from_dimensions(w: u32, h: u32) -> Self {
        Self {
            variant: SizeVariant::closest(w, h),
            width: w,
            height: h,
        }
    }

    /// Downscale factor of the actual viewport against the matched variant's
    /// canonical box: `min(width/cw, height/ch)`, clamped to `1.0`.
    ///
    /// The binding axis wins (`min`) so neither dimension overflows; the clamp
    /// keeps a larger-than-canonical viewport at authored sizes instead of
    /// inflating. This is the scale for *per-variant-layout* widgets (e.g. the
    /// digital and rectangular clock faces). A widget whose artwork is a single
    /// asset fit to one axis (e.g. the round clock dial) uses
    /// [`round_scale`](Self::round_scale) instead.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport and canonical dimensions are <= 1280, exact in f32"
    )]
    pub fn fit(self) -> f32 {
        let w_ratio = self.width as f32 / self.variant.width() as f32;
        let h_ratio = self.height as f32 / self.variant.height() as f32;
        w_ratio.min(h_ratio).min(1.0)
    }

    /// Asset-relative scale for the round face: `min(width, height) / 480`.
    /// Round widgets author their layout for the reference circle and multiply
    /// sizes by this, rather than [`fit`](Self::fit) (the rectangular downscale).
    #[must_use]
    pub fn round_scale(self) -> f32 {
        let diameter = u16::try_from(self.width.min(self.height)).unwrap_or(480);
        f32::from(diameter) / ROUND_FACE_PX
    }
}

/// Scale a native font size by `factor`, rounded to the nearest pixel with a
/// 1px minimum so a font never vanishes. `factor` is non-negative and normally
/// `<= 1.0` (e.g. the value from [`WidgetSize::fit`] or a widget's own
/// asset-relative scale).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sizes are small; factor is non-negative, so the result is small non-negative"
)]
pub fn scale_font(size: u32, factor: f32) -> u32 {
    let scaled = (size as f32 * factor).round() as u32;
    scaled.max(1)
}

/// Widget viewport dimensions, fetched from the host on demand.
///
/// The host caches these immutably at runtime construction, so this is a single
/// register read across the wasm boundary — call it as often as you need (in
/// `init`, in `render`, in helper functions) instead of stashing dimensions in
/// thread-locals.
///
/// # Panics
///
/// Off-target, where there is no host and so no real geometry to report.
/// A fabricated `0x0` would read as a genuine zero-sized widget,
/// so off-target consumers construct a [`WidgetSize`] themselves.
#[must_use]
pub fn widget_size() -> WidgetSize {
    let (width, height) = unpack(Platform::widget_size());
    WidgetSize::from_dimensions(width, height)
}

/// The widget's drawable rectangle, fetched from the host on demand.
/// Two cached reads across the wasm boundary, as cheap as [`widget_size`]; call it anywhere.
///
/// # Panics
///
/// Off-target, as [`widget_size`] does: callers there supply a [`WidgetViewport`]
/// rather than receive a fabricated rectangle.
/// Under a host, when it sends a shape this SDK does not know.
#[must_use]
pub fn widget_viewport() -> WidgetViewport {
    let (size, shape) = Platform::widget_viewport();
    let (width, height) = unpack(size);
    WidgetViewport {
        width,
        height,
        shape: ViewportShape::try_from(shape)
            .expect("BUG: host sent an invalid ViewportShape wire value"),
    }
}

/// The logical display info, fetched from the host on demand.
/// Cached host-side and immutable for the runtime's life.
///
/// # Panics
///
/// Off-target, where there is no display to describe: callers there supply
/// a [`DisplayInfo`] rather than read a fabricated device size.
/// Under a host, when it sends a shape this SDK does not know.
#[must_use]
pub fn display_info() -> DisplayInfo {
    let (size, shape_dpi) = Platform::display_info();
    let (width, height) = unpack(size);
    let (shape, dpi) = unpack(shape_dpi);
    DisplayInfo {
        width,
        height,
        shape: DisplayShape::try_from(shape)
            .expect("BUG: host sent an invalid DisplayShape wire value"),
        dpi,
    }
}

/// Each geometry query's answer, still packed as on the wire.
trait ViewportGeometryProvider {
    /// The widget viewport size, packed as `(width << 32) | height`.
    fn widget_size() -> u64;
    /// The packed widget size, and the viewport shape as its wire `u32`.
    fn widget_viewport() -> (u64, u32);
    /// The logical display size packed as `(width << 32) | height`,
    /// and its shape and dpi as `(shape << 32) | dpi`.
    fn display_info() -> (u64, u64);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_widget_size() -> u64;
        fn host_widget_viewport_shape() -> u32;
        fn host_display_size() -> u64;
        fn host_display_shape_dpi() -> u64;
    }

    pub(super) struct Host;

    impl super::ViewportGeometryProvider for Host {
        fn widget_size() -> u64 {
            unsafe { host_widget_size() }
        }

        fn widget_viewport() -> (u64, u32) {
            unsafe { (host_widget_size(), host_widget_viewport_shape()) }
        }

        fn display_info() -> (u64, u64) {
            unsafe { (host_display_size(), host_display_shape_dpi()) }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    pub(super) struct Host;

    impl super::ViewportGeometryProvider for Host {
        fn widget_size() -> u64 {
            panic!(
                "BUG: widget_size() called off-target — no host geometry exists; supply it explicitly"
            )
        }

        fn widget_viewport() -> (u64, u32) {
            panic!(
                "BUG: widget_viewport() called off-target — no host geometry exists; supply it explicitly"
            )
        }

        fn display_info() -> (u64, u64) {
            panic!(
                "BUG: display_info() called off-target — no host geometry exists; supply it explicitly"
            )
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

#[expect(
    clippy::cast_possible_truncation,
    reason = "high 32 bits selected via shift, low via truncation"
)]
fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

#[cfg(test)]
mod tests {
    use super::unpack;
    use crate::wasm_test;

    #[wasm_test]
    fn unpack_splits_the_high_half_from_the_low() {
        const WIDTH: u32 = 1_280;
        const HEIGHT: u32 = 480;
        assert_eq!(
            unpack((u64::from(WIDTH) << 32) | u64::from(HEIGHT)),
            (WIDTH, HEIGHT)
        );
        assert_eq!(unpack(u64::MAX), (u32::MAX, u32::MAX));
        assert_eq!(unpack(0), (0, 0));
    }
}

#[cfg(test)]
mod size_variant_tests {
    use super::{SizeVariant, WidgetSize};
    use crate::wasm_test;

    fn variant_of(w: u32, h: u32) -> SizeVariant {
        WidgetSize::from_dimensions(w, h).variant
    }

    #[wasm_test]
    fn exact_bmc100_dimensions_keep_their_variant() {
        assert_eq!(variant_of(1_280, 480), SizeVariant::Full);
        assert_eq!(variant_of(638, 480), SizeVariant::Large);
        assert_eq!(variant_of(638, 238), SizeVariant::Medium);
        assert_eq!(variant_of(317, 238), SizeVariant::Small);
    }

    #[wasm_test]
    fn non_deck_fullscreen_viewports_map_to_closest_legacy_variant() {
        assert_eq!(variant_of(320, 240), SizeVariant::Small);
        // BMM101 (480x320) lands on Large by a thin margin: normalized
        // distance Large 0.581 vs Medium 0.592. This assertion guards that
        // margin — a change to any canonical variant size that flips BMM101
        // from Large to Medium must update this test deliberately, not
        // silently. If the mapping target is wrong, fix the spec, not the test.
        assert_eq!(variant_of(480, 320), SizeVariant::Large);
        assert_eq!(variant_of(480, 480), SizeVariant::Large);
    }

    #[wasm_test]
    fn classification_is_deterministic() {
        // Same input, same output across repeated calls.
        assert_eq!(variant_of(200, 200), variant_of(200, 200));
    }
}

#[cfg(test)]
mod fit_and_scale_tests {
    use super::{WidgetSize, scale_font};
    use crate::wasm_test;

    fn fit_of(w: u32, h: u32) -> f32 {
        WidgetSize::from_dimensions(w, h).fit()
    }

    #[wasm_test]
    fn canonical_viewport_fits_at_one() {
        assert!((fit_of(1_280, 480) - 1.0).abs() < 1e-4);
        assert!((fit_of(638, 480) - 1.0).abs() < 1e-4);
        assert!((fit_of(317, 238) - 1.0).abs() < 1e-4);
    }

    #[wasm_test]
    fn medium_canonical_fits_at_one() {
        // 638x238 is Medium's canonical box, so fit() = 1.0 — a Medium-authored
        // font renders at its authored size there, shrinking only below canonical.
        // (The round dial's geometry scales separately by its own dial ratio;
        // its per-variant text annotations use fit, like the rectangular face.)
        assert!((fit_of(638, 238) - 1.0).abs() < 1e-4);
    }

    #[wasm_test]
    fn bmm101_large_downscales_by_binding_axis() {
        // 480x320 classifies to Large (638x480); min(480/638, 320/480) = 0.6667,
        // the height (binding) axis wins so neither dimension overflows.
        assert!((fit_of(480, 320) - 0.666_67).abs() < 1e-3);
    }

    #[wasm_test]
    fn larger_than_canonical_clamps_to_one() {
        // 1920x720 classifies to Full (1280x480); ratios 1.5/1.5 clamp to 1.0.
        assert!((fit_of(1_920, 720) - 1.0).abs() < 1e-4);
    }

    #[wasm_test]
    fn scale_font_at_unit_factor_is_identity() {
        assert_eq!(scale_font(24, 1.0), 24);
    }

    #[wasm_test]
    fn scale_font_halves_and_rounds() {
        assert_eq!(scale_font(24, 0.5), 12);
        assert_eq!(scale_font(16, 0.5), 8);
    }

    #[wasm_test]
    fn scale_font_floors_at_one_px() {
        // Degenerate factors never yield a 0px (invisible) font.
        assert_eq!(scale_font(16, 0.01), 1);
    }

    #[wasm_test]
    fn rect_numeral_size_shrinks_on_bmm101() {
        // Pins the size the rect fix must produce: Large numerals (40px authored)
        // on BMM101's 480x320 scale to 27px.
        let fit = WidgetSize::from_dimensions(480, 320).fit();
        assert_eq!(scale_font(40, fit), 27);
    }
}

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

//! The SDK's half of the tree: draws that register their asset with the host,
//! and the submission of an encoded tree to the host for layout and drawing.
//!
//! The tree, its builders and its codec are `bmc-ui`'s, re-exported here.

use std::cell::RefCell;

pub use bmc_ui::tree::*;
use bmc_ui::{
    Bitmap, Color, Mesh, MeshView, NinePatchAsset, RED_50, Svg, TextAlign, TextStyle,
    VerticalAlign, encode, encode_into,
};

use crate::assets::{
    ensure_bitmap_registered, ensure_mesh_registered, ensure_nine_patch_registered,
    ensure_registered,
};
use crate::host;

/// The draws that take an asset: each registers it with the host
/// (idempotent by name, see [`crate::assets`]) and hands the id to the shared constructor.
pub trait DrawAssets: Sized {
    /// Svg at local position within canvas.
    ///
    /// Use `TRANSPARENT` (0) as color to render with original SVG colors,
    /// or pass a color to tint the entire icon.
    fn svg(x: f32, y: f32, w: f32, h: f32, icon: &Svg, color: Color) -> Self;

    /// Fit `svg` inside a `size`×`size` box, preserving its aspect ratio and centering it
    /// — a browser's default `xMidYMid meet`.
    /// The host scales X and Y independently, so drawing a non-square glyph straight
    /// into a square box via [`DrawAssets::svg`] would stretch it;
    /// this reads the glyph's [`Svg::viewbox`] and shrinks the larger axis to fit.
    ///
    /// Use `TRANSPARENT` (0) as color to render with original SVG colors,
    /// or pass a color to tint the entire icon.
    fn svg_contain(svg: &Svg, size: f32, color: Color) -> Self;

    /// Bitmap (raster image) at local position within canvas.
    /// The host decodes and uploads the texture to VRAM when it first registers.
    fn bitmap(x: f32, y: f32, w: f32, h: f32, bmp: &Bitmap) -> Self;

    /// 9-patch at local position within canvas.
    fn nine_patch(
        x: impl PropsFieldValue<f32>,
        y: impl PropsFieldValue<f32>,
        w: impl PropsFieldValue<f32>,
        h: impl PropsFieldValue<f32>,
        asset: &NinePatchAsset,
    ) -> Self;

    /// 3D sphere at local position within canvas, textured by `bmp`.
    /// See [`Draw::sphere_id`] for the texture layout and what `zoom` and `light` mean.
    ///
    /// Prefer the [`crate::sphere!`] macro for ergonomic call sites.
    #[expect(clippy::too_many_arguments)]
    fn sphere(
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
    ) -> Self;

    /// 3D mesh at local position within canvas; see [`Draw::mesh_id`].
    ///
    /// A mesh `include_mesh!` could not pack draws its [`Mesh::error`] in its place.
    ///
    /// # Examples
    /// ```ignore
    /// Draw::mesh(0.0, 0.0, 200.0, 200.0, &SUZANNE, MeshView {
    ///     orientation: Orientation::from_euler(30.0, 45.0, 0.0),
    ///     ..Default::default()
    /// })
    /// ```
    fn mesh(x: f32, y: f32, w: f32, h: f32, mdl: &Mesh, view: MeshView) -> Self;
}

impl DrawAssets for Draw {
    fn svg(x: f32, y: f32, w: f32, h: f32, icon: &Svg, color: Color) -> Self {
        Self::svg_id(x, y, w, h, ensure_registered(icon), color)
    }

    fn svg_contain(svg: &Svg, size: f32, color: Color) -> Self {
        let (vw, vh) = svg.viewbox();
        let scale = (size / vw).min(size / vh);
        let w = vw * scale;
        let h = vh * scale;
        Self::svg((size - w) / 2.0, (size - h) / 2.0, w, h, svg, color)
    }

    fn bitmap(x: f32, y: f32, w: f32, h: f32, bmp: &Bitmap) -> Self {
        Self::bitmap_id(x, y, w, h, ensure_bitmap_registered(bmp))
    }

    fn nine_patch(
        x: impl PropsFieldValue<f32>,
        y: impl PropsFieldValue<f32>,
        w: impl PropsFieldValue<f32>,
        h: impl PropsFieldValue<f32>,
        asset: &NinePatchAsset,
    ) -> Self {
        Self::nine_patch_id(x, y, w, h, ensure_nine_patch_registered(asset))
    }

    fn sphere(
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
        Self::sphere_id(
            x,
            y,
            w,
            h,
            ensure_bitmap_registered(bmp),
            center_lat,
            center_lon,
            zoom,
            light,
            atmosphere,
        )
    }

    fn mesh(x: f32, y: f32, w: f32, h: f32, mdl: &Mesh, view: MeshView) -> Self {
        if let Some(error) = mdl.error {
            let style = TextStyle {
                color: RED_50,
                align: TextAlign::Center,
                vertical_align: VerticalAlign::Center,
                ..TextStyle::default()
            };
            return Self::autofit_text(x, y, w, h, error, style);
        }
        Self::mesh_id(x, y, w, h, ensure_mesh_registered(mdl), view)
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
        <$crate::Draw as $crate::DrawAssets>::sphere(
            $x, $y, $w, $h, $bmp, $lat, $lon, $z, None, false,
        )
    };
    ($bmp:expr, at: ($x:expr, $y:expr, $w:expr, $h:expr),
     center: ($lat:expr, $lon:expr), zoom: $z:expr, atmosphere) => {
        <$crate::Draw as $crate::DrawAssets>::sphere(
            $x, $y, $w, $h, $bmp, $lat, $lon, $z, None, true,
        )
    };
    ($bmp:expr, at: ($x:expr, $y:expr, $w:expr, $h:expr),
     center: ($lat:expr, $lon:expr), zoom: $z:expr,
     light: ($slat:expr, $slon:expr)) => {
        <$crate::Draw as $crate::DrawAssets>::sphere(
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
        <$crate::Draw as $crate::DrawAssets>::sphere(
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

thread_local! {
    static TREE_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(4096));
}

/// Begin building a tree (clears buffer)
pub fn begin_tree() {
    TREE_BUFFER.with(|buf| buf.borrow_mut().clear());
}

/// Submit the encoded tree to the host and clear the buffer.
pub fn submit_and_clear(width: u32, height: u32) {
    TREE_BUFFER.with(|buf| host::submit_tree(&buf.borrow(), width, height));
}

/// Encode a node to its wire bytes, for a native host that decodes them itself.
#[must_use]
pub fn serialize_node_to_bytes(node: &Node) -> Vec<u8> {
    encode(node)
}

/// Render UI tree using host-side layout.
/// Returns click and drag interactions keyed by string IDs.
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the widget hands its frame's tree over; nothing reuses one"
)]
pub fn render_ui(width: u32, height: u32, root: Node) -> TreeRenderResult {
    let keys = interaction_keys(&root);

    begin_tree();
    TREE_BUFFER.with(|buf| encode_into(&mut buf.borrow_mut(), &root));
    submit_and_clear(width, height);

    let mut result = TreeRenderResult::default();
    for key in keys {
        if let Some(hit) = host::get_touch_click(&key) {
            result.clicks.insert(key.clone(), hit);
        }
        if let Some(hit) = host::get_touch_drag(&key) {
            result.drags.insert(key, hit);
        }
    }
    result
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod failed_mesh_tests {
    use super::*;
    use bmc_wasm_protocol::StaticAssetSource;

    /// What `include_mesh!` leaves behind in a build that defers its errors.
    static FAILED: Mesh = Mesh {
        source: StaticAssetSource::embedded(&[]),
        face_normals: &[],
        name: "failed-mesh-test",
        error: Some("include_mesh!(\"missing.glb\") failed: mesh file not found"),
    };

    #[test]
    fn a_mesh_that_failed_to_pack_draws_its_error_in_its_place() {
        let draw = Draw::mesh(10.0, 20.0, 200.0, 100.0, &FAILED, MeshView::default());

        let Draw::AutofitText { text, .. } = draw else {
            panic!("a failed mesh drew {draw:?}");
        };
        assert_eq!(Some(text.as_str()), FAILED.error);
    }
}

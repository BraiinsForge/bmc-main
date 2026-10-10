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

//! Native asset glue: the `include_*!` macros over the shared descriptors,
//! and their registration with the renderer an embedder installs.
//!
//! Registration goes through one [`AssetHost`] per thread, installed with
//! [`install_asset_host`], which wires the skin registrar to it as well.

use std::cell::RefCell;

pub use bmc_render_skin::{NinePatchAsset, SkinRegistry, ensure_nine_patch_registered};
use bmc_ui::components::switcher::switcher_with_ids;
use bmc_ui::{
    Bitmap, BitmapId, Color, Draw, Mesh, MeshId, MeshView, Node, PropsFieldValue, RED_50,
    StaticAssetSource, Svg, SvgId, SwitcherTab, TextAlign, TextStyle, VerticalAlign,
};

#[doc(hidden)]
pub mod __macros {
    pub use bmc_render_macros::*;
}

// The proc macros name the descriptor types through the root they are handed,
// and `$crate::ui` re-exports them here whichever path a caller reached the wrapper by.

/// Compile an SVG into compact path data at build time, as a [`Svg`].
///
/// ```ignore
/// const STAR: Svg = include_svg!("assets/star.svg");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// walking up to parent directories for cross-crate scene files.
#[macro_export]
macro_rules! include_svg {
    ($($t:tt)*) => { $crate::assets::__macros::include_svg!($crate::ui, $($t)*) };
}

/// Package a raster image at build time, as a [`Bitmap`].
///
/// ```ignore
/// const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
/// ```
#[macro_export]
macro_rules! include_bitmap {
    ($($t:tt)*) => { $crate::assets::__macros::include_bitmap!($crate::ui, $($t)*) };
}

/// Package an audio file (WAV, OGG, MP3) at build time, as an `Audio`.
///
/// ```ignore
/// const TICK: Audio = include_audio!("assets/sounds/tick.wav");
/// ```
#[macro_export]
macro_rules! include_audio {
    ($($t:tt)*) => { $crate::assets::__macros::include_audio!($crate::ui, $($t)*) };
}

/// Pack a glTF 2.0 binary mesh at build time, as a [`Mesh`].
///
/// ```ignore
/// static SUZANNE: Mesh = include_mesh!("assets/suzanne.glb");
/// ```
///
/// With the `deferred-mesh-errors` feature, a mesh that fails validation compiles anyway
/// and [`DrawAssets::mesh`] draws the error in its place.
#[macro_export]
macro_rules! include_mesh {
    ($($t:tt)*) => { $crate::assets::__macros::include_mesh!($crate::ui, $($t)*) };
}

/// Package a `.9.png` at build time, as a [`NinePatchAsset`] with its insets decoded.
///
/// ```ignore
/// const BUTTON_BG: NinePatchAsset = include_nine_patch!("assets/button.9.png");
/// ```
#[macro_export]
macro_rules! include_nine_patch {
    ($($t:tt)*) => { $crate::assets::__macros::include_nine_patch!($crate::ui, $($t)*) };
}

/// Embed a skin directory or zip at build time, as a `Skin` with named assets.
///
/// ```ignore
/// static MY_SKIN: Skin = include_skin!("assets/my-skin/");
/// ```
#[macro_export]
macro_rules! include_skin {
    ($($t:tt)*) => { $crate::assets::__macros::include_skin!($crate::ui, $($t)*) };
}

/// What a renderer does with an asset's bytes, done here by the embedder.
pub trait AssetHost {
    fn register_svg(&self, tag: &str, data: &[u8]) -> Option<SvgId>;
    fn register_bitmap(&self, tag: &str, data: &[u8]) -> Option<BitmapId>;
    fn register_bitmap_nearest(&self, tag: &str, data: &[u8]) -> Option<BitmapId>;
    fn register_mesh(&self, tag: &str, data: &[u8]) -> Option<MeshId>;
}

thread_local! {
    static ASSET_HOST: RefCell<Option<Box<dyn AssetHost>>> = const { RefCell::new(None) };
}

/// Install this thread's asset host, which the skin registers through too.
pub fn install_asset_host(host: impl AssetHost + 'static) {
    ASSET_HOST.with(|slot| *slot.borrow_mut() = Some(Box::new(host)));
    bmc_render_skin::init(register_skin_bitmap);
}

fn with_host<R>(register: impl FnOnce(&dyn AssetHost) -> R) -> R {
    ASSET_HOST.with(|slot| {
        let host = slot.borrow();
        register(
            host.as_deref()
                .expect("BUG: no asset host installed on this thread — call install_asset_host()"),
        )
    })
}

/// Register an icon (host-side dedup by `icon.name`) and return its id.
#[must_use]
pub fn ensure_registered(icon: &Svg) -> Option<SvgId> {
    with_host(|host| host.register_svg(icon.name, icon.source.data()))
}

/// Register a bitmap (host-side dedup by `bmp.name`) and return its id.
#[must_use]
pub fn ensure_bitmap_registered(bmp: &Bitmap) -> Option<BitmapId> {
    with_host(|host| host.register_bitmap(bmp.name, bmp.source.data()))
}

/// Register a mesh (host-side dedup by `mesh.name`) and return its id.
#[must_use]
pub fn ensure_mesh_registered(mesh: &Mesh) -> Option<MeshId> {
    with_host(|host| host.register_mesh(mesh.name, mesh.source.data()))
}

/// A skin bitmap, registered with nearest-neighbour filtering.
fn register_skin_bitmap(tag: &str, source: StaticAssetSource) -> Option<BitmapId> {
    with_host(|host| host.register_bitmap_nearest(tag, source.data()))
}

/// The draws that take an asset: each registers it with the installed host
/// (idempotent by name) and hands the id to the shared constructor.
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
    fn svg_contain(svg: &Svg, size: f32, color: Color) -> Self;

    /// Bitmap (raster image) at local position within canvas.
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

/// An icon tab and the click id it emits when tapped.
#[derive(Clone, Copy, Debug)]
pub struct Tab {
    pub icon: &'static Svg,
    pub click_id: &'static str,
}

/// A segmented view switcher: a rounded pill of icon tabs, `active` highlighted.
/// A `disabled` switcher renders dimmed and registers no hit regions.
#[must_use]
pub fn switcher(active: usize, disabled: bool, tabs: &[Tab]) -> Node {
    switcher_with_ids(
        active,
        disabled,
        tabs.iter()
            .map(|t| SwitcherTab {
                icon: ensure_registered(t.icon),
                click_id: t.click_id.to_owned(),
            })
            .collect(),
    )
}

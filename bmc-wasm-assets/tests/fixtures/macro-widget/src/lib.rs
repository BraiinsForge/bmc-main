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

use bmc_wasm_sdk::{
    Audio, Bitmap, Mesh, NinePatchAsset, Skin, Svg, include_audio, include_bitmap, include_mesh,
    include_nine_patch, include_skin, include_svg,
};

const SVG: Svg = include_svg!("widgets-wasm/lib/remote-image/assets/renew.svg");
const BITMAP: Bitmap = include_bitmap!("widgets-wasm/spacex-launch/assets/unknown.png");
static MESH: Mesh = include_mesh!("widgets-wasm-examples/mesh-demo/assets/suzanne.glb");
const NINE_PATCH: NinePatchAsset = include_nine_patch!("bmc-render/assets/panel.9.png");
static SKIN: Skin = include_skin!("bmc-render/assets/skins/gallery_slider/");
const AUDIO: Audio =
    include_audio!("widgets-wasm-examples/metronome/assets/sounds/Perc_MetronomeQuartz_lo.wav");

#[unsafe(no_mangle)]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn selected_asset_ref(index: u32) -> *const u8 {
    let source = match index {
        0 => SVG.source,
        1 => BITMAP.source,
        2 => MESH.source,
        3 => NINE_PATCH.source,
        4 => SKIN.assets[0].source,
        5 => AUDIO.source,
        _ => return core::ptr::null(),
    };
    source.package_ref().as_bytes().as_ptr()
}

#[unsafe(no_mangle)]
#[cfg(not(target_arch = "wasm32"))]
pub extern "C" fn selected_asset_lengths() -> usize {
    [
        SVG.source,
        BITMAP.source,
        MESH.source,
        NINE_PATCH.source,
        SKIN.assets[0].source,
        AUDIO.source,
    ]
    .iter()
    .map(|source| source.data().len())
    .sum()
}

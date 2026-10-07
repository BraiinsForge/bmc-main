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

use bmc_wasm_protocol::{AudioId, BitmapId, MeshId, StaticAssetSource, SvgId};

use super::{AssetIdProvider, Audio, Bitmap, Svg};
use crate::cache::CacheSource;
use crate::host;
use crate::mesh::Mesh;

pub(super) struct Host;

impl AssetIdProvider for Host {
    fn svg(icon: &Svg) -> Option<SvgId> {
        host::register_svg_package(icon.name, icon.source.package_ref())
    }

    fn bitmap(bmp: &Bitmap) -> Option<BitmapId> {
        host::register_bitmap_package(bmp.name, bmp.source.package_ref())
    }

    fn image(source: CacheSource<'_>) -> Option<BitmapId> {
        host::register_bitmap_from_cache(source.tag())
    }

    fn mesh(mesh: &Mesh) -> Option<MeshId> {
        host::register_mesh_package(mesh.name, mesh.source.package_ref())
    }

    fn audio(audio: &Audio) -> Option<AudioId> {
        host::register_audio_package(audio.name, audio.source.package_ref())
    }

    fn skin_bitmap(tag: &str, source: StaticAssetSource) -> Option<BitmapId> {
        host::register_bitmap_nearest_package(tag, source.package_ref())
    }
}

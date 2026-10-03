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

use std::cell::RefCell;

use bmc_wasm_protocol::{AudioId, BitmapId, MeshId, SvgId};

use super::{AssetIdProvider, Audio, Bitmap, Svg};
use crate::cache::CacheSource;
use crate::mesh::Mesh;

type IconRegistrar = fn(&str, &[u8]) -> Option<SvgId>;
type BitmapRegistrar = fn(&str, &[u8]) -> Option<BitmapId>;
type MeshRegistrar = fn(&str, &[u8]) -> Option<MeshId>;
/// `(tag, rgba, width, height)` — pre-decoded pixels, upload only.
type ImageRegistrar = fn(&str, &[u8], u32, u32) -> Option<BitmapId>;

thread_local! {
    static ICON_REGISTRAR: RefCell<IconRegistrar> = RefCell::new(|_, _| panic!("BUG: icon registrar not initialized — call init_icon_registrar()"));
    static BITMAP_REGISTRAR: RefCell<BitmapRegistrar> = RefCell::new(|_, _| panic!("BUG: bitmap registrar not initialized — call init_bitmap_registrar()"));
    static IMAGE_REGISTRAR: RefCell<ImageRegistrar> = RefCell::new(|_, _, _, _| panic!("BUG: image registrar not initialized — call init_image_registrar()"));
    static MESH_REGISTRAR: RefCell<MeshRegistrar> = RefCell::new(|_, _| panic!("BUG: mesh registrar not initialized — call init_mesh_registrar()"));
}

/// Initialize the icon registrar for native targets.
pub fn init_icon_registrar(f: IconRegistrar) {
    ICON_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Initialize the bitmap registrar for native targets.
pub fn init_bitmap_registrar(f: BitmapRegistrar) {
    BITMAP_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Initialize the image registrar for native targets.
pub fn init_image_registrar(f: ImageRegistrar) {
    IMAGE_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Initialize the mesh registrar for native targets.
pub fn init_mesh_registrar(f: MeshRegistrar) {
    MESH_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Install dummy-id registrars for widget unit tests,
/// which assemble asset-bearing nodes without rendering them.
///
/// Per-thread, so call it at the top of each test.
/// Anything actually rendering (the gallery) must install real registrars instead.
pub fn init_test_registrars() {
    init_icon_registrar(|_, _| SvgId::from_wire(1));
    init_bitmap_registrar(|_, _| BitmapId::from_wire(1));
    init_mesh_registrar(|_, _| MeshId::from_wire(1));
    init_image_registrar(|_, _, _, _| BitmapId::from_wire(1));
}

pub(super) struct Host;

impl AssetIdProvider for Host {
    fn svg(icon: &Svg) -> Option<SvgId> {
        ICON_REGISTRAR.with(|r| r.borrow()(icon.name, icon.source.data()))
    }

    fn bitmap(bmp: &Bitmap) -> Option<BitmapId> {
        BITMAP_REGISTRAR.with(|r| r.borrow()(bmp.name, bmp.source.data()))
    }

    fn image(source: CacheSource<'_>) -> Option<BitmapId> {
        let entry = crate::cache::read_bytes(source.tag())?;
        let (width, height, _identity) = bmc_wasm_protocol::decode_image_meta(&entry.metadata)?;
        IMAGE_REGISTRAR.with(|r| r.borrow()(source.tag(), &entry.bytes, width, height))
    }

    fn mesh(mesh: &Mesh) -> Option<MeshId> {
        MESH_REGISTRAR.with(|r| r.borrow()(mesh.name, mesh.source.data()))
    }

    fn audio(_audio: &Audio) -> Option<AudioId> {
        panic!("ensure_audio_registered() is not available on native targets")
    }
}

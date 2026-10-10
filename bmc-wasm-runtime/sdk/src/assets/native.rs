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

use bmc_wasm_protocol::{AudioId, BitmapId, MeshId, StaticAssetSource, SvgId};

use super::{AssetIdProvider, Audio, Bitmap, Svg};
use crate::cache::CacheSource;
use crate::mesh::Mesh;

/// What the runtime does with an asset's bytes, done here by the embedder.
pub trait AssetHost {
    fn register_svg(&self, tag: &str, data: &[u8]) -> Option<SvgId>;
    fn register_bitmap(&self, tag: &str, data: &[u8]) -> Option<BitmapId>;
    fn register_bitmap_nearest(&self, tag: &str, data: &[u8]) -> Option<BitmapId>;
    fn register_mesh(&self, tag: &str, data: &[u8]) -> Option<MeshId>;
    /// Pre-decoded pixels, upload only.
    fn register_bitmap_rgba(
        &self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId>;
}

thread_local! {
    static ASSET_HOST: RefCell<Option<Box<dyn AssetHost>>> = const { RefCell::new(None) };
}

/// Install this thread's asset host.
pub fn install_asset_host(host: impl AssetHost + 'static) {
    ASSET_HOST.with(|slot| *slot.borrow_mut() = Some(Box::new(host)));
}

/// Install a host handing out dummy ids for widget unit tests,
/// which assemble asset-bearing nodes without rendering them.
///
/// Per-thread, so call it at the top of each test.
/// Anything actually rendering (the gallery) must install a real host instead.
pub fn init_test_registrars() {
    install_asset_host(TestAssetHost);
}

struct TestAssetHost;

impl AssetHost for TestAssetHost {
    fn register_svg(&self, _: &str, _: &[u8]) -> Option<SvgId> {
        SvgId::from_wire(1)
    }

    fn register_bitmap(&self, _: &str, _: &[u8]) -> Option<BitmapId> {
        BitmapId::from_wire(1)
    }

    fn register_bitmap_nearest(&self, _: &str, _: &[u8]) -> Option<BitmapId> {
        BitmapId::from_wire(1)
    }

    fn register_mesh(&self, _: &str, _: &[u8]) -> Option<MeshId> {
        MeshId::from_wire(1)
    }

    fn register_bitmap_rgba(&self, _: &str, _: &[u8], _: u32, _: u32) -> Option<BitmapId> {
        BitmapId::from_wire(1)
    }
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

pub(super) struct Host;

impl AssetIdProvider for Host {
    fn svg(icon: &Svg) -> Option<SvgId> {
        with_host(|host| host.register_svg(icon.name, icon.source.data()))
    }

    fn bitmap(bmp: &Bitmap) -> Option<BitmapId> {
        with_host(|host| host.register_bitmap(bmp.name, bmp.source.data()))
    }

    fn image(source: CacheSource<'_>) -> Option<BitmapId> {
        let entry = crate::cache::read_bytes(source.tag())?;
        let (width, height, _identity) = bmc_wasm_protocol::decode_image_meta(&entry.metadata)?;
        with_host(|host| host.register_bitmap_rgba(source.tag(), &entry.bytes, width, height))
    }

    fn mesh(mesh: &Mesh) -> Option<MeshId> {
        with_host(|host| host.register_mesh(mesh.name, mesh.source.data()))
    }

    fn audio(_audio: &Audio) -> Option<AudioId> {
        panic!("ensure_audio_registered() is not available on native targets")
    }

    fn skin_bitmap(tag: &str, source: StaticAssetSource) -> Option<BitmapId> {
        with_host(|host| host.register_bitmap_nearest(tag, source.data()))
    }
}

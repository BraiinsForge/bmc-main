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

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::{AssetTagState, Renderer};
use bmc_wasm_protocol::{
    BitmapId, MeshId, PackageAssetId, PackageAssetKind, PackageAssetRef, SvgId,
};
use bmc_wasm_runtime::{
    DiskCache, PackageAssetStore, RenderStatus, RuntimeConfig, WasmWidgetRuntime,
};

#[path = "common/asset_fixtures.rs"]
mod asset_fixtures;
mod common;
use asset_fixtures::{compiled_empty_svg, one_px_png, renderer_ptr, wat_string_literal};
use common::headless_egl;

const REGISTERED_TAG: &str = "resident";
const UNKNOWN_TAG: &str = "unknown";
const DORMANT_SVG_TAG: &str = "dormant-svg";
const DORMANT_BITMAP_TAG: &str = "dormant-bitmap";
const DORMANT_NEAREST_TAG: &str = "dormant-nearest";
const DORMANT_MESH_TAG: &str = "dormant-mesh";
const DORMANT_FIT_TAG: &str = "dormant-fit";
const DORMANT_CACHE_TAG: &str = "dormant-cache";
const IMAGE_DECODE_COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
enum AssetKind {
    Svg,
    Bitmap,
    BitmapNearest,
    Mesh,
}

impl AssetKind {
    fn import_name(self) -> &'static str {
        match self {
            Self::Svg => "host_register_svg",
            Self::Bitmap => "host_register_bitmap",
            Self::BitmapNearest => "host_register_bitmap_nearest",
            Self::Mesh => "host_register_mesh",
        }
    }

    fn fixture(self) -> Vec<u8> {
        match self {
            Self::Svg => compiled_line_svg(),
            Self::Bitmap | Self::BitmapNearest => one_px_png([0, 255, 0, 255]),
            Self::Mesh => minimal_triangle_mesh(),
        }
    }

    fn package_import_name(self) -> &'static str {
        match self {
            Self::Svg => "host_register_svg_package",
            Self::Bitmap => "host_register_bitmap_package",
            Self::BitmapNearest => "host_register_bitmap_nearest_package",
            Self::Mesh => "host_register_mesh_package",
        }
    }

    fn package_kind(self) -> PackageAssetKind {
        match self {
            Self::Svg => PackageAssetKind::Svg,
            Self::Bitmap | Self::BitmapNearest => PackageAssetKind::Bitmap,
            Self::Mesh => PackageAssetKind::Mesh,
        }
    }

    fn assert_resident(self, renderer: &FemtoVgRenderer, tag: &str, expected_id: u32) {
        match self {
            Self::Svg => {
                let id = SvgId::from_ffi(expected_id).expect("BUG: SVG import returned invalid ID");
                assert_eq!(renderer.svg_tag_state(tag), AssetTagState::Resident(id));
            }
            Self::Bitmap | Self::BitmapNearest => {
                let id = BitmapId::from_ffi(expected_id)
                    .expect("BUG: bitmap import returned invalid ID");
                assert_eq!(renderer.bitmap_tag_state(tag), AssetTagState::Resident(id));
            }
            Self::Mesh => {
                let id =
                    MeshId::from_ffi(expected_id).expect("BUG: mesh import returned invalid ID");
                assert_eq!(renderer.mesh_tag_state(tag), AssetTagState::Resident(id));
            }
        }
    }

    fn assert_suspended(self, renderer: &FemtoVgRenderer, tag: &str, expected_id: u32) {
        match self {
            Self::Svg => assert_eq!(
                renderer.svg_tag_state(tag),
                AssetTagState::Suspended(
                    SvgId::from_ffi(expected_id).expect("BUG: SVG import returned invalid ID")
                )
            ),
            Self::Bitmap | Self::BitmapNearest => assert_eq!(
                renderer.bitmap_tag_state(tag),
                AssetTagState::Suspended(
                    BitmapId::from_ffi(expected_id)
                        .expect("BUG: bitmap import returned invalid ID")
                )
            ),
            Self::Mesh => assert_eq!(
                renderer.mesh_tag_state(tag),
                AssetTagState::Suspended(
                    MeshId::from_ffi(expected_id).expect("BUG: mesh import returned invalid ID")
                )
            ),
        }
    }
}

fn package_registration_wat(kind: AssetKind, id: &[u8; 32]) -> String {
    let mut data = Vec::from(REGISTERED_TAG.as_bytes());
    let reference_ptr = data.len();
    let package_ref = PackageAssetRef::new(kind.package_kind(), PackageAssetId::from_bytes(*id));
    data.extend_from_slice(package_ref.as_bytes());
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "{import}" (func $register (param i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $wake_count (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32))
          (func (export "on_wake")
            global.get $wake_count
            i32.const 1
            i32.add
            global.set $wake_count)
          (func (export "register_valid") (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {reference_ptr}
            call $register)
          (func (export "wake_count") (result i32) global.get $wake_count))
        "#,
        import = kind.package_import_name(),
        tag_len = REGISTERED_TAG.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn package_svg_demand_wat(id: &[u8; 32]) -> String {
    bmc_wasm_sdk::init_test_registrars();
    let svg_id = SvgId::from_ffi(1).expect("BUG: first SVG registry ID must be valid");
    let tree = bmc_wasm_sdk::serialize_node_to_bytes(&bmc_wasm_sdk::canvas(
        bmc_wasm_sdk::PropsData::default(),
        [bmc_wasm_sdk::Draw::svg_builtin(
            0.0,
            0.0,
            1.0,
            1.0,
            svg_id,
            bmc_wasm_protocol::colors::WHITE,
        )],
    ));
    let mut data = Vec::from(REGISTERED_TAG.as_bytes());
    let reference_ptr = data.len();
    let package_ref = PackageAssetRef::new(PackageAssetKind::Svg, PackageAssetId::from_bytes(*id));
    data.extend_from_slice(package_ref.as_bytes());
    let tree_ptr = data.len();
    data.extend_from_slice(&tree);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_svg_package"
            (func $register (param i32 i32 i32) (result i32)))
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $wake_count (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const {tree_ptr}
            i32.const {tree_len}
            i32.const 64
            i32.const 64
            call $submit_tree)
          (func (export "on_wake")
            global.get $wake_count
            i32.const 1
            i32.add
            global.set $wake_count)
          (func (export "register_valid") (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {reference_ptr}
            call $register)
          (func (export "wake_count") (result i32) global.get $wake_count))
        "#,
        tag_len = REGISTERED_TAG.len(),
        tree_len = tree.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn cache_bitmap_demand_wat() -> String {
    let bitmap_id = BitmapId::from_ffi(1).expect("BUG: first bitmap registry ID must be valid");
    let tree = bmc_wasm_sdk::serialize_node_to_bytes(&bmc_wasm_sdk::canvas(
        bmc_wasm_sdk::PropsData::default(),
        [bmc_wasm_sdk::Draw::bitmap_id(
            0.0,
            0.0,
            1.0,
            1.0,
            Some(bitmap_id),
        )],
    ));
    let tree_ptr = DORMANT_CACHE_TAG.len();
    let mut data = Vec::from(DORMANT_CACHE_TAG.as_bytes());
    data.extend_from_slice(&tree);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_bitmap_from_cache"
            (func $register_cache (param i32 i32) (result i32)))
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $cache_id (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const {tree_ptr}
            i32.const {tree_len}
            i32.const 64
            i32.const 64
            call $submit_tree)
          (func (export "register_cache") (result i32)
            i32.const 0
            i32.const {tag_len}
            call $register_cache
            global.set $cache_id
            global.get $cache_id)
          (func (export "cache_id") (result i32) global.get $cache_id))
        "#,
        tag_len = DORMANT_CACHE_TAG.len(),
        tree_len = tree.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn selective_cache_bitmap_demand_wat() -> String {
    let bitmap_id = BitmapId::from_ffi(1).expect("BUG: first bitmap registry ID must be valid");
    let tree = bmc_wasm_sdk::serialize_node_to_bytes(&bmc_wasm_sdk::canvas(
        bmc_wasm_sdk::PropsData::default(),
        [bmc_wasm_sdk::Draw::bitmap_id(
            0.0,
            0.0,
            1.0,
            1.0,
            Some(bitmap_id),
        )],
    ));
    let unused_tag = "unused-cache";
    let unused_tag_ptr = DORMANT_CACHE_TAG.len();
    let tree_ptr = unused_tag_ptr + unused_tag.len();
    let mut data = Vec::from(DORMANT_CACHE_TAG.as_bytes());
    data.extend_from_slice(unused_tag.as_bytes());
    data.extend_from_slice(&tree);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_bitmap_from_cache"
            (func $register_cache (param i32 i32) (result i32)))
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $used_id (mut i32) (i32.const 0))
          (global $unused_id (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const {tree_ptr}
            i32.const {tree_len}
            i32.const 64
            i32.const 64
            call $submit_tree)
          (func (export "register_both") (result i32)
            i32.const 0
            i32.const {used_tag_len}
            call $register_cache
            global.set $used_id
            i32.const {unused_tag_ptr}
            i32.const {unused_tag_len}
            call $register_cache
            global.set $unused_id
            i32.const 1)
          (func (export "used_id") (result i32) global.get $used_id)
          (func (export "unused_id") (result i32) global.get $unused_id))
        "#,
        used_tag_len = DORMANT_CACHE_TAG.len(),
        unused_tag_len = unused_tag.len(),
        tree_len = tree.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn package_bitmap_fit_collision_wat(id: &[u8; 32], bitmap: &[u8]) -> String {
    let mut data = Vec::from(REGISTERED_TAG.as_bytes());
    let reference_ptr = data.len();
    let package_ref =
        PackageAssetRef::new(PackageAssetKind::Bitmap, PackageAssetId::from_bytes(*id));
    data.extend_from_slice(package_ref.as_bytes());
    let bitmap_ptr = data.len();
    data.extend_from_slice(bitmap);
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_bitmap_package"
            (func $package (param i32 i32 i32) (result i32)))
          (import "env" "host_register_bitmap_fit"
            (func $fit (param i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $ready_id (mut i32) (i32.const -1))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32))
          (func (export "__on_image_ready") (param i32 i32)
            local.get 1
            global.set $ready_id)
          (func (export "register_package") (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {reference_ptr}
            call $package)
          (func (export "start_fit") (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            i32.const 1
            i32.const 1
            i32.const 0
            i32.const 0
            i32.const 0
            call $fit)
          (func (export "ready_id") (result i32) global.get $ready_id))
        "#,
        tag_len = REGISTERED_TAG.len(),
        bitmap_len = bitmap.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn package_audio_registration_wat(id: &[u8; 32]) -> String {
    let mut data = Vec::from(REGISTERED_TAG.as_bytes());
    let reference_ptr = data.len();
    let package_ref =
        PackageAssetRef::new(PackageAssetKind::Audio, PackageAssetId::from_bytes(*id));
    data.extend_from_slice(package_ref.as_bytes());
    let data = wat_string_literal(&data);
    format!(
        r#"
        (module
          (import "env" "host_register_audio_package"
            (func $register (param i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32))
          (func (export "register_valid") (result i32)
            i32.const 0
            i32.const {tag_len}
            i32.const {reference_ptr}
            call $register))
        "#,
        tag_len = REGISTERED_TAG.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn write_package_asset(root: &std::path::Path, kind: PackageAssetKind, payload: &[u8]) -> [u8; 32] {
    let id = bmc_wasm_assets::package_asset_id(kind, payload);
    let directory = root.join("v1").join(kind.as_str());
    std::fs::create_dir_all(&directory).expect("BUG: create package asset fixture directory");
    std::fs::write(directory.join(format!("{id}.asset")), payload)
        .expect("BUG: write package asset fixture");
    *id.as_bytes()
}

fn package_asset_path(
    root: &std::path::Path,
    kind: PackageAssetKind,
    id: &[u8; 32],
) -> std::path::PathBuf {
    root.join("v1")
        .join(kind.as_str())
        .join(format!("{}.asset", PackageAssetId::from_bytes(*id)))
}

fn minimal_empty_mesh() -> Vec<u8> {
    let body_offset = bmc_wasm_protocol::mesh::HEADER_SIZE + std::mem::size_of::<[f32; 6]>();
    let mut data = vec![0_u8; body_offset];
    data[0..4].copy_from_slice(&bmc_wasm_protocol::mesh::MESH_MAGIC.to_le_bytes());
    let body_offset = u32::try_from(body_offset)
        .expect("BUG: minimal mesh offset must fit u32")
        .to_le_bytes();
    data[12..16].copy_from_slice(&body_offset);
    data[16..20].copy_from_slice(&body_offset);
    data
}

fn compiled_line_svg() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.push(0);
    data.extend_from_slice(&2_u16.to_le_bytes());
    data.push(bmc_wasm_protocol::svg::SVG_OP_MOVE_TO);
    data.extend_from_slice(&0.0_f32.to_le_bytes());
    data.extend_from_slice(&0.0_f32.to_le_bytes());
    data.push(bmc_wasm_protocol::svg::SVG_OP_LINE_TO);
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data
}

fn minimal_triangle_mesh() -> Vec<u8> {
    let vertex_offset = bmc_wasm_protocol::mesh::HEADER_SIZE + std::mem::size_of::<[f32; 6]>();
    let index_offset = vertex_offset + 3 * bmc_wasm_protocol::mesh::VERTEX_SIZE_NO_UV;
    let mut data = vec![0_u8; index_offset + 3 * std::mem::size_of::<u16>()];
    data[0..4].copy_from_slice(&bmc_wasm_protocol::mesh::MESH_MAGIC.to_le_bytes());
    data[4..8].copy_from_slice(&3_u32.to_le_bytes());
    data[8..12].copy_from_slice(&3_u32.to_le_bytes());
    data[12..16].copy_from_slice(
        &u32::try_from(vertex_offset)
            .expect("BUG: triangle vertex offset must fit u32")
            .to_le_bytes(),
    );
    data[16..20].copy_from_slice(
        &u32::try_from(index_offset)
            .expect("BUG: triangle index offset must fit u32")
            .to_le_bytes(),
    );
    for (position, index) in [0_u16, 1, 2].into_iter().enumerate() {
        let start = index_offset + position * std::mem::size_of::<u16>();
        data[start..start + std::mem::size_of::<u16>()].copy_from_slice(&index.to_le_bytes());
    }
    data
}

fn asset_registration_wat(kind: AssetKind, fixture: &[u8]) -> String {
    asset_registration_wat_for_sdk(kind, fixture, bmc_wasm_protocol::SDK_VERSION)
}

fn asset_registration_wat_for_sdk(
    kind: AssetKind,
    fixture: &[u8],
    sdk_version: (u16, u16, u16),
) -> String {
    let registered_tag_ptr = 0;
    let unknown_tag_ptr = REGISTERED_TAG.len();
    let fixture_ptr = unknown_tag_ptr + UNKNOWN_TAG.len();

    let mut data = Vec::new();
    data.extend_from_slice(REGISTERED_TAG.as_bytes());
    data.extend_from_slice(UNKNOWN_TAG.as_bytes());
    data.extend_from_slice(fixture);
    let data = wat_string_literal(&data);

    format!(
        r#"
        (module
          (import "env" "{import_name}"
            (func $register (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (func (export "__bmc_sdk_init") (result i64)
            i64.const {sdk})
          (func (export "render") (param i32))
          (func (export "register_valid") (result i32)
            (call $register
              (i32.const {registered_tag_ptr})
              (i32.const {registered_tag_len})
              (i32.const {fixture_ptr})
              (i32.const {fixture_len})))
          (func (export "register_resident_invalid") (result i32)
            (call $register
              (i32.const {registered_tag_ptr})
              (i32.const {registered_tag_len})
              (i32.const -1)
              (i32.const 1)))
          (func (export "register_unknown_invalid") (result i32)
            (call $register
              (i32.const {unknown_tag_ptr})
              (i32.const {unknown_tag_len})
              (i32.const -1)
              (i32.const 1)))
          )
        "#,
        import_name = kind.import_name(),
        registered_tag_len = REGISTERED_TAG.len(),
        unknown_tag_len = UNKNOWN_TAG.len(),
        fixture_len = fixture.len(),
        sdk = bmc_wasm_protocol::version_pack(sdk_version),
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "one WAT probe declares and invokes all six renderer registration imports"
)]
fn dormant_registration_probe_wat() -> String {
    let svg = compiled_empty_svg();
    let bitmap = one_px_png([0, 255, 0, 255]);
    let mesh = minimal_empty_mesh();
    let mut data = Vec::new();
    let mut append = |bytes: &[u8]| {
        let ptr = data.len();
        data.extend_from_slice(bytes);
        ptr
    };
    let svg_tag_ptr = append(DORMANT_SVG_TAG.as_bytes());
    let bitmap_tag_ptr = append(DORMANT_BITMAP_TAG.as_bytes());
    let nearest_tag_ptr = append(DORMANT_NEAREST_TAG.as_bytes());
    let mesh_tag_ptr = append(DORMANT_MESH_TAG.as_bytes());
    let fit_tag_ptr = append(DORMANT_FIT_TAG.as_bytes());
    let cache_tag_ptr = append(DORMANT_CACHE_TAG.as_bytes());
    let svg_ptr = append(&svg);
    let bitmap_ptr = append(&bitmap);
    let mesh_ptr = append(&mesh);
    let data = wat_string_literal(&data);

    format!(
        r#"
        (module
          (import "env" "host_register_svg"
            (func $svg (param i32 i32 i32 i32) (result i32)))
          (import "env" "host_register_bitmap"
            (func $bitmap (param i32 i32 i32 i32) (result i32)))
          (import "env" "host_register_bitmap_nearest"
            (func $nearest (param i32 i32 i32 i32) (result i32)))
          (import "env" "host_register_mesh"
            (func $mesh (param i32 i32 i32 i32) (result i32)))
          (import "env" "host_register_bitmap_fit"
            (func $fit (param i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
          (import "env" "host_register_bitmap_from_cache"
            (func $cache (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $svg_id (mut i32) (i32.const 0))
          (global $bitmap_id (mut i32) (i32.const 0))
          (global $nearest_id (mut i32) (i32.const 0))
          (global $mesh_id (mut i32) (i32.const 0))
          (global $fit_id (mut i32) (i32.const 0))
          (global $cache_id (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32))
          (func (export "__on_image_ready") (param i32 i32))
          (func (export "__on_image_dropped") (param i32))
          (func (export "register_reservations") (result i32)
            i32.const {svg_tag_ptr}
            i32.const {svg_tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $svg
            global.set $svg_id
            i32.const {bitmap_tag_ptr}
            i32.const {bitmap_tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            call $bitmap
            global.set $bitmap_id
            i32.const {nearest_tag_ptr}
            i32.const {nearest_tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            call $nearest
            global.set $nearest_id
            i32.const {mesh_tag_ptr}
            i32.const {mesh_tag_len}
            i32.const {mesh_ptr}
            i32.const {mesh_len}
            call $mesh
            global.set $mesh_id
            i32.const {cache_tag_ptr}
            i32.const {cache_tag_len}
            call $cache
            global.set $cache_id
            i32.const 1)
          (func (export "attempt_while_dormant") (result i32)
            i32.const {svg_tag_ptr}
            i32.const {svg_tag_len}
            i32.const {svg_ptr}
            i32.const {svg_len}
            call $svg
            global.set $svg_id
            i32.const {bitmap_tag_ptr}
            i32.const {bitmap_tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            call $bitmap
            global.set $bitmap_id
            i32.const {nearest_tag_ptr}
            i32.const {nearest_tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            call $nearest
            global.set $nearest_id
            i32.const {mesh_tag_ptr}
            i32.const {mesh_tag_len}
            i32.const {mesh_ptr}
            i32.const {mesh_len}
            call $mesh
            global.set $mesh_id
            i32.const {fit_tag_ptr}
            i32.const {fit_tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            i32.const 1
            i32.const 1
            i32.const 0
            i32.const 0
            i32.const 0
            call $fit
            global.set $fit_id
            i32.const {cache_tag_ptr}
            i32.const {cache_tag_len}
            call $cache
            global.set $cache_id
            i32.const 1)
          (func (export "promote_bitmap_to_cache") (result i32)
            i32.const {bitmap_tag_ptr}
            i32.const {bitmap_tag_len}
            i32.const {bitmap_ptr}
            i32.const {bitmap_len}
            i32.const 1
            i32.const 1
            i32.const 0
            i32.const 0
            i32.const 0
            call $fit)
          (func (export "promote_bitmap_from_cache") (result i32)
            i32.const {bitmap_tag_ptr}
            i32.const {bitmap_tag_len}
            call $cache)
          (func (export "svg_id") (result i32) global.get $svg_id)
          (func (export "bitmap_id") (result i32) global.get $bitmap_id)
          (func (export "nearest_id") (result i32) global.get $nearest_id)
          (func (export "mesh_id") (result i32) global.get $mesh_id)
          (func (export "fit_id") (result i32) global.get $fit_id)
          (func (export "cache_id") (result i32) global.get $cache_id))
        "#,
        svg_tag_len = DORMANT_SVG_TAG.len(),
        bitmap_tag_len = DORMANT_BITMAP_TAG.len(),
        nearest_tag_len = DORMANT_NEAREST_TAG.len(),
        mesh_tag_len = DORMANT_MESH_TAG.len(),
        fit_tag_len = DORMANT_FIT_TAG.len(),
        cache_tag_len = DORMANT_CACHE_TAG.len(),
        svg_len = svg.len(),
        bitmap_len = bitmap.len(),
        mesh_len = mesh.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn call_export(runtime: &mut WasmWidgetRuntime, renderer: &mut FemtoVgRenderer, name: &str) -> u32 {
    let result = runtime
        .with_renderer(renderer_ptr(renderer), |runtime| {
            runtime.call_export_i32(name)
        })
        .unwrap_or_else(|| panic!("BUG: missing or trapping {name} export"));
    u32::try_from(result).expect("BUG: asset registration result must fit u32")
}

fn package_runtime(kind: AssetKind, id: &[u8; 32], config: RuntimeConfig) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(package_registration_wat(kind, id))
        .expect("BUG: package registration WAT must parse");
    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: package runtime must construct")
}

fn assert_initial_suspension(kind: AssetKind, observation: RendererAssetSuspensionObservation) {
    assert_eq!(
        (
            observation.svg_suspended,
            observation.bitmap_suspended,
            observation.mesh_suspended,
        ),
        match kind {
            AssetKind::Svg => (1, 0, 0),
            AssetKind::Bitmap | AssetKind::BitmapNearest => (0, 1, 0),
            AssetKind::Mesh => (0, 0, 1),
        },
        "{kind:?} sleep must report the newly suspended registry"
    );
    #[cfg(feature = "profiling")]
    {
        let released = (
            observation.svg_path_bytes_released,
            observation.bitmap_released,
            observation.mesh_bytes_released,
        );
        match kind {
            AssetKind::Svg => assert!(
                released.0 > 0 && released.1 == 0 && released.2 == 0,
                "SVG suspension must release only non-empty path payloads: {released:?}"
            ),
            AssetKind::Bitmap | AssetKind::BitmapNearest => {
                assert!(
                    released.0 == 0 && released.1 > 0 && released.2 == 0,
                    "bitmap suspension must release only non-empty texture payloads: {released:?}"
                );
            }
            AssetKind::Mesh => assert!(
                released.0 == 0 && released.1 == 0 && released.2 > 0,
                "mesh suspension must release only non-empty buffer payloads: {released:?}"
            ),
        }
    }
}

fn assert_package_demand_fails(runtime: &mut WasmWidgetRuntime, gl: &headless_egl::HeadlessGl) {
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let registered = call_export(runtime, &mut renderer, "register_valid");
    assert_ne!(
        registered, 0,
        "registration must reserve an ID without loading the package payload"
    );
    assert_eq!(
        (
            observation.svg_suspended,
            observation.bitmap_suspended,
            observation.mesh_suspended,
        ),
        (0, 0, 0),
        "an already suspended reservation must not be counted twice"
    );
    #[cfg(feature = "profiling")]
    assert_eq!(
        (
            observation.svg_heap_bytes_released,
            observation.svg_path_bytes_released,
            observation.bitmap_released,
            observation.mesh_bytes_released,
        ),
        (0, 0, 0, 0),
        "an already suspended reservation must not release payload bytes"
    );
}

fn run_registration_contract(kind: AssetKind) {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let fixture = kind.fixture();
    let wat = asset_registration_wat(kind, &fixture);
    let wasm = wat::parse_str(&wat).expect("BUG: asset registration WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime must construct");
    let namespace = runtime.asset_namespace();
    let renderer_tag = format!("{namespace}:{REGISTERED_TAG}");

    let original_id = call_export(&mut runtime, &mut renderer, "register_valid");
    assert_ne!(original_id, 0, "{kind:?} fixture must register");
    kind.assert_resident(&renderer, &renderer_tag, original_id);

    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_resident_invalid"),
        original_id,
        "{kind:?} resident registration must not read its payload",
    );
    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_unknown_invalid"),
        0,
        "{kind:?} unknown registration must still read its payload",
    );

    kind.assert_resident(&renderer, &renderer_tag, original_id);
}

#[test]
fn svg_registration_skips_resident_payload_copy() {
    run_registration_contract(AssetKind::Svg);
}

#[test]
fn bitmap_registration_skips_resident_payload_copy() {
    run_registration_contract(AssetKind::Bitmap);
}

#[test]
fn nearest_bitmap_registration_skips_resident_payload_copy() {
    run_registration_contract(AssetKind::BitmapNearest);
}

#[test]
fn mesh_registration_skips_resident_payload_copy() {
    run_registration_contract(AssetKind::Mesh);
}

#[test]
fn initially_dormant_runtime_keeps_pointer_asset_registration_usable() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let fixture = compiled_empty_svg();
    let wat = asset_registration_wat(AssetKind::Svg, &fixture);
    let wasm = wat::parse_str(&wat).expect("BUG: asset registration WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime must construct");
    let renderer_tag = format!("{}:{REGISTERED_TAG}", runtime.asset_namespace());

    runtime.initialize_dormant();

    let id = call_export(&mut runtime, &mut renderer, "register_valid");
    assert_ne!(
        id, 0,
        "a 0.2 pointer asset has no restore source outside WASM"
    );
    AssetKind::Svg.assert_resident(&renderer, &renderer_tag, id);
}

#[test]
fn legacy_0_2_widget_keeps_the_pointer_asset_abi() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let fixture = compiled_empty_svg();
    let wat = asset_registration_wat_for_sdk(AssetKind::Svg, &fixture, (0, 2, 0));
    let wasm = wat::parse_str(&wat).expect("BUG: legacy asset registration WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: 0.2 runtime must construct");

    assert_ne!(
        call_export(&mut runtime, &mut renderer, "register_valid"),
        0,
        "the 0.2 pointer import must remain available"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the contract repeats the full suspend and restore cycle for every asset kind"
)]
fn dormant_package_assets_survive_coalesced_edges_and_restore_the_same_ids() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let package_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");

    for kind in [
        AssetKind::Svg,
        AssetKind::Bitmap,
        AssetKind::BitmapNearest,
        AssetKind::Mesh,
    ] {
        let payload = kind.fixture();
        let id = write_package_asset(package_dir.path(), kind.package_kind(), &payload);
        let wasm = wat::parse_str(package_registration_wat(kind, &id))
            .expect("BUG: package registration WAT must parse");
        let config = RuntimeConfig {
            package_assets: Some(PackageAssetStore::new(package_dir.path())),
            ..RuntimeConfig::default()
        };
        let mut runtime = WasmWidgetRuntime::new(
            &wasm,
            64,
            64,
            bmc_wasm_protocol::ViewportShape::Rectangular,
            common::test_display(64, 64),
            chrono::Local::now().fixed_offset(),
            config,
        )
        .expect("BUG: runtime must construct");
        let renderer_tag = format!("{}:{REGISTERED_TAG}", runtime.asset_namespace());
        runtime.initialize_dormant();

        let reserved_id = call_export(&mut runtime, &mut renderer, "register_valid");
        assert_ne!(reserved_id, 0, "{kind:?} package must reserve an ID");
        kind.assert_suspended(&renderer, &renderer_tag, reserved_id);

        runtime.notify_wake();
        runtime.notify_dormant();
        runtime.notify_wake();
        assert!(
            runtime
                .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
                .expect("BUG: lifecycle delivery must not trap"),
            "{kind:?} restoration must request a GPU completion fence"
        );
        kind.assert_resident(&renderer, &renderer_tag, reserved_id);

        runtime.notify_dormant();
        assert!(
            runtime
                .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
                .expect("BUG: lifecycle delivery must not trap"),
            "{kind:?} suspension must request a GPU completion fence"
        );
        let observation = runtime
            .last_asset_suspension_for_test()
            .expect("BUG: a completed sleep must record suspension accounting");
        assert_initial_suspension(kind, observation);
        assert_eq!(
            call_export(&mut runtime, &mut renderer, "register_valid"),
            reserved_id,
            "{kind:?} package registration must preserve its suspended ID"
        );

        runtime.notify_dormant();
        assert!(
            !runtime
                .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
                .expect("BUG: repeated lifecycle delivery must not trap"),
            "repeated {kind:?} suspension must not fence an already-suspended payload"
        );
        let repeated = runtime
            .last_asset_suspension_for_test()
            .expect("BUG: repeated sleep must record suspension accounting");
        assert_repeated_suspension(repeated);
        runtime.stage_deliveries();
        assert!(
            !runtime.has_staged_renderer_delivery(),
            "an idle dormant runtime with suspended assets must not request a GPU scope"
        );
        std::fs::remove_file(package_asset_path(
            package_dir.path(),
            kind.package_kind(),
            &id,
        ))
        .expect("BUG: dormant package fixture must be removable");
        assert_eq!(
            call_export(&mut runtime, &mut renderer, "register_valid"),
            reserved_id,
            "{kind:?} dedup must not open an unreferenced package payload"
        );
    }
}

#[test]
fn active_package_registration_reserves_without_reading_the_store() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let package_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");

    for kind in [
        AssetKind::Svg,
        AssetKind::Bitmap,
        AssetKind::BitmapNearest,
        AssetKind::Mesh,
    ] {
        let payload = kind.fixture();
        let id = write_package_asset(package_dir.path(), kind.package_kind(), &payload);
        let mut runtime = package_runtime(
            kind,
            &id,
            RuntimeConfig {
                package_assets: Some(PackageAssetStore::new(package_dir.path())),
                ..RuntimeConfig::default()
            },
        );
        let registered_id = call_export(&mut runtime, &mut renderer, "register_valid");
        assert_ne!(registered_id, 0, "{kind:?} package must reserve an ID");
        kind.assert_suspended(
            &renderer,
            &format!("{}:{REGISTERED_TAG}", runtime.asset_namespace()),
            registered_id,
        );
        std::fs::remove_file(package_asset_path(
            package_dir.path(),
            kind.package_kind(),
            &id,
        ))
        .expect("BUG: unreferenced package fixture must be removable");

        assert_eq!(
            call_export(&mut runtime, &mut renderer, "register_valid"),
            registered_id,
            "{kind:?} reservation dedup must not open the package payload"
        );
    }
}

#[test]
fn package_audio_loads_from_the_store_and_deduplicates_by_tag() {
    let package_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let payload = b"not decoded when the audio feature is disabled";
    let id = write_package_asset(package_dir.path(), PackageAssetKind::Audio, payload);
    let wasm = wat::parse_str(package_audio_registration_wat(&id))
        .expect("BUG: package audio WAT must parse");
    let config = RuntimeConfig {
        package_assets: Some(PackageAssetStore::new(package_dir.path())),
        ..RuntimeConfig::default()
    };
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: runtime must construct");

    let first = runtime
        .call_export_i32("register_valid")
        .expect("BUG: package audio import must return");
    std::fs::remove_file(package_asset_path(
        package_dir.path(),
        PackageAssetKind::Audio,
        &id,
    ))
    .expect("BUG: resident package-audio fixture must be removable");
    let second = runtime
        .call_export_i32("register_valid")
        .expect("BUG: repeated package audio import must return");
    assert_ne!(first, 0, "package audio must register from the host store");
    assert_eq!(second, first, "package audio must deduplicate by tag");
}

#[test]
fn package_renderer_failures_are_deferred_until_a_tree_demands_the_asset() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let valid_payload = compiled_empty_svg();
    let valid_id =
        *bmc_wasm_assets::package_asset_id(PackageAssetKind::Svg, &valid_payload).as_bytes();

    let mut missing_store = package_svg_demand_runtime(&valid_id, RuntimeConfig::default());
    assert_package_demand_fails(&mut missing_store, &gl);

    let corrupt_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let corrupt_path = package_asset_path(corrupt_dir.path(), PackageAssetKind::Svg, &valid_id);
    std::fs::create_dir_all(
        corrupt_path
            .parent()
            .expect("BUG: package asset path must have a directory"),
    )
    .expect("BUG: package fixture directory must construct");
    std::fs::write(&corrupt_path, b"corrupt payload")
        .expect("BUG: corrupt package fixture must be writable");
    let mut corrupt = package_svg_demand_runtime(
        &valid_id,
        RuntimeConfig {
            package_assets: Some(PackageAssetStore::new(corrupt_dir.path())),
            ..RuntimeConfig::default()
        },
    );
    assert_package_demand_fails(&mut corrupt, &gl);

    let invalid_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let invalid_id = write_package_asset(
        invalid_dir.path(),
        PackageAssetKind::Svg,
        b"not compiled SVG data",
    );
    let mut invalid = package_svg_demand_runtime(
        &invalid_id,
        RuntimeConfig {
            package_assets: Some(PackageAssetStore::new(invalid_dir.path())),
            ..RuntimeConfig::default()
        },
    );
    assert_package_demand_fails(&mut invalid, &gl);
}

#[test]
fn missing_package_audio_traps_instead_of_returning_zero() {
    let id = *bmc_wasm_assets::package_asset_id(PackageAssetKind::Audio, b"missing").as_bytes();
    let wasm = wat::parse_str(package_audio_registration_wat(&id))
        .expect("BUG: package audio WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime must construct");

    assert!(
        runtime.call_export_i32("register_valid").is_none(),
        "a missing package-audio store must trap instead of returning zero"
    );
}

#[test]
fn failed_demand_restore_marks_the_widget_dead_after_on_wake() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let package_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let payload = compiled_empty_svg();
    let id = write_package_asset(package_dir.path(), PackageAssetKind::Svg, &payload);
    let wasm =
        wat::parse_str(package_svg_demand_wat(&id)).expect("BUG: package demand WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            package_assets: Some(PackageAssetStore::new(package_dir.path())),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let renderer_tag = format!("{}:{REGISTERED_TAG}", runtime.asset_namespace());
    let asset_id = SvgId::from_ffi(call_export(&mut runtime, &mut renderer, "register_valid"))
        .expect("BUG: package SVG must register");

    runtime.notify_dormant();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: dormant delivery must not trap");
    assert_eq!(
        renderer.svg_tag_state(&renderer_tag),
        AssetTagState::Suspended(asset_id)
    );
    std::fs::remove_file(package_asset_path(
        package_dir.path(),
        PackageAssetKind::Svg,
        &id,
    ))
    .expect("BUG: wake-failure fixture must be removable");

    runtime.notify_wake();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: wake delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("wake_count"),
        Some(1),
        "on_wake must run before the first tree demands package restoration"
    );
    let status = runtime
        .with_renderer(renderer_ptr(&mut renderer), |runtime| runtime.render(16))
        .expect("BUG: failed restoration must still render the dead overlay");
    assert_eq!(status, RenderStatus::Dead);
    assert_eq!(
        renderer.svg_tag_state(&renderer_tag),
        AssetTagState::Suspended(asset_id),
        "failed restoration must not replace or renumber the reservation"
    );
}

#[test]
fn bitmap_fit_cannot_replace_package_backing() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let package_dir = tempfile::tempdir().expect("BUG: package tempdir must construct");
    let cache_dir = tempfile::tempdir().expect("BUG: cache tempdir must construct");
    let cache = DiskCache::new(cache_dir.path().to_path_buf(), 1_048_576);
    let bitmap = one_px_png([0x11, 0x22, 0x33, 0xFF]);
    let id = write_package_asset(package_dir.path(), PackageAssetKind::Bitmap, &bitmap);
    let wasm = wat::parse_str(package_bitmap_fit_collision_wat(&id, &bitmap))
        .expect("BUG: collision WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            package_assets: Some(PackageAssetStore::new(package_dir.path())),
            asset_cache: Some(cache.clone()),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let renderer_tag = format!("{}:{REGISTERED_TAG}", runtime.asset_namespace());
    let package_id =
        BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "register_package"))
            .expect("BUG: package bitmap must register");
    assert_eq!(call_export(&mut runtime, &mut renderer, "start_fit"), 0);
    assert!(!runtime.has_pending_image_decodes());
    assert!(cache.get(REGISTERED_TAG).is_none());
    assert_eq!(
        runtime.call_export_i32("ready_id"),
        Some(-1),
        "a backing collision must be rejected before an async decode starts"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&renderer_tag),
        AssetTagState::Suspended(package_id),
        "the package reservation must remain lazy after the rejected decode"
    );
    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_package"),
        package_id.to_ffi(),
        "package re-registration must still resolve through its immutable backing"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the contract checks six return values and every affected registry state"
)]
fn sleep_suspends_cache_assets_but_keeps_pointer_assets_resident() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let cache_dir = tempfile::tempdir().expect("BUG: cache tempdir must construct");
    let cache = DiskCache::new(cache_dir.path().to_path_buf(), 1_048_576);
    let mut metadata = Vec::from(1_u32.to_le_bytes());
    metadata.extend_from_slice(&1_u32.to_le_bytes());
    metadata.extend_from_slice(b"cache identity");
    cache
        .put(DORMANT_CACHE_TAG, 1, &metadata, &[0x11, 0x22, 0x33, 0xFF])
        .expect("BUG: cache fixture must be writable");
    cache
        .put(DORMANT_BITMAP_TAG, 1, &metadata, &[0x44, 0x55, 0x66, 0xFF])
        .expect("BUG: promotion cache fixture must be writable");
    let wasm = wat::parse_str(dormant_registration_probe_wat())
        .expect("BUG: dormant registration WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let config = RuntimeConfig {
        asset_cache: Some(cache.clone()),
        ..RuntimeConfig::default()
    };
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        config,
    )
    .expect("BUG: runtime must construct");
    let namespace = runtime.asset_namespace();

    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_reservations"),
        1
    );
    let svg_id = SvgId::from_ffi(call_export(&mut runtime, &mut renderer, "svg_id"))
        .expect("BUG: initial SVG registration must return an ID");
    let bitmap_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "bitmap_id"))
        .expect("BUG: initial bitmap registration must return an ID");
    let nearest_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "nearest_id"))
        .expect("BUG: initial nearest bitmap registration must return an ID");
    let mesh_id = MeshId::from_ffi(call_export(&mut runtime, &mut renderer, "mesh_id"))
        .expect("BUG: initial mesh registration must return an ID");
    let cache_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "cache_id"))
        .expect("BUG: initial cache registration must return an ID");

    runtime.notify_dormant();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: dormant delivery must not trap");
    let svg_tag = format!("{namespace}:{DORMANT_SVG_TAG}");
    let bitmap_tag = format!("{namespace}:{DORMANT_BITMAP_TAG}");
    let nearest_tag = format!("{namespace}:{DORMANT_NEAREST_TAG}");
    let mesh_tag = format!("{namespace}:{DORMANT_MESH_TAG}");
    let fit_tag = format!("{namespace}:{DORMANT_FIT_TAG}");
    let cache_tag = format!("{namespace}:{DORMANT_CACHE_TAG}");
    assert_eq!(
        renderer.svg_tag_state(&svg_tag),
        AssetTagState::Resident(svg_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(&bitmap_tag),
        AssetTagState::Resident(bitmap_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(&nearest_tag),
        AssetTagState::Resident(nearest_id)
    );
    assert_eq!(
        renderer.mesh_tag_state(&mesh_tag),
        AssetTagState::Resident(mesh_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Suspended(cache_id),
        "cache-backed payload must be released while its ID remains reserved"
    );

    assert_eq!(
        call_export(&mut runtime, &mut renderer, "attempt_while_dormant"),
        1
    );
    for (export, expected) in [
        ("svg_id", svg_id.to_ffi()),
        ("bitmap_id", bitmap_id.to_ffi()),
        ("nearest_id", nearest_id.to_ffi()),
        ("mesh_id", mesh_id.to_ffi()),
        ("cache_id", cache_id.to_ffi()),
    ] {
        assert_eq!(
            call_export(&mut runtime, &mut renderer, export),
            expected,
            "dormant re-registration must preserve the existing reservation"
        );
    }
    assert_ne!(
        call_export(&mut runtime, &mut renderer, "fit_id"),
        0,
        "bitmap-fit must still populate the disk cache while dormant"
    );
    assert!(
        runtime.has_pending_image_decodes(),
        "dormant bitmap-fit must start the cache-producing decode job"
    );
    let deadline = Instant::now() + IMAGE_DECODE_COMPLETION_TIMEOUT;
    let mut renderer_accessed = false;
    while runtime.has_pending_image_decodes() && Instant::now() < deadline {
        renderer_accessed |= runtime
            .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
            .expect("BUG: image decode delivery must not trap");
        std::thread::yield_now();
    }
    assert!(
        !runtime.has_pending_image_decodes(),
        "dormant bitmap-fit did not complete within {IMAGE_DECODE_COMPLETION_TIMEOUT:?}"
    );
    assert!(
        renderer_accessed,
        "a cache-backed dormant decode reservation must request a GPU fence"
    );
    let cached_fit = cache
        .get(DORMANT_FIT_TAG)
        .expect("BUG: dormant bitmap-fit must populate the disk cache");
    assert_eq!(
        cached_fit.metadata(),
        [1_u32.to_le_bytes(), 1_u32.to_le_bytes()].concat(),
        "cached bitmap-fit metadata must retain the decoded dimensions"
    );
    assert_eq!(
        cached_fit.bytes(),
        [0, 255, 0, 255],
        "cached bitmap-fit must retain the decoded pixel"
    );
    assert_eq!(
        renderer.svg_tag_state(&svg_tag),
        AssetTagState::Resident(svg_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(&bitmap_tag),
        AssetTagState::Resident(bitmap_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(&nearest_tag),
        AssetTagState::Resident(nearest_id)
    );
    assert_eq!(
        renderer.mesh_tag_state(&mesh_tag),
        AssetTagState::Resident(mesh_id)
    );
    assert!(
        matches!(
            renderer.bitmap_tag_state(&fit_tag),
            AssetTagState::Suspended(_)
        ),
        "a cached dormant decode must reserve an ID without uploading pixels"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Suspended(cache_id)
    );

    assert_eq!(
        call_export(&mut runtime, &mut renderer, "promote_bitmap_from_cache"),
        bitmap_id.to_ffi(),
        "cache promotion must retain the volatile bitmap reservation"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&bitmap_tag),
        AssetTagState::Suspended(bitmap_id),
        "cache promotion while dormant must release the resident volatile payload"
    );

    assert_ne!(
        call_export(&mut runtime, &mut renderer, "promote_bitmap_to_cache"),
        0,
        "a dormant volatile bitmap must be eligible for cache promotion"
    );
    let deadline = Instant::now() + IMAGE_DECODE_COMPLETION_TIMEOUT;
    while runtime.has_pending_image_decodes() && Instant::now() < deadline {
        runtime
            .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
            .expect("BUG: image decode delivery must not trap");
        std::thread::yield_now();
    }
    assert!(
        !runtime.has_pending_image_decodes(),
        "volatile-to-cache decode did not complete within {IMAGE_DECODE_COMPLETION_TIMEOUT:?}"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&bitmap_tag),
        AssetTagState::Suspended(bitmap_id),
        "cache promotion while dormant must release the formerly volatile payload"
    );

    runtime.notify_wake();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: wake delivery must not trap");
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Suspended(cache_id),
        "wake alone must not restore cached pixels"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&bitmap_tag),
        AssetTagState::Suspended(bitmap_id),
        "wake alone must not restore a promoted bitmap"
    );
    assert!(
        matches!(
            renderer.bitmap_tag_state(&fit_tag),
            AssetTagState::Suspended(_)
        ),
        "wake alone must not restore a bitmap-fit result"
    );
}

#[test]
fn render_restores_only_the_cache_bitmap_referenced_by_the_tree() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let cache_dir = tempfile::tempdir().expect("BUG: cache tempdir must construct");
    let cache = DiskCache::new(cache_dir.path().to_path_buf(), 1_048_576);
    let metadata = [1_u32.to_le_bytes(), 1_u32.to_le_bytes()].concat();
    for tag in [DORMANT_CACHE_TAG, "unused-cache"] {
        cache
            .put(tag, 1, &metadata, &[0x11, 0x22, 0x33, 0xFF])
            .expect("BUG: cache fixture must be writable");
    }
    let wasm = wat::parse_str(selective_cache_bitmap_demand_wat())
        .expect("BUG: selective cache demand WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            asset_cache: Some(cache),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let namespace = runtime.asset_namespace();

    assert_eq!(call_export(&mut runtime, &mut renderer, "register_both"), 1);
    let used_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "used_id"))
        .expect("BUG: used bitmap reservation must be valid");
    let unused_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "unused_id"))
        .expect("BUG: unused bitmap reservation must be valid");
    assert_eq!(
        runtime
            .with_renderer(renderer_ptr(&mut renderer), |runtime| runtime.render(16))
            .expect("BUG: selective cache-demand render must complete"),
        RenderStatus::Ok
    );

    assert_eq!(
        renderer.bitmap_tag_state(&format!("{namespace}:{DORMANT_CACHE_TAG}")),
        AssetTagState::Resident(used_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(&format!("{namespace}:unused-cache")),
        AssetTagState::Suspended(unused_id),
        "a registered cache bitmap absent from the tree must remain unloaded"
    );

    assert_eq!(
        runtime
            .with_renderer(renderer_ptr(&mut renderer), |runtime| runtime.render(16))
            .expect("BUG: repeated selective cache-demand render must complete"),
        RenderStatus::Ok
    );
    assert!(
        runtime.last_asset_restoration_for_test().is_none(),
        "resident demand-loaded assets must leave the steady-state preflight empty"
    );

    runtime.notify_dormant();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: sleep delivery after demand restoration must not trap");
    let suspension = runtime
        .last_asset_suspension_for_test()
        .expect("demand-restored assets must produce a suspension observation");
    assert_eq!(suspension.bitmap_suspended, 1);
    assert_eq!(
        renderer.bitmap_tag_state(&format!("{namespace}:{DORMANT_CACHE_TAG}")),
        AssetTagState::Suspended(used_id)
    );

    runtime.notify_wake();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: wake delivery must not trap");
    runtime.notify_dormant();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: repeated sleep delivery must not trap");
    let repeated_suspension = runtime
        .last_asset_suspension_for_test()
        .expect("repeated sleep must produce a suspension observation");
    assert_eq!(repeated_suspension.bitmap_suspended, 0);
}

#[test]
fn missing_cache_payload_is_reported_as_skipped_when_rendered() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let cache_dir = tempfile::tempdir().expect("BUG: cache tempdir must construct");
    let cache = DiskCache::new(cache_dir.path().to_path_buf(), 1_048_576);
    let mut metadata = Vec::from(1_u32.to_le_bytes());
    metadata.extend_from_slice(&1_u32.to_le_bytes());
    cache
        .put(DORMANT_CACHE_TAG, 1, &metadata, &[0x11, 0x22, 0x33, 0xFF])
        .expect("BUG: cache fixture must be writable");
    let wasm = wat::parse_str(cache_bitmap_demand_wat()).expect("BUG: cache demand WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            asset_cache: Some(cache.clone()),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let cache_tag = format!("{}:{DORMANT_CACHE_TAG}", runtime.asset_namespace());

    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_cache"),
        1
    );
    let cache_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "cache_id"))
        .expect("BUG: cache registration must return an ID");
    runtime.notify_dormant();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: dormant delivery must not trap");
    cache.evict(DORMANT_CACHE_TAG);

    runtime.notify_wake();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: wake delivery must not trap");

    assert!(
        runtime.last_asset_restoration_for_test().is_none(),
        "wake must not inspect an unused cache-backed reservation"
    );
    assert_eq!(
        runtime
            .with_renderer(renderer_ptr(&mut renderer), |runtime| runtime.render(16))
            .expect("BUG: cache-demand render must complete"),
        RenderStatus::Ok
    );

    assert_eq!(
        runtime
            .last_asset_restoration_for_test()
            .expect("BUG: rendering must record restoration accounting"),
        bmc_wasm_runtime::RendererAssetRestorationObservation {
            skipped: 1,
            ..bmc_wasm_runtime::RendererAssetRestorationObservation::default()
        },
        "a missing cache payload must not be counted as restored"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Suspended(cache_id),
        "the missing cache payload must retain its suspended reservation"
    );

    assert_eq!(
        runtime
            .with_renderer(renderer_ptr(&mut renderer), |runtime| runtime.render(16))
            .expect("BUG: repeated cache-demand render must complete"),
        RenderStatus::Ok
    );
    assert!(
        runtime.last_asset_restoration_for_test().is_none(),
        "a known cache miss must not be retried on every frame"
    );

    cache
        .put(DORMANT_CACHE_TAG, 1, &metadata, &[0x11, 0x22, 0x33, 0xFF])
        .expect("BUG: cache fixture must be writable again");
    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_cache"),
        cache_id.to_ffi(),
        "re-registration after a refill must preserve the reservation ID"
    );
    assert_eq!(
        runtime
            .with_renderer(renderer_ptr(&mut renderer), |runtime| runtime.render(16))
            .expect("BUG: refilled cache-demand render must complete"),
        RenderStatus::Ok
    );
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Resident(cache_id),
        "re-registration must re-enable demand restoration"
    );
}

#[test]
fn missing_cache_payload_is_reported_as_skipped_on_wake() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let cache_dir = tempfile::tempdir().expect("BUG: cache tempdir must construct");
    let cache = DiskCache::new(cache_dir.path().to_path_buf(), 1_048_576);
    let mut metadata = Vec::from(1_u32.to_le_bytes());
    metadata.extend_from_slice(&1_u32.to_le_bytes());
    cache
        .put(DORMANT_CACHE_TAG, 1, &metadata, &[0x11, 0x22, 0x33, 0xFF])
        .expect("BUG: cache fixture must be writable");
    let wasm = wat::parse_str(dormant_registration_probe_wat())
        .expect("BUG: dormant registration WAT must parse");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            asset_cache: Some(cache.clone()),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime must construct");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct");
    let cache_tag = format!("{}:{DORMANT_CACHE_TAG}", runtime.asset_namespace());

    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_reservations"),
        1
    );
    let cache_id = BitmapId::from_ffi(call_export(&mut runtime, &mut renderer, "cache_id"))
        .expect("BUG: cache registration must return an ID");
    runtime.notify_dormant();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: dormant delivery must not trap");
    cache.evict(DORMANT_CACHE_TAG);

    runtime.notify_wake();
    runtime
        .poll_deliveries_with_renderer(renderer_ptr(&mut renderer))
        .expect("BUG: wake delivery must not trap");

    assert_eq!(
        runtime
            .last_asset_restoration_for_test()
            .expect("BUG: wake must record restoration accounting"),
        bmc_wasm_runtime::RendererAssetRestorationObservation {
            skipped: 1,
            ..bmc_wasm_runtime::RendererAssetRestorationObservation::default()
        },
        "a missing cache payload must not be counted as restored"
    );
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Suspended(cache_id),
        "the missing cache payload must retain its suspended reservation"
    );
}

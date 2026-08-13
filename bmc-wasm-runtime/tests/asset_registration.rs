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
use bmc_wasm_protocol::{BitmapId, MeshId, SvgId};
use bmc_wasm_runtime::{DiskCache, RuntimeConfig, WasmWidgetRuntime};

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
            Self::Svg => compiled_empty_svg(),
            Self::Bitmap | Self::BitmapNearest => one_px_png([0, 255, 0, 255]),
            Self::Mesh => minimal_empty_mesh(),
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

fn asset_registration_wat(kind: AssetKind, fixture: &[u8]) -> String {
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
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
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
    runtime.poll_deliveries_with_renderer(renderer_ptr(&mut renderer));
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
    while runtime.has_pending_image_decodes() && Instant::now() < deadline {
        runtime.poll_deliveries_with_renderer(renderer_ptr(&mut renderer));
        std::thread::yield_now();
    }
    assert!(
        !runtime.has_pending_image_decodes(),
        "dormant bitmap-fit did not complete within {IMAGE_DECODE_COMPLETION_TIMEOUT:?}"
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

    runtime.notify_wake();
    runtime.poll_deliveries_with_renderer(renderer_ptr(&mut renderer));
    assert_eq!(
        renderer.bitmap_tag_state(&cache_tag),
        AssetTagState::Resident(cache_id),
        "wake must restore cached pixels into the preserved bitmap ID"
    );
    assert!(
        matches!(
            renderer.bitmap_tag_state(&fit_tag),
            AssetTagState::Resident(_)
        ),
        "wake must restore a bitmap-fit result produced while dormant"
    );
}

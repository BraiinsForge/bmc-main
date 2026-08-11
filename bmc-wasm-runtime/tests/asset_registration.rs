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

use std::fmt::Write as _;
use std::ptr::NonNull;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::{AssetTagState, Renderer};
use bmc_wasm_protocol::{BitmapId, MeshId, SvgId};
use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

mod common;
use common::headless_egl;

const REGISTERED_TAG: &str = "resident";
const UNKNOWN_TAG: &str = "unknown";

#[derive(Clone, Copy, Debug)]
enum AssetKind {
    Svg,
    Bitmap,
    BitmapNearest,
    Mesh,
}

#[derive(Clone, Copy)]
enum ExpectedState {
    Resident,
    Suspended,
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
            Self::Bitmap | Self::BitmapNearest => one_px_png(),
            Self::Mesh => minimal_empty_mesh(),
        }
    }

    fn assert_state(
        self,
        renderer: &FemtoVgRenderer,
        tag: &str,
        expected_id: u32,
        expected_state: ExpectedState,
    ) {
        match self {
            Self::Svg => {
                let id = SvgId::from_ffi(expected_id).expect("BUG: SVG import returned invalid ID");
                let expected = match expected_state {
                    ExpectedState::Resident => AssetTagState::Resident(id),
                    ExpectedState::Suspended => AssetTagState::Suspended(id),
                };
                assert_eq!(renderer.svg_tag_state(tag), expected);
            }
            Self::Bitmap | Self::BitmapNearest => {
                let id = BitmapId::from_ffi(expected_id)
                    .expect("BUG: bitmap import returned invalid ID");
                let expected = match expected_state {
                    ExpectedState::Resident => AssetTagState::Resident(id),
                    ExpectedState::Suspended => AssetTagState::Suspended(id),
                };
                assert_eq!(renderer.bitmap_tag_state(tag), expected);
            }
            Self::Mesh => {
                let id =
                    MeshId::from_ffi(expected_id).expect("BUG: mesh import returned invalid ID");
                let expected = match expected_state {
                    ExpectedState::Resident => AssetTagState::Resident(id),
                    ExpectedState::Suspended => AssetTagState::Suspended(id),
                };
                assert_eq!(renderer.mesh_tag_state(tag), expected);
            }
        }
    }
}

fn compiled_empty_svg() -> Vec<u8> {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&0_u16.to_le_bytes());
    data
}

fn one_px_png() -> Vec<u8> {
    let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(1, 1, Rgba([0, 255, 0, 255]));
    let mut data = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut data, ImageFormat::Png)
        .expect("BUG: PNG fixture must encode");
    data.into_inner()
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

fn wat_string_literal(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 4);
    for byte in bytes {
        write!(output, "\\{byte:02x}").expect("BUG: write to String cannot fail");
    }
    output
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
          (func (export "register_suspended_invalid") (result i32)
            (call $register
              (i32.const {registered_tag_ptr})
              (i32.const {registered_tag_len})
              (i32.const -1)
              (i32.const 1))))
        "#,
        import_name = kind.import_name(),
        registered_tag_len = REGISTERED_TAG.len(),
        unknown_tag_len = UNKNOWN_TAG.len(),
        fixture_len = fixture.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn renderer_ptr(renderer: &mut FemtoVgRenderer) -> NonNull<dyn Renderer> {
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null")
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
    kind.assert_state(
        &renderer,
        &renderer_tag,
        original_id,
        ExpectedState::Resident,
    );

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

    assert_eq!(renderer.suspend_prefix(&namespace), 1);
    kind.assert_state(
        &renderer,
        &renderer_tag,
        original_id,
        ExpectedState::Suspended,
    );
    assert_eq!(
        call_export(&mut runtime, &mut renderer, "register_suspended_invalid"),
        0,
        "{kind:?} suspended registration must still read its payload",
    );
    kind.assert_state(
        &renderer,
        &renderer_tag,
        original_id,
        ExpectedState::Suspended,
    );
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

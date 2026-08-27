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

#![cfg(all(target_os = "linux", feature = "testing"))]

use std::io::Cursor;

use chrono::Local;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use bmc_render::MAX_DECODE_IMAGE_PIXELS;
use bmc_wasm_runtime::{
    RuntimeConfig, RuntimeDisplayInfo, RuntimeResourceLimits, WasmWidgetRuntime,
};

#[derive(Clone, Copy)]
enum BodyDelivery {
    Guest,
    Host,
}

#[derive(Clone, Copy)]
enum CallbackBehavior {
    Inspect,
    Decode { max_width: u32, max_height: u32 },
    Trap,
}

#[expect(
    clippy::too_many_lines,
    reason = "keeping the ABI fixture in one WAT module makes its memory path auditable"
)]
fn fetch_body_wat(delivery: BodyDelivery, behavior: CallbackBehavior) -> String {
    let retain = if matches!(delivery, BodyDelivery::Host) {
        "global.get $request_id call $host_fetch_response_body_ref drop"
    } else {
        "nop"
    };
    let callback = match behavior {
        CallbackBehavior::Decode {
            max_width,
            max_height,
        } => format!(
            r"
        global.get $request_id
        i32.const 2048
        call $host_image_dimensions_ref
        global.set $dimensions
        i32.const 2048
        i64.load
        global.set $max_source_pixels
        i32.const 19
        i32.const 5
        global.get $request_id
        i32.const {max_width}
        i32.const {max_height}
        i32.const 0
        i32.const 0
        i32.const 0
        call $host_register_bitmap_fit_ref
        global.set $image_job
        global.get $request_id
        i32.const 2048
        call $host_image_dimensions_ref
        global.set $after_decode_dimensions"
        ),
        CallbackBehavior::Inspect | CallbackBehavior::Trap => r"
        global.get $request_id
        i32.const 0
        call $host_image_dimensions_ref
        global.set $null_dimensions
        global.get $request_id
        i32.const 2048
        call $host_image_dimensions_ref
        global.set $dimensions
        i32.const 2048
        i64.load
        global.set $max_source_pixels"
            .to_owned(),
    };
    let trap = match behavior {
        CallbackBehavior::Trap => "unreachable",
        CallbackBehavior::Inspect | CallbackBehavior::Decode { .. } => "nop",
    };

    format!(
        r#"
    (module
      (import "env" "host_fetch"
        (func $host_fetch
          (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))
      (import "env" "host_fetch_response_body_ref"
        (func $host_fetch_response_body_ref (param i32) (result i32)))
      (import "env" "host_fetch_cancel"
        (func $host_fetch_cancel (param i32) (result i32)))
      (import "env" "host_image_dimensions_ref"
        (func $host_image_dimensions_ref (param i32 i32) (result i64)))
      (import "env" "host_register_bitmap_fit_ref"
        (func $host_register_bitmap_fit_ref
          (param i32 i32 i32 i32 i32 i32 i32 i32)
          (result i32)))

      (memory (export "memory") 1)
      (data (i32.const 0) "GEThttps://x.test/iimage")

      (global $request_id (mut i32) (i32.const 0))
      (global $alloc_count (mut i32) (i32.const 0))
      (global $body_ptr (mut i32) (i32.const -1))
      (global $body_len (mut i32) (i32.const 0))
      (global $dimensions (mut i64) (i64.const -1))
      (global $after_decode_dimensions (mut i64) (i64.const -1))
      (global $null_dimensions (mut i64) (i64.const 0))
      (global $max_source_pixels (mut i64) (i64.const 0))
      (global $image_job (mut i32) (i32.const 0))

      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})

      (func (export "__alloc") (param $len i32) (result i32)
        global.get $alloc_count
        i32.const 1
        i32.add
        global.set $alloc_count
        i32.const 1024)

      (func (export "init")
        i32.const 10000
        i32.const 0
        i32.const 3
        i32.const 3
        i32.const 16
        i32.const 0
        i32.const 0
        i32.const 0
        i32.const 0
        call $host_fetch
        global.set $request_id
        {retain})

      (func (export "__on_fetch_response")
        (param $request_id_arg i32)
        (param $status i32)
        (param $body_ptr_arg i32)
        (param $body_len_arg i32)
        local.get $body_ptr_arg
        global.set $body_ptr
        local.get $body_len_arg
        global.set $body_len
        {callback}
        {trap})

      (func (export "render") (param i32))

      (func (export "cancel") (result i32)
        global.get $request_id
        call $host_fetch_cancel)

      (func (export "alloc_count") (result i32) global.get $alloc_count)
      (func (export "body_ptr") (result i32) global.get $body_ptr)
      (func (export "body_len") (result i32) global.get $body_len)
      (func (export "dimensions") (result i64) global.get $dimensions)
      (func (export "null_dimensions") (result i64) global.get $null_dimensions)
      (func (export "memory_prefix") (result i64)
        i32.const 0 i64.load)
      (func (export "max_source_pixels") (result i64) global.get $max_source_pixels)
      (func (export "after_decode_dimensions") (result i64)
        global.get $after_decode_dimensions)
      (func (export "image_job") (result i32) global.get $image_job)
      (func (export "ref_dimensions") (result i64)
        global.get $request_id
        i32.const 2048
        call $host_image_dimensions_ref))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn runtime(wat: String, body: Vec<u8>) -> WasmWidgetRuntime {
    runtime_with_resource_limits(wat, body, RuntimeResourceLimits::default())
}

fn runtime_with_resource_limits(
    wat: String,
    body: Vec<u8>,
    resource_limits: RuntimeResourceLimits,
) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(wat).expect("BUG: fetch-body-ref WAT must parse");
    WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        RuntimeDisplayInfo {
            width: 64,
            height: 64,
            shape: bmc_wasm_protocol::DisplayShape::Rectangular,
            dpi: 1,
        },
        Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(move |_method, _url| Some((200, body.clone())))),
            resource_limits,
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime must construct")
}

fn deliver(runtime: &mut WasmWidgetRuntime) {
    runtime.set_time(Local::now().fixed_offset(), 1);
    runtime
        .poll_deliveries()
        .expect("BUG: fetch body delivery must not trap");
}

fn image_body(format: ImageFormat) -> Vec<u8> {
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 0xFF])))
        .write_to(&mut encoded, format)
        .expect("BUG: test image must encode");
    encoded.into_inner()
}

fn png_body() -> Vec<u8> {
    image_body(ImageFormat::Png)
}

#[test]
fn retained_body_skips_guest_allocation_and_expires_after_callback() {
    let mut body = png_body();
    body.resize(1_048_576, 0);
    let mut runtime = runtime(
        fetch_body_wat(BodyDelivery::Host, CallbackBehavior::Inspect),
        body,
    );

    deliver(&mut runtime);

    assert_eq!(runtime.call_export_i32("alloc_count"), Some(0));
    assert_eq!(runtime.call_export_i32("body_ptr"), Some(0));
    assert_eq!(runtime.call_export_i32("body_len"), Some(1_048_576));
    assert_eq!(
        runtime.call_export_i64("dimensions"),
        Some((2_i64 << 32) | 2)
    );
    assert_eq!(
        runtime.call_export_i64("null_dimensions"),
        Some(-1),
        "a null metadata pointer must be rejected"
    );
    assert_eq!(
        runtime.call_export_i64("memory_prefix"),
        Some(i64::from_le_bytes(*b"GEThttps")),
        "rejecting a null metadata pointer must not write at guest address zero"
    );
    assert_eq!(
        runtime.call_export_i64("ref_dimensions"),
        Some(-1),
        "the body reference must expire when its callback returns"
    );
    assert_eq!(runtime.test_active_fetch_body_count(), 0);
}

#[test]
fn retained_dimensions_report_the_host_detected_format_limit() {
    for (format, expected_limit) in [
        (ImageFormat::Png, MAX_DECODE_IMAGE_PIXELS),
        (
            ImageFormat::Jpeg,
            MAX_DECODE_IMAGE_PIXELS.saturating_mul(64),
        ),
    ] {
        let mut runtime = runtime(
            fetch_body_wat(BodyDelivery::Host, CallbackBehavior::Inspect),
            image_body(format),
        );

        deliver(&mut runtime);

        assert_eq!(
            runtime.call_export_i64("dimensions"),
            Some((2_i64 << 32) | 2)
        );
        assert_eq!(
            runtime.call_export_i64("max_source_pixels"),
            i64::try_from(expected_limit).ok(),
            "{format:?} must expose its source-pixel allowance through the ABI"
        );
    }
}

#[test]
fn ordinary_fetch_still_allocates_and_copies_guest_bytes() {
    let mut runtime = runtime(
        fetch_body_wat(BodyDelivery::Guest, CallbackBehavior::Inspect),
        b"abc".to_vec(),
    );

    deliver(&mut runtime);

    assert_eq!(runtime.call_export_i32("alloc_count"), Some(1));
    assert_eq!(runtime.call_export_i32("body_ptr"), Some(1024));
    assert_eq!(runtime.call_export_i32("body_len"), Some(3));
    assert_eq!(runtime.test_active_fetch_body_count(), 0);
}

#[test]
fn retained_body_is_released_when_the_callback_traps() {
    let mut runtime = runtime(
        fetch_body_wat(BodyDelivery::Host, CallbackBehavior::Trap),
        vec![0xFF, 0xD8, 0xFF],
    );
    runtime.set_time(Local::now().fixed_offset(), 1);

    runtime
        .poll_deliveries()
        .expect_err("a trapping retained-body callback must reach the host");

    assert_eq!(runtime.test_active_fetch_body_count(), 0);
}

#[test]
fn cancelled_in_flight_body_ref_is_released_with_its_settlement() {
    let mut runtime = runtime(
        fetch_body_wat(BodyDelivery::Host, CallbackBehavior::Inspect),
        b"ignored".to_vec(),
    );

    assert_eq!(runtime.test_fetch_body_ref_count(), 1);
    assert_eq!(runtime.call_export_i32("cancel"), Some(0));
    deliver(&mut runtime);

    assert_eq!(runtime.test_fetch_body_ref_count(), 0);
    assert_eq!(runtime.test_active_fetch_body_count(), 0);
}

#[test]
fn bitmap_fit_consumes_the_retained_body_without_guest_allocation() {
    let body = png_body();
    let expected_len = i32::try_from(body.len()).expect("BUG: test PNG fits i32");
    let mut runtime = runtime(
        fetch_body_wat(
            BodyDelivery::Host,
            CallbackBehavior::Decode {
                max_width: 64,
                max_height: 64,
            },
        ),
        body,
    );

    deliver(&mut runtime);

    assert_eq!(runtime.call_export_i32("alloc_count"), Some(0));
    assert_eq!(runtime.call_export_i32("body_len"), Some(expected_len));
    assert_eq!(
        runtime.call_export_i64("dimensions"),
        Some((2_i64 << 32) | 2)
    );
    assert!(
        runtime
            .call_export_i32("image_job")
            .is_some_and(|job| job > 0),
        "the retained body should start an image decode"
    );
    assert_eq!(runtime.test_active_fetch_body_count(), 0);
    assert_eq!(
        runtime.call_export_i64("after_decode_dimensions"),
        Some(-1),
        "an accepted decode must consume the retained body"
    );
}

fn assert_rejected_bitmap_fit_preserves_body(
    max_width: u32,
    max_height: u32,
    resource_limits: RuntimeResourceLimits,
) {
    let mut runtime = runtime_with_resource_limits(
        fetch_body_wat(
            BodyDelivery::Host,
            CallbackBehavior::Decode {
                max_width,
                max_height,
            },
        ),
        png_body(),
        resource_limits,
    );

    deliver(&mut runtime);

    assert_eq!(runtime.call_export_i32("image_job"), Some(0));
    assert_eq!(
        runtime.call_export_i64("after_decode_dimensions"),
        Some((2_i64 << 32) | 2),
        "a rejected decode must leave the body usable for the rest of the callback"
    );
    assert_eq!(runtime.test_active_fetch_body_count(), 0);
}

#[test]
fn zero_dimension_rejection_preserves_the_retained_body() {
    assert_rejected_bitmap_fit_preserves_body(0, 64, RuntimeResourceLimits::default());
}

#[test]
fn over_budget_rejection_preserves_the_retained_body() {
    assert_rejected_bitmap_fit_preserves_body(100_000, 100_000, RuntimeResourceLimits::default());
}

#[test]
fn decode_limit_rejection_preserves_the_retained_body() {
    let resource_limits = RuntimeResourceLimits {
        max_image_decodes: 0,
        ..RuntimeResourceLimits::default()
    };

    assert_rejected_bitmap_fit_preserves_body(64, 64, resource_limits);
}

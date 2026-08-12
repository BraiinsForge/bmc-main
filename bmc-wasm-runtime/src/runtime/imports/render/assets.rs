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

//! Asset-loading imports for icons, bitmaps, meshes, and image decoding.

#![expect(clippy::cast_possible_truncation)]

use anyhow::{Result, bail};
use bmc_render::{
    MAX_DECODE_IMAGE_ALLOC_BYTES, MAX_DECODE_IMAGE_PIXELS, decode_scaled_to_cover,
    decode_scaled_to_fit,
};
use bmc_wasm_protocol::{BitmapId, ImageJobId, MeshId, SvgId};
use wasmi::{Caller, Extern, Linker};

use crate::host_api::{CompletedImageDecode, HostState};

use super::super::super::memory::read_bytes;

/// Read a tag string from guest memory. Returns `None` on out-of-bounds or
/// non-UTF-8 bytes.
fn read_tag(caller: &Caller<'_, HostState>, ptr: u32, len: u32) -> Option<String> {
    let bytes = read_bytes(caller, ptr, len)?;
    String::from_utf8(bytes).ok()
}

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_svg_import(linker)?;
    register_bitmap_import(linker)?;
    register_bitmap_nearest_import(linker)?;
    register_bitmap_fit_import(linker)?;
    register_mesh_import(linker)?;
    register_bitmap_sample_import(linker)?;
    register_image_decode_import(linker)?;
    register_max_image_pixels_import(linker)?;
    register_bitmap_from_cache_import(linker)?;
    Ok(())
}

// Restore a bitmap from the per-instance cache:
// mmap RGBA → texture, no fetch, no decode,
// and the bytes never enter wasm.
//
// Metadata is the image layer's `[w u32 | h u32 | identity]`;
// only the dims are needed to re-upload.
fn register_bitmap_from_cache_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_bitmap_from_cache",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32|
         -> Result<u32, wasmi::Error> {
            let Some(raw_tag) = read_tag(&caller, tag_ptr, tag_len) else {
                return Ok(0);
            };
            super::super::with_renderer_and_state(&mut caller, |renderer, state| {
                let Some(cache) = state.asset_cache.as_ref() else {
                    return 0;
                };
                let Some(blob) = cache.get(&raw_tag) else {
                    #[cfg(feature = "profiling")]
                    tracing::info!(target: bmc_render::profile::TARGET, tag = %raw_tag, "cache restore miss");
                    return 0;
                };
                let meta = blob.metadata();
                if meta.len() < 8 {
                    return 0;
                }
                let w = u32::from_le_bytes([meta[0], meta[1], meta[2], meta[3]]);
                let h = u32::from_le_bytes([meta[4], meta[5], meta[6], meta[7]]);
                let tag = state.namespaced_tag(&raw_tag);
                let id = renderer
                    .register_bitmap_rgba(&tag, blob.bytes(), w, h)
                    .map_or(0, BitmapId::to_ffi);
                #[cfg(feature = "profiling")]
                {
                    let age_ms = u64::try_from(state.system_time.timestamp_millis())
                        .unwrap_or(0)
                        .saturating_sub(blob.saved_at);
                    let resident = renderer.bitmap_resident_bytes();
                    tracing::info!(
                        target: bmc_render::profile::TARGET,
                        tag = %raw_tag,
                        age_ms,
                        resident,
                        "cache restore hit"
                    );
                }
                id
            })
        },
    )?;
    Ok(())
}

fn register_svg_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_svg",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         data_ptr: u32,
         data_len: u32|
         -> Result<u32, wasmi::Error> {
            let Some(tag) = read_tag(&caller, tag_ptr, tag_len) else {
                return Ok(0);
            };
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return Ok(0);
            };
            let tag = caller.data_mut().namespaced_tag(&tag);
            super::super::with_renderer(&mut caller, |renderer| {
                renderer.register_svg(&tag, &data).map_or(0, SvgId::to_ffi)
            })
        },
    )?;
    Ok(())
}

fn register_bitmap_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_bitmap",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         data_ptr: u32,
         data_len: u32|
         -> Result<u32, wasmi::Error> {
            let Some(tag) = read_tag(&caller, tag_ptr, tag_len) else {
                return Ok(0);
            };
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return Ok(0);
            };
            let tag = caller.data_mut().namespaced_tag(&tag);
            super::super::with_renderer(&mut caller, |renderer| {
                renderer
                    .register_bitmap(&tag, &data)
                    .map_or(0, BitmapId::to_ffi)
            })
        },
    )?;
    Ok(())
}

fn register_bitmap_nearest_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_bitmap_nearest",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         data_ptr: u32,
         data_len: u32|
         -> Result<u32, wasmi::Error> {
            let Some(tag) = read_tag(&caller, tag_ptr, tag_len) else {
                return Ok(0);
            };
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return Ok(0);
            };
            let tag = caller.data_mut().namespaced_tag(&tag);
            super::super::with_renderer(&mut caller, |renderer| {
                renderer
                    .register_bitmap_nearest(&tag, &data)
                    .map_or(0, BitmapId::to_ffi)
            })
        },
    )?;
    Ok(())
}

/// True if a `max_w`×`max_h` decode target fits the pixel budget — cover mode
/// resizes to exactly the target, so an unbounded target over-allocates.
/// Overflow counts as over-budget.
fn decode_target_within_budget(max_w: u32, max_h: u32) -> bool {
    u64::from(max_w)
        .checked_mul(u64::from(max_h))
        .is_some_and(|px| px <= MAX_DECODE_IMAGE_PIXELS)
}

/// Decode off the render thread, register when done.
/// Returns a job id (`0` = rejected); guest notified via `__on_image_ready`.
fn register_bitmap_fit_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_bitmap_fit",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         data_ptr: u32,
         data_len: u32,
         max_w: u32,
         max_h: u32,
         cover: u32,
         identity_ptr: u32,
         identity_len: u32|
         -> u32 {
            let Some(raw_tag) = read_tag(&caller, tag_ptr, tag_len) else {
                return 0;
            };
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return 0;
            };
            let identity = read_bytes(&caller, identity_ptr, identity_len).unwrap_or_default();
            // A zero max dimension divides by zero in the cover crop; reject it
            // here rather than trust the guest's integers.
            if max_w == 0 || max_h == 0 {
                tracing::warn!("host_register_bitmap_fit rejected: zero max dimension");
                return 0;
            }
            if !decode_target_within_budget(max_w, max_h) {
                tracing::warn!(
                    max_w,
                    max_h,
                    "host_register_bitmap_fit rejected: target over budget"
                );
                return 0;
            }
            let state = caller.data_mut();
            if state.in_flight_image_decodes as usize >= state.resource_limits.max_image_decodes {
                tracing::warn!(
                    max_image_decodes = state.resource_limits.max_image_decodes,
                    "host_register_bitmap_fit rejected: decode limit reached"
                );
                return 0;
            }
            let tag = state.namespaced_tag(&raw_tag);
            let job_id = ImageJobId::alloc(&mut state.next_image_job_id);
            state.in_flight_image_decodes += 1;
            let tx = state.image_decode_tx.clone();
            let cache = state.asset_cache.clone();
            let saved_at = u64::try_from(state.system_time.timestamp_millis()).unwrap_or(0);
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                #[cfg(feature = "profiling")]
                let probe = bmc_render::profile::MemProbe::start();
                // catch_unwind so a decode panic still reports a failed job and
                // releases the in-flight slot instead of leaking it forever.
                let result = std::panic::catch_unwind(|| {
                    if cover != 0 {
                        decode_scaled_to_cover(&data, max_w, max_h)
                    } else {
                        decode_scaled_to_fit(&data, max_w, max_h)
                    }
                    .map_err(|e| e.to_string())
                })
                .unwrap_or_else(|_| Err("image decode panicked".to_owned()));
                let decode_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                #[cfg(feature = "profiling")]
                if let Ok((_, w, h)) = &result {
                    log_host_decode_image(*w, *h, data_len, &probe);
                }
                // Write-at-decode, off the render thread; wake restores from it.
                if let (Ok((rgba, w, h)), Some(cache)) = (&result, &cache) {
                    let mut meta = Vec::with_capacity(8 + identity.len());
                    meta.extend_from_slice(&w.to_le_bytes());
                    meta.extend_from_slice(&h.to_le_bytes());
                    meta.extend_from_slice(&identity);
                    if let Err(e) = cache.put(&raw_tag, saved_at, &meta, rgba) {
                        tracing::warn!("image cache write failed ({raw_tag}): {e}");
                    }
                }
                let _ = tx.send(CompletedImageDecode {
                    job_id,
                    tag,
                    result,
                    decode_us,
                });
            });
            job_id.to_wire()
        },
    )?;
    Ok(())
}

fn register_mesh_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_mesh",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         data_ptr: u32,
         data_len: u32|
         -> Result<u32, wasmi::Error> {
            #[cfg(feature = "profiling")]
            let probe = bmc_render::profile::MemProbe::start();

            let Some(tag) = read_tag(&caller, tag_ptr, tag_len) else {
                return Ok(0);
            };
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return Ok(0);
            };
            let tag = caller.data_mut().namespaced_tag(&tag);
            let id: u32 = super::super::with_renderer(&mut caller, |renderer| {
                renderer
                    .register_mesh(&tag, &data)
                    .map_or(0, MeshId::to_ffi)
            })?;

            #[cfg(feature = "profiling")]
            log_host_register_mesh(id, data_len, &probe);

            Ok(id)
        },
    )?;
    Ok(())
}

/// Register an inert `host_bitmap_sample` so widgets built against SDK 0.2.x
/// that sampled still instantiate.
///
/// `0` is the sentinel those widgets already read as "no sampled colour".
fn register_bitmap_sample_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_bitmap_sample",
        |_bitmap_id: u32, _x: u32, _y: u32, _w: u32, _h: u32| -> u32 { 0 },
    )?;
    Ok(())
}

/// Log the FFI-side cost of `host_register_mesh`. The wasmi-side
/// ``read_bytes`` copy and the renderer-internal upload are both included;
/// `MeshRenderer::register_mesh` emits its own narrower log line for the
/// upload portion. The difference between the two is the wasmi memory copy.
#[cfg(feature = "profiling")]
fn log_host_register_mesh(id: u32, data_len: u32, probe: &bmc_render::profile::MemProbe) {
    let s = probe.snapshot();
    tracing::info!(
        target: bmc_render::profile::TARGET,
        "host_register_mesh id={id} data_len={data_len} ffi_us={ffi_us} \
         vmrss_delta_kb={vmrss:+} rss_shmem_delta_kb={shmem:+} \
         cma_free_delta_kb={cma:+} mem_free_kb={mem_free}",
        ffi_us = s.elapsed_us,
        vmrss = s.vmrss_delta_kb,
        shmem = s.rss_shmem_delta_kb,
        cma = s.cma_free_delta_kb,
        mem_free = s.mem_free_kb,
    );
}

/// The guest applies its own pixel budget and drops oversized images silently,
/// so this is the only record that one arrived, and of what it measured.
#[cfg(feature = "profiling")]
fn log_host_image_probe(w: u32, h: u32, data_len: u32) {
    tracing::info!(
        target: bmc_render::profile::TARGET,
        "host_image_probe {w}x{h} px={px} data_len={data_len}",
        px = u64::from(w) * u64::from(h),
    );
}

/// `vmrss_delta_kb` is a process-wide RSS difference sampled either side
/// of the decode: an order-of-magnitude hint, not a peak, and not attributable
/// to the decode alone.
#[cfg(feature = "profiling")]
fn log_host_decode_image(w: u32, h: u32, data_len: u32, probe: &bmc_render::profile::MemProbe) {
    let s = probe.snapshot();
    tracing::info!(
        target: bmc_render::profile::TARGET,
        "host_decode_image {w}x{h} data_len={data_len} decode_us={decode_us} \
         vmrss_delta_kb={vmrss:+} rss_shmem_delta_kb={shmem:+} mem_free_kb={mem_free}",
        decode_us = s.elapsed_us,
        vmrss = s.vmrss_delta_kb,
        shmem = s.rss_shmem_delta_kb,
        mem_free = s.mem_free_kb,
    );
}

fn register_image_decode_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_decode_image",
        |mut caller: Caller<'_, HostState>,
         data_ptr: u32,
         data_len: u32,
         rgba_out_ptr: u32,
         rgba_out_cap: u32|
         -> i64 {
            let Some(image_data) = read_bytes(&caller, data_ptr, data_len) else {
                return -1;
            };

            // A null output pointer means dimensions-only: probe the header (no
            // decode, no pixel budget) so oversized-but-valid images still
            // report their real size. The budget gates only the actual decode.
            if rgba_out_ptr == 0 {
                return match probe_image_dimensions(&image_data) {
                    Ok((w, h)) => {
                        #[cfg(feature = "profiling")]
                        log_host_image_probe(w, h, data_len);
                        (i64::from(w) << 32) | i64::from(h)
                    }
                    Err(e) => {
                        tracing::error!("host_decode_image probe: {e}");
                        -1
                    }
                };
            }

            #[cfg(feature = "profiling")]
            let probe = bmc_render::profile::MemProbe::start();
            let rgba = match decode_image_rgba_limited(&image_data) {
                Ok(rgba) => rgba,
                Err(e) => {
                    tracing::error!("host_decode_image: {e}");
                    return -1;
                }
            };
            #[cfg(feature = "profiling")]
            log_host_decode_image(rgba.width(), rgba.height(), data_len, &probe);
            let (w, h) = (rgba.width(), rgba.height());
            let pixels = rgba.as_raw();
            let needed = pixels.len() as u32;

            if needed <= rgba_out_cap && rgba_out_ptr != 0 {
                let memory = caller.get_export("memory").and_then(Extern::into_memory);
                if let Some(memory) = memory {
                    let data = memory.data_mut(&mut caller);
                    // `start + needed` would wrap on armv7 for guest-supplied
                    // ptr/len; `checked_add` catches that.
                    let start = rgba_out_ptr as usize;
                    if let Some(end) = start.checked_add(needed as usize)
                        && end <= data.len()
                    {
                        data[start..end].copy_from_slice(pixels);
                    }
                }
            }

            #[expect(clippy::cast_lossless)]
            {
                ((w as i64) << 32) | (h as i64)
            }
        },
    )?;

    Ok(())
}

fn register_max_image_pixels_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_max_image_pixels",
        |_caller: Caller<'_, HostState>| -> u32 {
            u32::try_from(MAX_DECODE_IMAGE_PIXELS).unwrap_or(u32::MAX)
        },
    )?;
    Ok(())
}

fn rgba_byte_len_limited(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    anyhow::ensure!(
        pixels <= MAX_DECODE_IMAGE_PIXELS,
        "decoded image exceeds pixel budget ({pixels} > {MAX_DECODE_IMAGE_PIXELS})"
    );
    let bytes = pixels
        .checked_mul(4)
        .expect("BUG: RGBA byte count overflow after pixel budget check");
    usize::try_from(bytes).map_err(Into::into)
}

fn probe_image_dimensions(data: &[u8]) -> Result<(u32, u32)> {
    match std::panic::catch_unwind(|| {
        image::ImageReader::new(std::io::Cursor::new(data))
            .with_guessed_format()
            .map_err(image::ImageError::IoError)
            .and_then(image::ImageReader::into_dimensions)
    }) {
        Ok(Ok(dimensions)) => Ok(dimensions),
        Ok(Err(e)) => Err(anyhow::anyhow!("{e}")),
        Err(_) => bail!("decoder panicked while probing dimensions"),
    }
}

fn decode_image_rgba_limited(data: &[u8]) -> Result<image::RgbaImage> {
    let (width, height) = probe_image_dimensions(data)?;
    let _ = rgba_byte_len_limited(width, height)?;
    let mut limits = image::io::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_IMAGE_ALLOC_BYTES);

    match std::panic::catch_unwind(|| {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(data));
        reader.limits(limits);
        reader
            .with_guessed_format()
            .map_err(image::ImageError::IoError)
            .and_then(image::ImageReader::decode)
    }) {
        Ok(Ok(img)) => Ok(img.to_rgba8()),
        Ok(Err(e)) => Err(anyhow::anyhow!("{e}")),
        Err(_) => bail!("decoder panicked"),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba, RgbaImage};

    use super::{
        MAX_DECODE_IMAGE_PIXELS, decode_image_rgba_limited, decode_target_within_budget,
        probe_image_dimensions, rgba_byte_len_limited,
    };

    #[test]
    fn decode_target_budget_rejects_oversized_and_overflow() {
        assert!(decode_target_within_budget(64, 64));
        assert!(!decode_target_within_budget(100_000, 100_000));
        assert!(!decode_target_within_budget(u32::MAX, u32::MAX));
    }

    #[test]
    fn rgba_budget_rejects_images_over_limit() {
        let mut side = 1_u32;
        while u64::from(side) * u64::from(side) <= MAX_DECODE_IMAGE_PIXELS {
            side += 1;
        }

        assert!(rgba_byte_len_limited(side, side).is_err());
    }

    #[test]
    fn decode_image_limited_accepts_small_png() {
        let img = RgbaImage::from_pixel(2, 2, Rgba([0x12, 0x34, 0x56, 0xFF]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("BUG: test PNG encoding should succeed");

        let rgba = decode_image_rgba_limited(&encoded.into_inner())
            .expect("BUG: small PNG should decode within budget");

        assert_eq!((rgba.width(), rgba.height()), (2, 2));
        assert_eq!(rgba.as_raw().len(), 16);
    }

    #[test]
    fn decode_image_limited_rejects_high_bit_depth_png_over_alloc_budget() {
        let img =
            ImageBuffer::from_pixel(2048, 2048, image::Rgba([0x1234, 0x5678, 0x9ABC, 0xFFFF]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba16(img)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("BUG: test PNG encoding should succeed");

        assert!(decode_image_rgba_limited(&encoded.into_inner()).is_err());
    }

    const PIXEL: Rgba<u8> = Rgba([0x12, 0x34, 0x56, 0xFF]);

    /// Enabled formats whose encoder reproduces `PIXEL` exactly.
    ///
    /// Farbfeld is absent because it cannot encode the fixture —
    /// image 0.25.10 answers "does not support the color type `Rgba8`".
    /// Its decoder is covered by the `deck image-formats` corpus instead.
    const LOSSLESS_FORMATS: &[ImageFormat] = &[
        ImageFormat::Bmp,
        ImageFormat::Gif,
        ImageFormat::Png,
        ImageFormat::Pnm,
        ImageFormat::Qoi,
        ImageFormat::Tiff,
        ImageFormat::WebP,
    ];

    fn encode(format: ImageFormat) -> Vec<u8> {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, PIXEL))
            .write_to(&mut encoded, format)
            .unwrap_or_else(|e| panic!("BUG: encoding {format:?} for the fixture failed: {e}"));
        encoded.into_inner()
    }

    #[test]
    fn decode_image_limited_round_trips_every_lossless_format() {
        for &format in LOSSLESS_FORMATS {
            let rgba = decode_image_rgba_limited(&encode(format))
                .unwrap_or_else(|e| panic!("BUG: {format:?} should decode within budget: {e}"));

            assert_eq!(
                (rgba.width(), rgba.height()),
                (2, 2),
                "{format:?} dimensions"
            );
            assert!(
                rgba.pixels().all(|p| *p == PIXEL),
                "{format:?} did not preserve pixels losslessly"
            );
        }
    }

    #[test]
    fn decode_image_limited_accepts_lossy_jpeg() {
        let rgba = decode_image_rgba_limited(&encode(ImageFormat::Jpeg))
            .expect("BUG: JPEG should decode within budget");

        assert_eq!((rgba.width(), rgba.height()), (2, 2));
    }

    /// `host_decode_image` probes or decodes depending on a null output
    /// pointer, so WebP has to survive both branches.
    #[test]
    fn webp_probes_and_decodes_through_host_decode_image() {
        let encoded = encode(ImageFormat::WebP);

        assert_eq!(
            probe_image_dimensions(&encoded).expect("BUG: WebP dimensions should probe"),
            (2, 2)
        );
        assert!(decode_image_rgba_limited(&encoded).is_ok());
    }

    /// HDR decodes to `Rgb32F`, so the binding limit is `Limits::reserve`
    /// rather than `MAX_DECODE_IMAGE_PIXELS`.
    #[test]
    fn decode_image_limited_rejects_hdr_over_alloc_budget() {
        let img = ImageBuffer::from_pixel(2048, 2048, image::Rgb([0.5_f32, 0.25, 0.125]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb32F(img)
            .write_to(&mut encoded, ImageFormat::Hdr)
            .expect("BUG: test HDR encoding should succeed");

        let encoded = encoded.into_inner();

        // Probe first, so the rejection below is attributable to the budget
        // rather than to HDR simply not being recognised.
        assert_eq!(
            probe_image_dimensions(&encoded).expect("BUG: HDR dimensions should probe"),
            (2048, 2048)
        );
        assert!(decode_image_rgba_limited(&encoded).is_err());
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Asset-loading imports for icons, bitmaps, meshes, and image decoding.

#![expect(clippy::cast_possible_truncation)]

use anyhow::{Result, bail};
use bmc_render::{MAX_DECODE_IMAGE_ALLOC_BYTES, MAX_DECODE_IMAGE_PIXELS};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{BitmapId, MeshId, SvgId};
use wasmi::{Caller, Extern, Linker};

use crate::host_api::HostState;

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
                renderer
                    .register_svg(&tag, &data)
                    .map_or(0, SvgId::to_wire)
                    .into()
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
                    .map_or(0, BitmapId::to_wire)
                    .into()
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
                    .map_or(0, BitmapId::to_wire)
                    .into()
            })
        },
    )?;
    Ok(())
}

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
         max_h: u32|
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
                    .register_bitmap_fit(&tag, &data, max_w, max_h)
                    .map_or(0, BitmapId::to_wire)
                    .into()
            })
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
                    .map_or(0, MeshId::to_wire)
                    .into()
            })?;

            #[cfg(feature = "profiling")]
            log_host_register_mesh(id, data_len, &probe);

            Ok(id)
        },
    )?;
    Ok(())
}

fn register_bitmap_sample_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_bitmap_sample",
        |mut caller: Caller<'_, HostState>,
         bitmap_id: u32,
         x: u32,
         y: u32,
         w: u32,
         h: u32|
         -> Result<u32, wasmi::Error> {
            let Some(bitmap_id) = BitmapId::from_wire(bitmap_id as u16) else {
                return Ok(0);
            };
            super::super::with_renderer(&mut caller, |renderer| {
                renderer
                    .bitmap_sample(bitmap_id, x, y, w, h)
                    .map_or(0, Color::to_u32)
            })
        },
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
                    Ok((w, h)) => (i64::from(w) << 32) | i64::from(h),
                    Err(e) => {
                        tracing::error!("host_decode_image probe: {e}");
                        -1
                    }
                };
            }

            let rgba = match decode_image_rgba_limited(&image_data) {
                Ok(rgba) => rgba,
                Err(e) => {
                    tracing::error!("host_decode_image: {e}");
                    return -1;
                }
            };
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
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
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

    use super::{MAX_DECODE_IMAGE_PIXELS, decode_image_rgba_limited, rgba_byte_len_limited};

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
}

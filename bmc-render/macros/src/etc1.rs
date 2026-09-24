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

//! ETC1 compression for mesh textures.
//!
//! A port of the ETC1 path of Intel's ISPC Texture Compressor
//! (`ispc_texcomp/kernel.ispc`, <https://github.com/GameTechDev/ISPCTextureCompressor>),
//! MIT-licensed — the notice is in `LICENSE-ISPC-TEXCOMP` beside this crate's manifest.
//!
//! One change is deliberate.
//! The kernel leaves a group's mean color unset when no pixel falls in it
//! (`colors[q][7..10]`, kernel.ispc:3565), then multiplies it by the group's zero count.
//! Stack garbage that is NaN or too large to square turns that product into NaN:
//! no candidate wins, and `etc_pack` is handed table -1,
//! which trips its `v<pow2(bits)` assertion and aborts the process.
//! Here those means start at zero, as they would on a fresh stack.
//!
//! [`min`] and [`clamp`] keep the kernel's select semantics, where NaN yields the bound:
//! a center fitted to an empty group is 0 / 0, and the kernel's choice depends on it clamping.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division,
    reason = "the kernel's quantization casts and 4×4 block arithmetic, ported as they are"
)]

/// Groupings of a half-block that get a full fit:
/// the kernel's `fastSkipTreshold` at its slow settings.
const CANDIDATES: usize = 6;

/// ETC1's intensity modifier tables, darkest offset first.
const MODIFIERS: [[i32; 4]; 8] = [
    [-8, -2, 2, 8],
    [-17, -5, 5, 17],
    [-29, -9, 9, 29],
    [-42, -13, 13, 42],
    [-60, -18, 18, 60],
    [-80, -24, 24, 80],
    [-106, -33, 33, 106],
    [-183, -47, 47, 183],
];

/// ETC1's pixel index for each luma group, darkest group first.
const PIXEL_INDEX: [u32; 4] = [3, 2, 0, 1];

const BLOCK_BYTES: usize = 8;

/// Compress an RGBA8 image into ETC1, one 8-byte block per 4×4 pixels, row by row.
///
/// Alpha is dropped.
pub fn compress(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(
        rgba.len(),
        width * height * 4,
        "BUG: an RGBA8 image carries four bytes per pixel"
    );
    assert!(
        width >= 4 && height >= 4 && width.is_multiple_of(4) && height.is_multiple_of(4),
        "BUG: ETC1 packs whole 4×4 blocks, and mesh.rs admits no {width}x{height} texture"
    );
    let row_bytes = width / 4 * BLOCK_BYTES;
    let block_rows = height / 4;
    let mut out = vec![0_u8; block_rows * row_bytes];

    // A 512² texture takes over a second on one core even optimized.
    let workers = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let rows_per_worker = block_rows.div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in out.chunks_mut(rows_per_worker * row_bytes).enumerate() {
            scope.spawn(move || {
                for (row_in_chunk, row) in chunk.chunks_exact_mut(row_bytes).enumerate() {
                    let block_y = chunk_index * rows_per_worker + row_in_chunk;
                    compress_row(rgba, width, block_y, row);
                }
            });
        }
    });
    out
}

fn compress_row(rgba: &[u8], width: usize, block_y: usize, row: &mut [u8]) {
    for (block_x, dst) in row.chunks_exact_mut(BLOCK_BYTES).enumerate() {
        let [header, selectors] = compress_block(&load_block(rgba, width, block_x, block_y));
        dst[..4].copy_from_slice(&header.to_le_bytes());
        dst[4..].copy_from_slice(&selectors.to_le_bytes());
    }
}

/// One 4×4 block as red, green and blue planes of 16 values each.
fn load_block(rgba: &[u8], width: usize, block_x: usize, block_y: usize) -> [f32; 48] {
    let mut block = [0.0; 48];
    for y in 0..4 {
        for x in 0..4 {
            let pixel = ((block_y * 4 + y) * width + block_x * 4 + x) * 4;
            for channel in 0..3 {
                block[16 * channel + y * 4 + x] = f32::from(rgba[pixel + channel]);
            }
        }
    }
    block
}

/// A fitted half-block: which modifier table, which base color,
/// and a pixel index per pixel.
struct Half {
    pixel_bits: u32,
    table: usize,
    base: [i32; 3],
    err: f32,
}

fn compress_block(block: &[f32; 48]) -> [u32; 2] {
    let mut transposed = [0.0; 48];
    for y in 0..4 {
        for x in 0..4 {
            for channel in 0..3 {
                transposed[16 * channel + x * 4 + y] = block[16 * channel + y * 4 + x];
            }
        }
    }

    let mut best_err = f32::INFINITY;
    let mut best = [0; 2];
    for flip in [false, true] {
        let pixels = if flip { block } else { &transposed };
        for diff in [true, false] {
            let first = compress_half(pixels, 0, diff, None);
            let second = compress_half(pixels, 8, diff, Some(first.base));
            let err = first.err + second.err;
            if err < best_err {
                best_err = err;
                best = pack(&first, &second, diff, flip);
            }
        }
    }
    best
}

/// Fit the eight pixels at `offset` in each plane of `pixels`.
///
/// Every split of the pixels, ranked by luma, into four ordered groups
/// is scored on luma alone; the best [`CANDIDATES`] are then fitted in color.
/// `prev` is the other half's base, which a differential base must stay near.
fn compress_half(pixels: &[f32; 48], offset: usize, diff: bool, prev: Option<[i32; 3]>) -> Half {
    let px = |k: usize, channel: usize| pixels[offset + 16 * channel + k];

    let mut by_luma: [i32; 8] = std::array::from_fn(|k| {
        let luma = px(k, 0) + px(k, 1) + px(k, 2);
        ((luma as i32) << 4) + k as i32
    });
    partial_sort(&mut by_luma, 8);
    let luma: [f32; 8] = std::array::from_fn(|rank| (by_luma[rank] >> 4) as f32 / 3.0);
    let mut rank_of: [i32; 8] =
        std::array::from_fn(|rank| ((by_luma[rank] & 0xF) << 4) + rank as i32);
    partial_sort(&mut rank_of, 8);

    let mut splits = [0_i32; 165];
    let mut split = 0;
    for level1 in 0..=8 {
        for level2 in level1..=8 {
            for level3 in level2..=8 {
                let mut sum = [0.0_f32; 4];
                let mut sum_sq = [0.0_f32; 4];
                let mut count = [0.0_f32; 4];
                let mut inv_count = [0.0_f32; 4];
                for (rank, &y) in luma.iter().enumerate() {
                    let group = group_of(rank, level1, level2, level3);
                    sum[group] += y;
                    sum_sq[group] += sq(y);
                    count[group] += 1.0;
                }
                for group in 0..4 {
                    if count[group] > 0.0 {
                        inv_count[group] = 1.0 / count[group];
                    }
                }

                let mut base_err = 0.0;
                for group in 0..4 {
                    base_err += sum_sq[group] - sq(sum[group]) * inv_count[group];
                }

                let mut luma_err = sq(256.0) * 8.0;
                for table in &MODIFIERS {
                    let mut center = 0.0;
                    for group in 0..4 {
                        center += sum[group] - table[group] as f32 * count[group];
                    }
                    center /= 8.0;

                    let mut err = base_err;
                    for group in 0..4 {
                        err += sq(center + table[group] as f32 - sum[group] * inv_count[group])
                            * count[group];
                    }
                    luma_err = min(luma_err, err);
                }

                debug_assert!(
                    luma_err < 524_288.0,
                    "BUG: the mildest table's error stays under 2^19, clear of the shift's sign bit"
                );
                let levels = ((level1 * 16 + level2) * 16 + level3) as i32;
                splits[split] = ((luma_err as i32) << 12) + levels;
                split += 1;
            }
        }
    }
    partial_sort(&mut splits, CANDIDATES);

    let mut best_err = sq(255.0) * 3.0 * 8.0;
    let mut best = None;
    for &candidate in &splits[..CANDIDATES] {
        let levels = candidate & 0xFFF;
        let level1 = ((levels >> 8) & 0xF) as usize;
        let level2 = ((levels >> 4) & 0xF) as usize;
        let level3 = (levels & 0xF) as usize;

        // Per group: channel sums, pixel count, channel sums of squares, channel means.
        let mut colors = [[0.0_f32; 10]; 4];
        let mut pixel_bits = 0_u32;
        for (k, &ranked) in rank_of.iter().enumerate() {
            let group = group_of((ranked & 0xF) as usize, level1, level2, level3);
            let index = PIXEL_INDEX[group];
            let (x, y) = (k & 3, k >> 2);
            pixel_bits |= (index & 1) << (y + x * 4);
            pixel_bits |= (index >> 1) << (16 + y + x * 4);

            colors[group][3] += 1.0;
            for channel in 0..3 {
                let value = px(k, channel);
                colors[group][channel] += value;
                colors[group][4 + channel] += sq(value);
            }
        }

        let mut base_err = 0.0;
        for stats in &mut colors {
            if stats[3] > 0.0 {
                for channel in 0..3 {
                    stats[7 + channel] = stats[channel] / stats[3];
                    base_err += stats[4 + channel] - sq(stats[7 + channel]) * stats[3];
                }
            }
        }

        for (table_index, table) in MODIFIERS.iter().enumerate() {
            let mut center: [f32; 3] =
                std::array::from_fn(|channel| optimize_center(&colors, channel, table));
            let base = quantize_base(&mut center, diff, prev);

            let mut err = base_err;
            for (stats, &modifier) in colors.iter().zip(table) {
                for channel in 0..3 {
                    err +=
                        sq(clamp(center[channel] + modifier as f32, 0.0, 255.0)
                            - stats[7 + channel])
                            * stats[3];
                }
            }

            if err < best_err {
                best_err = err;
                best = Some((pixel_bits, table_index, base));
            }
        }
    }

    let (pixel_bits, table, base) =
        best.expect("BUG: some fit always beats every channel of every pixel being 255 off");
    Half {
        pixel_bits,
        table,
        base,
        err: best_err,
    }
}

/// Which luma group the pixel ranked `rank` falls in, split at the three levels.
fn group_of(rank: usize, level1: usize, level2: usize, level3: usize) -> usize {
    let mut group = 0;
    if rank >= level1 {
        group = 1;
    }
    if rank >= level2 {
        group = 2;
    }
    if rank >= level3 {
        group = 3;
    }
    group
}

/// The base value for one channel that best fits its groups under `table`:
/// the mean, or a mean that lets the outermost groups clamp.
fn optimize_center(colors: &[[f32; 10]; 4], channel: usize, table: &[i32; 4]) -> f32 {
    let group_err = |center: f32| {
        let mut err = 0.0;
        for (stats, &modifier) in colors.iter().zip(table) {
            err += sq(clamp(center + modifier as f32, 0.0, 255.0) - stats[7 + channel]) * stats[3];
        }
        err
    };

    let mut best_center = 0.0;
    for (stats, &modifier) in colors.iter().zip(table) {
        best_center += (stats[7 + channel] - modifier as f32) * stats[3];
    }
    best_center /= 8.0;
    let mut best_err = group_err(best_center);

    for branch in 0..4 {
        let mut center = 0.0;
        let mut weight = 0.0;
        for (group, (stats, &modifier)) in colors.iter().zip(table).enumerate() {
            if branch <= 1 && group <= branch || branch >= 2 && group >= branch {
                continue;
            }
            center += (stats[7 + channel] - modifier as f32) * stats[3];
            weight += stats[3];
        }
        center /= weight;

        let err = group_err(center);
        if err < best_err {
            best_err = err;
            best_center = center;
        }
    }
    best_center
}

/// Quantize a half's base color, replacing `center` with what it decodes to.
///
/// A differential base keeps 5 bits a channel and must lie within -4..=3
/// of `prev`, the other half's base; an individual one keeps 4 bits.
fn quantize_base(center: &mut [f32; 3], diff: bool, prev: Option<[i32; 3]>) -> [i32; 3] {
    let mut base = [0; 3];
    for channel in 0..3 {
        if diff {
            let mut q = quantize(center[channel], 31);
            if let Some(prev) = prev {
                q = q.clamp(prev[channel] - 4, prev[channel] + 3);
            }
            center[channel] = ((q << 3) | (q >> 2)) as f32;
            base[channel] = q;
        } else {
            let q = quantize(center[channel], 15);
            center[channel] = ((q << 4) | q) as f32;
            base[channel] = q;
        }
    }
    base
}

fn quantize(value: f32, max: i32) -> i32 {
    clamp((value / 255.0) * max as f32 + 0.5, 0.0, max as f32) as i32
}

/// The two halves as ETC1's 64 bits, in the order `compress` stores them:
/// bases, tables and flags, then the pixel indices.
fn pack(first: &Half, second: &Half, diff: bool, flip: bool) -> [u32; 2] {
    let mut header = 0_u32;
    let mut pos = 0;
    let mut put = |bits: u32, value: u32| {
        assert!(
            value < 1 << bits,
            "BUG: {value} overflows its {bits}-bit field"
        );
        header |= value << pos;
        pos += bits;
    };

    for channel in 0..3 {
        let (base0, base1) = (first.base[channel], second.base[channel]);
        if diff {
            put(3, ((base1 - base0) & 7) as u32);
            put(5, base0 as u32);
        } else {
            put(4, base1 as u32);
            put(4, base0 as u32);
        }
    }
    put(1, u32::from(flip));
    put(1, u32::from(diff));
    put(3, second.table as u32);
    put(3, first.table as u32);

    let by_half = (second.pixel_bits << 2) | first.pixel_bits;
    let mut selectors = by_half;
    if !flip {
        selectors = 0;
        for plane in 0..2 {
            for y in 0..4 {
                for x in 0..4 {
                    let bit = (by_half >> (plane * 16 + x * 4 + y)) & 1;
                    selectors |= bit << (plane * 16 + y * 4 + x);
                }
            }
        }
    }
    [header, selectors.swap_bytes()]
}

fn sq(v: f32) -> f32 {
    v * v
}

fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn clamp(v: f32, low: f32, high: f32) -> f32 {
    let raised = if v > low { v } else { low };
    if raised < high { raised } else { high }
}

/// Selection-sort the `count` smallest values to the front, as the kernel does:
/// the first of equal values stays first.
fn partial_sort(list: &mut [i32], count: usize) {
    for k in 0..count {
        let mut best_index = k;
        let mut best_value = list[k];
        for (i, &value) in list.iter().enumerate().skip(k + 1) {
            if best_value > value {
                best_value = value;
                best_index = i;
            }
        }
        list[best_index] = list[k];
        list[k] = best_value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode one block to 16 RGB pixels, row-major, as the GPU would.
    fn decode(block: &[u8]) -> [[i32; 3]; 16] {
        let diff = block[3] & 2 != 0;
        let flip = block[3] & 1 != 0;
        let tables = [usize::from(block[3] >> 5), usize::from((block[3] >> 2) & 7)];
        let mut bases = [[0; 3]; 2];
        for channel in 0..3 {
            let byte = i32::from(block[channel]);
            if diff {
                let base = byte >> 3;
                let delta = ((byte & 7) << 29) >> 29;
                let extend = |v: i32| (v << 3) | (v >> 2);
                bases[0][channel] = extend(base);
                bases[1][channel] = extend(base + delta);
            } else {
                let extend = |v: i32| (v << 4) | v;
                bases[0][channel] = extend(byte >> 4);
                bases[1][channel] = extend(byte & 15);
            }
        }

        let selectors = u32::from_be_bytes([block[4], block[5], block[6], block[7]]);
        let mut pixels = [[0; 3]; 16];
        for y in 0..4 {
            for x in 0..4 {
                let bit = x * 4 + y;
                let index = (((selectors >> (bit + 16)) & 1) << 1) | ((selectors >> bit) & 1);
                let half = usize::from(if flip { y >= 2 } else { x >= 2 });
                // Per the spec: small positive, large positive, small negative, large negative.
                let [large_neg, small_neg, small_pos, large_pos] = MODIFIERS[tables[half]];
                let modifier = [small_pos, large_pos, small_neg, large_neg][index as usize];
                for channel in 0..3 {
                    pixels[y * 4 + x][channel] = (bases[half][channel] + modifier).clamp(0, 255);
                }
            }
        }
        pixels
    }

    /// Squared error of the decoded image against the RGBA source.
    fn decoded_error(rgba: &[u8], width: usize, blocks: &[u8]) -> i64 {
        let mut err = 0;
        for (n, block) in blocks.chunks_exact(BLOCK_BYTES).enumerate() {
            let (block_x, block_y) = (n % (width / 4), n / (width / 4));
            for (i, pixel) in decode(block).iter().enumerate() {
                let source = ((block_y * 4 + i / 4) * width + block_x * 4 + i % 4) * 4;
                for (channel, &value) in pixel.iter().enumerate() {
                    err += i64::from(value - i32::from(rgba[source + channel])).pow(2);
                }
            }
        }
        err
    }

    #[test]
    fn a_flat_color_comes_back_within_a_quantization_step() {
        let color = [200, 120, 40];
        let rgba: Vec<u8> = std::iter::repeat_n([color[0], color[1], color[2], 255], 16)
            .flatten()
            .collect();

        for pixel in decode(&compress(&rgba, 4, 4)) {
            for (&decoded, &source) in pixel.iter().zip(&color) {
                // Half a 5-bit step, plus the smallest modifier.
                assert!(
                    (decoded - i32::from(source)).abs() <= 6,
                    "{pixel:?} strays from {color:?}"
                );
            }
        }
    }

    /// Intel's kernel, run on a clean stack, decoded to this error
    /// on the gallery's Suzanne texture.
    const INTEL_SUZANNE_ERROR: i64 = 3_042_377;

    #[test]
    fn the_gallery_suzanne_compresses_as_well_as_intels_kernel() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/suzanne.glb");
        let (_, _, images) = gltf::import(path).expect("BUG: the gallery's Suzanne must import");
        let image = &images[0];
        assert_eq!(
            image.format,
            gltf::image::Format::R8G8B8,
            "BUG: Suzanne's texture changed format"
        );
        let rgba: Vec<u8> = image
            .pixels
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect();
        let width = image.width as usize;

        let err = decoded_error(&rgba, width, &compress(&rgba, width, image.height as usize));

        assert!(
            err <= INTEL_SUZANNE_ERROR + INTEL_SUZANNE_ERROR / 1_000,
            "decoded error {err} against the kernel's {INTEL_SUZANNE_ERROR}"
        );
    }

    /// Two pixels wide fills no block, and a buffer sized some other way
    /// than the decoder's fails only when the mesh loads.
    #[test]
    #[should_panic(expected = "whole 4×4 blocks")]
    fn a_side_short_of_a_block_is_refused() {
        compress(&[255; 2 * 8 * 4], 2, 8);
    }
}

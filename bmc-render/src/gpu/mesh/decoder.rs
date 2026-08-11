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

//! Binary mesh format parser + GPU upload.
//!
//! Owns the validate-then-trust contract for guest-supplied bytes:
//! `validate_mesh_header` rejects every malformed offset/size combination
//! up front, then `parse_and_upload` walks the validated regions to push
//! VBO/IBO/texture/normal-map handles into GL. RAII guards from
//! `super::raii` cover partial-failure leaks across the `?` chain.

#![expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

use anyhow::{Result, bail};
use glow::HasContext;

use bmc_wasm_protocol::mesh::{
    FLAG_HAS_NORMAL_MAP, FLAG_HAS_TANGENTS, FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE,
    MAX_TEXTURE_SIZE, MAX_TRIANGLES, MAX_VERTICES, MESH_MAGIC, TextureFormat,
};

use super::UploadedMesh;
use super::atlas::AABB_SIZE;
use super::raii::{BufferGuard, TextureGuard};

/// Texture-format bytes the renderer can actually decode. `Etc2Rgb` is in
/// the protocol enum but no encoder writes it and the upload path has no
/// branch for it — so it is rejected up-front instead of silently falling
/// through to the RGBA8 path.
const VALID_TEX_FORMATS: &[u8] = &[
    TextureFormat::Rgba8 as u8,
    TextureFormat::Etc1 as u8,
    TextureFormat::Msdf as u8,
];

/// Header fields that have been bounds-checked against `data.len()` and the
/// protocol limits. After construction every offset/length combination is
/// known to fit inside `data`, so subsequent reads can use unchecked indexing
/// without risk of panicking on attacker-controlled input.
#[derive(Debug)]
struct ValidatedHeader {
    vertex_count: usize,
    index_count: usize,
    vertex_offset: usize,
    index_offset: usize,
    texture_offset: usize,
    tex_width: u32,
    tex_height: u32,
    tex_format: u8,
    nmap_offset: usize,
    nmap_width: u32,
    nmap_height: u32,
    flags: u8,
    quantized_vertex_size: usize,
    floats_per_vertex: usize,
}

impl ValidatedHeader {
    fn has_texture(&self) -> bool {
        self.flags & FLAG_HAS_TEXTURE != 0
    }
    fn has_uvs(&self) -> bool {
        self.flags & FLAG_HAS_UVS != 0
    }
    fn has_tangents(&self) -> bool {
        self.flags & FLAG_HAS_TANGENTS != 0
    }
    fn has_normal_map(&self) -> bool {
        self.flags & FLAG_HAS_NORMAL_MAP != 0
    }
}

/// Verify the dimension caps and flag-derived vertex layout fields. Returns
/// `(floats_per_vertex, quantized_vertex_size)`.
fn validate_dimensions(
    vertex_count: u32,
    index_count: u32,
    tex_width: u32,
    tex_height: u32,
    nmap_width: u32,
    nmap_height: u32,
    flags: u8,
) -> Result<(usize, usize)> {
    if vertex_count > MAX_VERTICES {
        bail!("vertex_count {vertex_count} exceeds MAX_VERTICES {MAX_VERTICES}");
    }
    let max_indices = MAX_TRIANGLES.saturating_mul(3);
    if index_count > max_indices {
        bail!("index_count {index_count} exceeds 3 * MAX_TRIANGLES ({max_indices})");
    }
    if !index_count.is_multiple_of(3) {
        bail!("index_count {index_count} is not a multiple of 3");
    }
    if tex_width > MAX_TEXTURE_SIZE || tex_height > MAX_TEXTURE_SIZE {
        bail!("texture {tex_width}x{tex_height} exceeds MAX_TEXTURE_SIZE {MAX_TEXTURE_SIZE}");
    }
    if nmap_width > MAX_TEXTURE_SIZE || nmap_height > MAX_TEXTURE_SIZE {
        bail!("normal map {nmap_width}x{nmap_height} exceeds MAX_TEXTURE_SIZE {MAX_TEXTURE_SIZE}");
    }
    let has_uvs = flags & FLAG_HAS_UVS != 0;
    let has_tangents = flags & FLAG_HAS_TANGENTS != 0;
    Ok(match (has_uvs, has_tangents) {
        (false, _) => (6, 10),
        (true, false) => (8, 14),
        (true, true) => (12, 22),
    })
}

/// Texture or normal-map region descriptor used during header validation.
#[derive(Clone, Copy)]
struct ImageRegion {
    offset: usize,
    width: u32,
    height: u32,
}

/// Validate that the (optional) texture and normal-map regions fit in `data`.
fn check_image_regions(
    data_len: usize,
    flags: u8,
    tex_format: u8,
    texture: ImageRegion,
    normal_map: ImageRegion,
) -> Result<()> {
    let is_etc1 = tex_format == TextureFormat::Etc1 as u8;
    let image_size = |w: u32, h: u32| -> Result<usize> {
        if is_etc1 {
            Ok(etc1_data_size(w, h))
        } else {
            rgba8_byte_len(w, h)
        }
    };
    if flags & FLAG_HAS_TEXTURE != 0 && texture.width > 0 && texture.height > 0 {
        let size = image_size(texture.width, texture.height)?;
        check_region(data_len, texture.offset, Some(size), "texture region")?;
    }
    if flags & FLAG_HAS_NORMAL_MAP != 0 && normal_map.width > 0 && normal_map.height > 0 {
        let size = image_size(normal_map.width, normal_map.height)?;
        check_region(data_len, normal_map.offset, Some(size), "normal map region")?;
    }
    Ok(())
}

/// Parse and bounds-check the mesh header against `data` and the protocol
/// limits. The `host_register_mesh` import is reachable from untrusted WASM
/// guests, so every offset/size combination must be validated before any
/// indexing happens — the renderer must not panic on malformed input.
fn validate_mesh_header(data: &[u8]) -> Result<ValidatedHeader> {
    if data.len() < HEADER_SIZE + AABB_SIZE {
        bail!(
            "mesh data too small: {} bytes (need at least {})",
            data.len(),
            HEADER_SIZE + AABB_SIZE,
        );
    }
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != MESH_MAGIC {
        bail!("invalid mesh magic: 0x{magic:08X}");
    }

    let vertex_count = read_u32(data, 4);
    let index_count = read_u32(data, 8);
    let vertex_offset = read_u32(data, 12) as usize;
    let index_offset = read_u32(data, 16) as usize;
    let texture_offset = read_u32(data, 20) as usize;
    let tex_width = u32::from(read_u16(data, 24));
    let tex_height = u32::from(read_u16(data, 26));
    let tex_format = data[28];
    if !VALID_TEX_FORMATS.contains(&tex_format) {
        bail!(
            "unknown or unsupported texture format: {tex_format} \
             (expected Rgba8={}, Etc1={}, or Msdf={})",
            TextureFormat::Rgba8 as u8,
            TextureFormat::Etc1 as u8,
            TextureFormat::Msdf as u8,
        );
    }
    let flags = data[29];
    let nmap_offset = read_u32(data, 30) as usize;
    let nmap_width = u32::from(read_u16(data, 34));
    let nmap_height = u32::from(read_u16(data, 36));

    let (floats_per_vertex, quantized_vertex_size) = validate_dimensions(
        vertex_count,
        index_count,
        tex_width,
        tex_height,
        nmap_width,
        nmap_height,
        flags,
    )?;

    let vertex_count = vertex_count as usize;
    let index_count = index_count as usize;

    check_region(
        data.len(),
        vertex_offset,
        vertex_count.checked_mul(quantized_vertex_size),
        "vertex region",
    )?;
    check_region(
        data.len(),
        index_offset,
        index_count.checked_mul(2),
        "index region",
    )?;
    check_image_regions(
        data.len(),
        flags,
        tex_format,
        ImageRegion {
            offset: texture_offset,
            width: tex_width,
            height: tex_height,
        },
        ImageRegion {
            offset: nmap_offset,
            width: nmap_width,
            height: nmap_height,
        },
    )?;

    Ok(ValidatedHeader {
        vertex_count,
        index_count,
        vertex_offset,
        index_offset,
        texture_offset,
        tex_width,
        tex_height,
        tex_format,
        nmap_offset,
        nmap_width,
        nmap_height,
        flags,
        quantized_vertex_size,
        floats_per_vertex,
    })
}

/// Verify that `[offset .. offset + size]` fits inside `data_len`. Both the
/// `size` computation upstream and the `offset + size` sum are checked for
/// overflow; either failure yields an `Err` rather than a wrap or panic.
fn check_region(data_len: usize, offset: usize, size: Option<usize>, label: &str) -> Result<()> {
    let Some(size) = size else {
        bail!("{label} size overflow");
    };
    let Some(end) = offset.checked_add(size) else {
        bail!("{label} end overflow (offset={offset} + size={size})");
    };
    if end > data_len {
        bail!("{label} extends past data ({offset}..{end} > {data_len})",);
    }
    Ok(())
}

/// RGBA8 byte length with overflow-checked multiplication. `width` and
/// `height` are already capped at `MAX_TEXTURE_SIZE` by the caller, but the
/// arithmetic remains overflow-safe to keep the helper reusable.
fn rgba8_byte_len(width: u32, height: u32) -> Result<usize> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|p| p.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("RGBA8 byte length overflow"))?;
    usize::try_from(bytes).map_err(Into::into)
}

/// Parse the optimized binary format and upload VBO/IBO/texture to GL.
#[expect(clippy::too_many_lines)]
pub(super) fn parse_and_upload(gl: &glow::Context, data: &[u8]) -> Result<UploadedMesh> {
    let header = validate_mesh_header(data)?;
    let has_texture = header.has_texture();
    let has_uvs = header.has_uvs();
    let has_tangents = header.has_tangents();
    let has_normal_map = header.has_normal_map();
    let ValidatedHeader {
        vertex_count,
        index_count,
        vertex_offset,
        index_offset,
        texture_offset,
        tex_width,
        tex_height,
        tex_format,
        nmap_offset,
        nmap_width,
        nmap_height,
        quantized_vertex_size,
        floats_per_vertex,
        ..
    } = header;

    // Read AABB (6 floats after header)
    let aabb_offset = HEADER_SIZE;
    let aabb_min = [
        read_f32(data, aabb_offset),
        read_f32(data, aabb_offset + 4),
        read_f32(data, aabb_offset + 8),
    ];
    let aabb_max = [
        read_f32(data, aabb_offset + 12),
        read_f32(data, aabb_offset + 16),
        read_f32(data, aabb_offset + 20),
    ];

    // Dequantize vertices into float VBO. `vertex_count` is bounded by
    // `MAX_VERTICES` and `floats_per_vertex` ≤ 12, so the capacity below
    // cannot overflow `usize` on any supported target.
    let mut vertex_floats = Vec::with_capacity(vertex_count * floats_per_vertex);

    for i in 0..vertex_count {
        let base = vertex_offset + i * quantized_vertex_size;

        // Dequantize position from i16
        let qx = read_i16(data, base);
        let qy = read_i16(data, base + 2);
        let qz = read_i16(data, base + 4);
        vertex_floats.push(dequantize_position(qx, aabb_min[0], aabb_max[0]));
        vertex_floats.push(dequantize_position(qy, aabb_min[1], aabb_max[1]));
        vertex_floats.push(dequantize_position(qz, aabb_min[2], aabb_max[2]));

        // Unpack normal from 10/10/10/2
        let packed_normal = read_u32(data, base + 6);
        let (nx, ny, nz) = unpack_normal_10_10_10_2(packed_normal);
        vertex_floats.push(nx);
        vertex_floats.push(ny);
        vertex_floats.push(nz);

        if has_uvs {
            let qu = read_u16(data, base + 10);
            let qv = read_u16(data, base + 12);
            vertex_floats.push(f32::from(qu) / 65_535.0);
            vertex_floats.push(f32::from(qv) / 65_535.0);

            if has_tangents {
                // Dequantize tangent xyzw from i16 (range -32767..32767 → -1..1)
                let dq = |off: usize| f32::from(read_i16(data, off)) / 32_767.0;
                vertex_floats.push(dq(base + 14));
                vertex_floats.push(dq(base + 16));
                vertex_floats.push(dq(base + 18));
                vertex_floats.push(dq(base + 20));
            }
        }
    }

    // Upload VBO. The guard deletes the buffer if any later step bails so
    // partial-failure paths don't leak GL handles.
    let vbo_guard = unsafe {
        let vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        let guard = BufferGuard::new(gl, vbo);
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let bytes: &[u8] = std::slice::from_raw_parts(
            vertex_floats.as_ptr().cast::<u8>(),
            vertex_floats.len() * 4,
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        guard
    };

    // Upload IBO (indices are u16, already in the right format)
    let ibo_guard = unsafe {
        let ibo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        let guard = BufferGuard::new(gl, ibo);
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
        let index_bytes = &data[index_offset..index_offset + index_count * 2];
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
        guard
    };

    let is_etc1 = tex_format == TextureFormat::Etc1 as u8;
    let is_msdf = tex_format == TextureFormat::Msdf as u8;

    // Body and label colors (MSDF-only; left zero for ETC1 meshes).
    let body_color = read_rgba_u8(data, 40);
    let label_color = read_rgba_u8(data, 44);

    // Upload texture (optional). MSDF and Rgba8 share the uncompressed
    // RGBA8 upload path; only ETC1 takes the compressed path. Bounds were
    // already checked in `validate_mesh_header`, so the slice cannot panic.
    let texture_guard = if has_texture && tex_width > 0 && tex_height > 0 {
        let tex_size = if is_etc1 {
            etc1_data_size(tex_width, tex_height)
        } else {
            rgba8_byte_len(tex_width, tex_height)
                .expect("BUG: rgba8 size validated in validate_mesh_header")
        };
        let tex_data = &data[texture_offset..texture_offset + tex_size];
        Some(TextureGuard::new(
            gl,
            upload_texture(gl, tex_width, tex_height, tex_data, is_etc1)?,
        ))
    } else {
        None
    };

    // Upload normal map (optional, same format as albedo)
    let normal_map_guard = if has_normal_map && nmap_width > 0 && nmap_height > 0 {
        let nmap_size = if is_etc1 {
            etc1_data_size(nmap_width, nmap_height)
        } else {
            rgba8_byte_len(nmap_width, nmap_height)
                .expect("BUG: rgba8 size validated in validate_mesh_header")
        };
        let nmap_data = &data[nmap_offset..nmap_offset + nmap_size];
        Some(TextureGuard::new(
            gl,
            upload_texture(gl, nmap_width, nmap_height, nmap_data, is_etc1)?,
        ))
    } else {
        None
    };

    let resident_bytes = vertex_floats
        .len()
        .checked_mul(std::mem::size_of::<f32>())
        .and_then(|bytes| bytes.checked_add(index_count.checked_mul(2)?))
        .and_then(|bytes| {
            bytes.checked_add(if has_texture {
                if is_etc1 {
                    etc1_data_size(tex_width, tex_height)
                } else {
                    rgba8_byte_len(tex_width, tex_height)
                        .expect("BUG: rgba8 size validated in validate_mesh_header")
                }
            } else {
                0
            })
        })
        .and_then(|bytes| {
            bytes.checked_add(if has_normal_map {
                if is_etc1 {
                    etc1_data_size(nmap_width, nmap_height)
                } else {
                    rgba8_byte_len(nmap_width, nmap_height)
                        .expect("BUG: rgba8 size validated in validate_mesh_header")
                }
            } else {
                0
            })
        })
        .and_then(|bytes| u64::try_from(bytes).ok())
        .expect("BUG: validated mesh resident byte count must fit u64");

    // All steps succeeded — defuse every guard so the handles survive into
    // the returned `UploadedMesh`.
    let vbo = vbo_guard.defuse();
    let ibo = ibo_guard.defuse();
    let texture = texture_guard.map(TextureGuard::defuse);
    let normal_map = normal_map_guard.map(TextureGuard::defuse);

    #[expect(clippy::integer_division)]
    let triangle_count = index_count / 3;
    tracing::info!(
        "mesh uploaded: {vertex_count} vertices, {triangle_count} triangles, \
         texture={}, normal_map={}",
        texture.is_some(),
        normal_map.is_some()
    );

    Ok(UploadedMesh {
        vbo,
        ibo,
        index_count: index_count as i32,
        texture,
        normal_map,
        resident_bytes,
        has_uvs,
        has_tangents,
        is_msdf,
        body_color,
        label_color,
    })
}

/// Read four consecutive u8 channels from the header and convert to a
/// linear `[f32; 4]` in [0..1] suitable for direct shader-uniform use.
fn read_rgba_u8(data: &[u8], offset: usize) -> [f32; 4] {
    [
        f32::from(data[offset]) / 255.0,
        f32::from(data[offset + 1]) / 255.0,
        f32::from(data[offset + 2]) / 255.0,
        f32::from(data[offset + 3]) / 255.0,
    ]
}

/// GL_ETC1_RGB8_OES (from GL_OES_compressed_ETC1_RGB8_texture).
const GL_ETC1_RGB8_OES: u32 = 0x8D64;

/// Compute ETC1 compressed data size: 8 bytes per 4×4 block.
fn etc1_data_size(width: u32, height: u32) -> usize {
    (width.div_ceil(4) * height.div_ceil(4) * 8) as usize
}

fn upload_texture(
    gl: &glow::Context,
    width: u32,
    height: u32,
    data: &[u8],
    etc1: bool,
) -> Result<glow::Texture> {
    unsafe {
        let texture = gl.create_texture().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        if etc1 {
            gl.compressed_tex_image_2d(
                glow::TEXTURE_2D,
                0,
                GL_ETC1_RGB8_OES as i32,
                width as i32,
                height as i32,
                0,
                data.len() as i32, // texture data ≤ 1MB
                data,
            );
        } else {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(data)),
            );
        }
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(texture)
    }
}

fn dequantize_position(q: i16, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range < 1e-8 {
        return min;
    }
    // Symmetric with `quantize_position`: full i16 range maps to [0, 1]
    // exactly (i16::MIN → 0.0, i16::MAX → 1.0). The asymmetric form
    // (65_534 / ±32_767) would produce sub-pixel drift on the unused
    // `-32_768` slot.
    let t = (f32::from(q) + 32_768.0) / 65_535.0; // 0..1
    min + t * range
}

fn unpack_normal_10_10_10_2(packed: u32) -> (f32, f32, f32) {
    let from_10bit = |bits: u32| -> f32 {
        // Sign-extend 10-bit to i32
        let signed = if bits & 0x200 != 0 {
            (bits | 0xFFFF_FC00) as i32
        } else {
            bits as i32
        };
        signed as f32 / 511.0
    };
    let x = from_10bit(packed & 0x3FF);
    let y = from_10bit((packed >> 10) & 0x3FF);
    let z = from_10bit((packed >> 20) & 0x3FF);
    (x, y, z)
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_i16(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::validate_mesh_header;
    use crate::gpu::mesh::atlas::AABB_SIZE;
    use bmc_wasm_protocol::mesh::{
        FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE, MAX_TEXTURE_SIZE, MAX_TRIANGLES, MAX_VERTICES,
        MESH_MAGIC,
    };

    /// First byte after the 48-byte header + 24-byte AABB record. Used as the
    /// default region offset in fixtures so vertex/index regions sit
    /// immediately after the AABB.
    const BODY_OFFSET: usize = HEADER_SIZE + AABB_SIZE;

    fn write_u32(buf: &mut [u8], range: std::ops::Range<usize>, value: usize) {
        let v = u32::try_from(value).expect("BUG: test fixture exceeds u32 range");
        buf[range].copy_from_slice(&v.to_le_bytes());
    }

    /// Build a minimal valid header with no texture / no normal map and the
    /// requested vertex/index counts/offsets. Caller must ensure `total_len`
    /// is large enough for the requested regions.
    fn header_buffer(
        vertex_count: usize,
        index_count: usize,
        vertex_offset: usize,
        index_offset: usize,
        flags: u8,
        total_len: usize,
    ) -> Vec<u8> {
        let mut buf = vec![0_u8; total_len.max(BODY_OFFSET)];
        buf[0..4].copy_from_slice(&MESH_MAGIC.to_le_bytes());
        write_u32(&mut buf, 4..8, vertex_count);
        write_u32(&mut buf, 8..12, index_count);
        write_u32(&mut buf, 12..16, vertex_offset);
        write_u32(&mut buf, 16..20, index_offset);
        buf[29] = flags;
        buf
    }

    #[test]
    fn rejects_payload_smaller_than_header_plus_aabb() {
        let buf = vec![0_u8; HEADER_SIZE]; // missing AABB
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut buf = vec![0_u8; BODY_OFFSET];
        buf[0..4].copy_from_slice(b"NOPE");
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("magic"), "{err}");
    }

    #[test]
    fn rejects_excessive_vertex_count() {
        let buf = header_buffer(
            MAX_VERTICES as usize + 1,
            0,
            BODY_OFFSET,
            BODY_OFFSET,
            0,
            BODY_OFFSET,
        );
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("MAX_VERTICES"), "{err}");
    }

    #[test]
    fn rejects_excessive_index_count() {
        let buf = header_buffer(
            0,
            MAX_TRIANGLES as usize * 3 + 3,
            BODY_OFFSET,
            BODY_OFFSET,
            0,
            BODY_OFFSET,
        );
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("MAX_TRIANGLES"), "{err}");
    }

    #[test]
    fn rejects_index_count_not_multiple_of_three() {
        let buf = header_buffer(0, 7, BODY_OFFSET, BODY_OFFSET, 0, BODY_OFFSET + 14);
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("multiple of 3"), "{err}");
    }

    #[test]
    fn rejects_oversized_texture_dimension() {
        let mut buf = header_buffer(
            0,
            0,
            BODY_OFFSET,
            BODY_OFFSET,
            FLAG_HAS_TEXTURE | FLAG_HAS_UVS,
            BODY_OFFSET,
        );
        let oversize =
            u16::try_from(MAX_TEXTURE_SIZE + 1).expect("BUG: MAX_TEXTURE_SIZE+1 fits in u16");
        buf[24..26].copy_from_slice(&oversize.to_le_bytes());
        buf[26..28].copy_from_slice(&oversize.to_le_bytes());
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("MAX_TEXTURE_SIZE"), "{err}");
    }

    #[test]
    fn rejects_truncated_index_region() {
        // Claim 6 indices (12 bytes) at offset = HEADER+AABB but make the
        // buffer only large enough to hold the header.
        let buf = header_buffer(0, 6, BODY_OFFSET, BODY_OFFSET, 0, BODY_OFFSET);
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(err.contains("index region"), "{err}");
    }

    #[test]
    fn rejects_offset_overflow() {
        // index_offset = u32::MAX, index_count = 6 → end overflows
        let buf = header_buffer(0, 6, BODY_OFFSET, u32::MAX as usize, 0, BODY_OFFSET);
        let err = validate_mesh_header(&buf)
            .expect_err("BUG: validator must reject this header")
            .to_string();
        assert!(
            err.contains("end overflow") || err.contains("index region"),
            "{err}",
        );
    }

    #[test]
    fn accepts_minimal_empty_mesh() {
        let buf = header_buffer(0, 0, BODY_OFFSET, BODY_OFFSET, 0, BODY_OFFSET);
        let h = validate_mesh_header(&buf)
            .expect("BUG: minimal valid header fixture must satisfy validate_mesh_header");
        assert_eq!(h.vertex_count, 0);
        assert_eq!(h.index_count, 0);
    }
}

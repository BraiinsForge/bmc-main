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

//! Binary mesh format constants for the 3D mesh pipeline.
//!
//! The format is produced by `include_mesh!` at compile time and consumed
//! by the host-side `MeshRenderer` at runtime.

/// Magic bytes: "MDL2" in little-endian (V2 adds MSDF format + body/label
/// colors). V1 ("MDL1") is no longer accepted; meshes are rebuilt with the
/// runtime, so backward compatibility isn't required.
pub const MESH_MAGIC: u32 = u32::from_le_bytes(*b"MDL2");

/// Maximum triangle count per mesh (hardware budget for GC400 at 30fps).
pub const MAX_TRIANGLES: u32 = 5_000;

/// Maximum vertex count (u16 index limit).
pub const MAX_VERTICES: u32 = 65_535;

/// Maximum texture dimension (width or height).
pub const MAX_TEXTURE_SIZE: u32 = 1_024;

/// Binary header size in bytes.
/// Layout:
///   [0..4]   magic: u32
///   [4..8]   vertex_count: u32
///   [8..12]  index_count: u32
///   [12..16] vertex_offset: u32
///   [16..20] index_offset: u32
///   [20..24] texture_offset: u32
///   [24..26] texture_width: u16
///   [26..28] texture_height: u16
///   [28]     texture_format: u8
///   [29]     flags: u8
///   [30..34] normal_map_offset: u32
///   [34..36] normal_map_width: u16
///   [36..38] normal_map_height: u16
///   [38..40] _reserved: [u8; 2]
///   [40..44] body_color: [u8; 4]   (RGBA, used by MSDF format)
///   [44..48] label_color: [u8; 4]  (RGBA, used by MSDF format)
pub const HEADER_SIZE: usize = 48;

/// Texture format tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TextureFormat {
    Rgba8 = 0,
    Etc1 = 1,
    Etc2Rgb = 2,
    /// Multi-channel SDF (RGB888). Sampled as median-of-RGB, smoothstep
    /// thresholded to coverage, lerped between body_color and label_color
    /// from the header. No normal map; geometry is simple-lit.
    Msdf = 3,
}

/// Mesh flags.
pub const FLAG_HAS_TEXTURE: u8 = 0x01;
pub const FLAG_HAS_UVS: u8 = 0x02;
pub const FLAG_HAS_TANGENTS: u8 = 0x04;
pub const FLAG_HAS_NORMAL_MAP: u8 = 0x08;

/// Per-vertex size: 3x i16 pos + 1x u32 packed normal + 2x u16 uv = 12 bytes.
pub const VERTEX_SIZE: usize = 12;

/// Per-vertex size without UVs: 3x i16 pos + 1x u32 packed normal = 10 bytes.
pub const VERTEX_SIZE_NO_UV: usize = 10;

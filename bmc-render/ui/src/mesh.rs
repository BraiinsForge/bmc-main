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

use crate::orientation::Orientation;

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

/// Static 3D mesh descriptor (output of `include_mesh!`).
///
/// WASM builds load the optimized mesh from the widget package; native builds
/// retain embedded bytes for storybook rendering.
///
/// `face_normals` contains per-face normals in glTF Y-up space, ordered by
/// display face number (index 0 = face 1). Extracted from glTF `extras` at
/// compile time. Empty if the model has no face normal metadata.
#[derive(Debug)]
pub struct Mesh<S> {
    pub source: S,
    pub face_normals: &'static [[f32; 3]],
    /// Stable, unique-per-host registration tag (e.g. `"crate::stem"`).
    pub name: &'static str,
    /// Why `include_mesh!` could not pack the mesh,
    /// in a build that defers its errors; `None` otherwise.
    pub error: Option<&'static str>,
}

/// Directional light angles, in degrees.
#[derive(Debug, Clone, Copy)]
pub struct LightAngles {
    pub pitch: f32,
    pub yaw: f32,
}

/// UV-rect highlight tint. `uv_rect` is `(u_min, v_min, u_max, v_max)` in UV
/// space; `color` is sRGB `(r, g, b)` in `[0, 1]`.
#[derive(Debug, Clone, Copy)]
pub struct Highlight {
    pub uv_rect: [f32; 4],
    pub color: [f32; 3],
}

/// Camera, transform, and lighting parameters for rendering a 3D mesh.
#[derive(Debug, Clone, Copy)]
pub struct MeshView {
    /// Vertical field of view in degrees.
    pub fov: f32,
    /// Camera distance from origin.
    pub distance: f32,
    /// Object orientation (unit quaternion).
    pub orientation: Orientation,
    /// Object position offset.
    pub position: [f32; 3],
    /// Uniform scale factor.
    pub scale: f32,
    /// Directional light angles (in degrees). `None` = unlit.
    pub light: Option<LightAngles>,
    /// Ambient light level (0.0–1.0). Controls brightness of shadowed faces.
    pub ambient: f32,
    /// Specular highlight strength (0.0–1.0).
    pub specular: f32,
    /// UV-rect highlight tint. `None` = no highlight.
    pub highlight: Option<Highlight>,
}

impl Default for MeshView {
    fn default() -> Self {
        Self {
            fov: 45.0,
            distance: 3.0,
            orientation: Orientation::IDENTITY,
            position: [0.0; 3],
            scale: 1.0,
            light: None,
            ambient: 0.3,
            specular: 0.4,
            highlight: None,
        }
    }
}

// ── Mesh draw arguments ─────────────────────────────────────────────

/// Camera + 3D transform parameters for a mesh draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshTransform {
    pub fov: f32,
    pub distance: f32,
    /// Orientation quaternion `(x, y, z, w)`.
    pub quat: [f32; 4],
    /// Offset relative to the draw rect centre in mesh-local units.
    pub position: [f32; 3],
    pub scale: f32,
}

/// Directional lighting.
/// `pitch == NaN` is the sentinel for "lighting disabled" (matches [`MeshView::light`] being `None`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshLighting {
    pub pitch: f32,
    pub yaw: f32,
    pub ambient: f32,
    pub specular: f32,
}

/// Optional UV-rect highlight tint.
/// `u_min == NaN` is the sentinel for "no highlight".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshHighlight {
    pub u_min: f32,
    pub v_min: f32,
    pub u_max: f32,
    pub v_max: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// Bundle of every per-draw mesh parameter,
/// as the wire carries it and the renderer consumes it.
/// One struct rather than a wide tuple, so a field added here
/// reaches every signature at once and a dirty check compares every field by definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshDrawArgs {
    pub transform: MeshTransform,
    pub lighting: MeshLighting,
    pub highlight: MeshHighlight,
}

impl From<MeshView> for MeshDrawArgs {
    fn from(view: MeshView) -> Self {
        let light = view.light.unwrap_or(LightAngles {
            pitch: f32::NAN,
            yaw: f32::NAN,
        });
        let highlight = view.highlight.unwrap_or(Highlight {
            uv_rect: [f32::NAN, 0.0, 0.0, 0.0],
            color: [0.0, 0.0, 0.0],
        });
        Self {
            transform: MeshTransform {
                fov: view.fov,
                distance: view.distance,
                quat: [
                    view.orientation.x,
                    view.orientation.y,
                    view.orientation.z,
                    view.orientation.w,
                ],
                position: view.position,
                scale: view.scale,
            },
            lighting: MeshLighting {
                pitch: light.pitch,
                yaw: light.yaw,
                ambient: view.ambient,
                specular: view.specular,
            },
            highlight: MeshHighlight {
                u_min: highlight.uv_rect[0],
                v_min: highlight.uv_rect[1],
                u_max: highlight.uv_rect[2],
                v_max: highlight.uv_rect[3],
                r: highlight.color[0],
                g: highlight.color[1],
                b: highlight.color[2],
            },
        }
    }
}

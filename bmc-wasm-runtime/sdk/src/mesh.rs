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

//! 3D mesh types for the WASM widget SDK.
//!
//! A `Mesh` embeds a preprocessed binary blob (output of `include_mesh!`).
//! `MeshView` defines camera, transform, and lighting for rendering.

use crate::orientation::Orientation;

/// Embedded 3D mesh data (output of `include_mesh!` proc macro).
///
/// The `data` field contains the optimized binary format (header + quantized
/// vertices + indices + optional texture). On first use, this data is sent to
/// the host via `host_register_mesh()` which uploads VBO/IBO/texture to GPU.
///
/// `face_normals` contains per-face normals in glTF Y-up space, ordered by
/// display face number (index 0 = face 1). Extracted from glTF `extras` at
/// compile time. Empty if the model has no face normal metadata.
#[derive(Debug)]
pub struct Mesh {
    pub data: &'static [u8],
    pub face_normals: &'static [[f32; 3]],
    /// Stable, unique-per-host registration tag (e.g. `"crate::stem"`).
    pub name: &'static str,
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

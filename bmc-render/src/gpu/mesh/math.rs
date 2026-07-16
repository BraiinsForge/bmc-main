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

//! Matrix and quaternion helpers used by the mesh renderer.
//!
//! Pure math; no GL or wasm context. Kept separate from `gpu/mesh.rs` so the
//! main file can focus on the GL state machine and renderer lifecycle.

pub(super) fn quat_to_mat3(q: [f32; 4]) -> [[f32; 3]; 3] {
    let m = glam::Mat3::from_quat(glam::Quat::from_xyzw(q[0], q[1], q[2], q[3]));
    // Each element is a glam column vector: r[col][row].
    [m.x_axis.into(), m.y_axis.into(), m.z_axis.into()]
}

pub(super) fn compute_mvp(
    rotation: &[[f32; 3]; 3],
    position: [f32; 3],
    scale: f32,
    fov_deg: f32,
    aspect: f32,
    distance: f32,
) -> [f32; 16] {
    // Model matrix: translate(position) * rotate(quat) * scale(s)
    // View matrix: translate(0, 0, -distance)
    // Projection: perspective

    let near = 0.1_f32;
    let far = 100.0_f32;
    let fov_rad = fov_deg.to_radians();
    let f = 1.0 / (fov_rad / 2.0).tan();

    // Perspective projection (column-major)
    let proj = [
        f / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        -f,
        0.0,
        0.0, // Negate Y to flip for FBO→femtovg (same as sphere)
        0.0,
        0.0,
        (far + near) / (near - far),
        -1.0,
        0.0,
        0.0,
        (2.0 * far * near) / (near - far),
        0.0,
    ];

    // Model-view matrix (column-major): view * translate * rotate * scale
    // r[col][row] from quat_to_mat3 — columns of R go into columns of MV.
    let r = rotation;
    let mv = [
        r[0][0] * scale,
        r[0][1] * scale,
        r[0][2] * scale,
        0.0,
        r[1][0] * scale,
        r[1][1] * scale,
        r[1][2] * scale,
        0.0,
        r[2][0] * scale,
        r[2][1] * scale,
        r[2][2] * scale,
        0.0,
        position[0],
        position[1],
        position[2] - distance,
        1.0,
    ];

    // MVP = proj * mv (4x4 column-major multiply)
    mat4_mul(&proj, &mv)
}

fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let ma = glam::Mat4::from_cols_array(a);
    let mb = glam::Mat4::from_cols_array(b);
    (ma * mb).to_cols_array()
}

pub(super) fn flatten_mat3(m: &[[f32; 3]; 3]) -> [f32; 9] {
    // m[col][row] → column-major flat array for GL uniform
    [
        m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2],
    ]
}

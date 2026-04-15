// Copyright (C) 2026  Braiins Systems s.r.o.

//! Quaternion-based 3D orientation with a human-readable construction API.
//!
//! Internally a unit quaternion `[x, y, z, w]`. All math runs in WASM —
//! the host receives the final four floats and passes them straight to
//! the vertex shader as a uniform.

use core::f32::consts::PI;

/// A unit quaternion representing a 3D orientation.
///
/// Constructed via builder methods that hide the quaternion math.
/// The raw `[x, y, z, w]` components are accessible for serialization.
#[derive(Debug, Clone, Copy)]
pub struct Orientation {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Orientation {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Orientation {
    /// Identity — no rotation.
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// Identity — no rotation (alias for readability).
    #[must_use]
    pub fn none() -> Self {
        Self::IDENTITY
    }

    /// Euler angles in degrees. Applied as Rz(roll) * Rx(pitch) * Ry(yaw).
    #[must_use]
    pub fn from_euler(pitch_deg: f32, yaw_deg: f32, roll_deg: f32) -> Self {
        let pitch = pitch_deg * (PI / 180.0);
        let yaw = yaw_deg * (PI / 180.0);
        let roll = roll_deg * (PI / 180.0);

        let (sp, cp) = (pitch * 0.5).sin_cos();
        let (sy, cy) = (yaw * 0.5).sin_cos();
        let (sr, cr) = (roll * 0.5).sin_cos();

        // Rz(roll) * Rx(pitch) * Ry(yaw)
        Self {
            x: cr * sp * cy - sr * cp * sy,
            y: cr * cp * sy + sr * sp * cy,
            z: sr * cp * cy - cr * sp * sy,
            w: cr * cp * cy + sr * sp * sy,
        }
        .normalized()
    }

    /// Rotate `angle_deg` degrees around an arbitrary axis.
    ///
    /// The axis does not need to be normalized — it will be normalized internally.
    #[must_use]
    pub fn from_axis_angle(ax: f32, ay: f32, az: f32, angle_deg: f32) -> Self {
        let len = (ax * ax + ay * ay + az * az).sqrt();
        if len < 1e-8 {
            return Self::IDENTITY;
        }
        let inv = 1.0 / len;
        let half = angle_deg * (PI / 360.0);
        let (s, c) = half.sin_cos();
        Self {
            x: ax * inv * s,
            y: ay * inv * s,
            z: az * inv * s,
            w: c,
        }
    }

    /// Point the mesh's +Z toward a geographic direction.
    ///
    /// Useful for globe-like widgets where you want to aim a mesh at a
    /// specific latitude/longitude.
    #[must_use]
    pub fn look_at(lat_deg: f32, lon_deg: f32) -> Self {
        // Ry(lon) * Rx(-lat)
        let ry = Self::from_axis_angle(0.0, 1.0, 0.0, lon_deg);
        let rx = Self::from_axis_angle(1.0, 0.0, 0.0, -lat_deg);
        ry.then(rx)
    }

    /// Compose: apply `other` rotation after `self`.
    ///
    /// `a.then(b)` means "first rotate by a, then by b".
    #[must_use]
    pub fn then(self, other: Self) -> Self {
        // Hamilton product: other * self (quaternion multiplication is right-to-left)
        Self {
            x: other.w * self.x + other.x * self.w + other.y * self.z - other.z * self.y,
            y: other.w * self.y - other.x * self.z + other.y * self.w + other.z * self.x,
            z: other.w * self.z + other.x * self.y - other.y * self.x + other.z * self.w,
            w: other.w * self.w - other.x * self.x - other.y * self.y - other.z * self.z,
        }
        .normalized()
    }

    /// Normalize to unit quaternion (handles accumulated floating-point drift).
    #[must_use]
    fn normalized(self) -> Self {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if len < 1e-8 {
            return Self::IDENTITY;
        }
        let inv = 1.0 / len;
        Self {
            x: self.x * inv,
            y: self.y * inv,
            z: self.z * inv,
            w: self.w * inv,
        }
    }
}

// -- glam interop (requires `math-3d` feature) --

#[cfg(feature = "math-3d")]
impl From<glam::Quat> for Orientation {
    fn from(q: glam::Quat) -> Self {
        Self {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.w,
        }
    }
}

#[cfg(feature = "math-3d")]
impl From<Orientation> for glam::Quat {
    fn from(o: Orientation) -> Self {
        Self::from_xyzw(o.x, o.y, o.z, o.w)
    }
}

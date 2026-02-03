// Copyright (C) 2025  Braiins Systems s.r.o.

//! 2D transform helpers for rotation, scaling, and translation.

use core::f32::consts::PI;

/// A 2D transformation that can be applied to points.
///
/// Transformations are applied in order: scale -> rotate -> translate.
/// The origin point defines the center for rotation and scaling.
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    /// Translation offset (x, y)
    pub translate: (f32, f32),
    /// Rotation angle in radians
    pub rotate: f32,
    /// Scale factors (x, y)
    pub scale: (f32, f32),
    /// Transform origin / pivot point
    pub origin: (f32, f32),
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    /// Identity transform (no change).
    pub fn identity() -> Self {
        Self {
            translate: (0.0, 0.0),
            rotate: 0.0,
            scale: (1.0, 1.0),
            origin: (0.0, 0.0),
        }
    }

    /// Create a rotation transform around a center point.
    ///
    /// # Example
    /// ```ignore
    /// let transform = Transform::rotate_around((120.0, 120.0), PI / 2.0);
    /// let (x, y) = transform.apply_point(120.0, 40.0); // Rotates point 90 degrees
    /// ```
    pub fn rotate_around(center: (f32, f32), angle: f32) -> Self {
        Self {
            translate: (0.0, 0.0),
            rotate: angle,
            scale: (1.0, 1.0),
            origin: center,
        }
    }

    /// Create a scale transform around a center point.
    pub fn scale_around(center: (f32, f32), scale: (f32, f32)) -> Self {
        Self {
            translate: (0.0, 0.0),
            rotate: 0.0,
            scale,
            origin: center,
        }
    }

    /// Apply this transform to a point, returning the transformed coordinates.
    pub fn apply_point(&self, x: f32, y: f32) -> (f32, f32) {
        // Translate to origin
        let dx = x - self.origin.0;
        let dy = y - self.origin.1;

        // Scale
        let sx = dx * self.scale.0;
        let sy = dy * self.scale.1;

        // Rotate
        let cos = cos_approx(self.rotate);
        let sin = sin_approx(self.rotate);
        let rx = sx * cos - sy * sin;
        let ry = sx * sin + sy * cos;

        // Translate back and apply offset
        (
            rx + self.origin.0 + self.translate.0,
            ry + self.origin.1 + self.translate.1,
        )
    }

    /// Set translation.
    pub fn with_translate(mut self, x: f32, y: f32) -> Self {
        self.translate = (x, y);
        self
    }

    /// Set rotation in radians.
    pub fn with_rotate(mut self, angle: f32) -> Self {
        self.rotate = angle;
        self
    }

    /// Set scale.
    pub fn with_scale(mut self, sx: f32, sy: f32) -> Self {
        self.scale = (sx, sy);
        self
    }

    /// Set transform origin.
    pub fn with_origin(mut self, x: f32, y: f32) -> Self {
        self.origin = (x, y);
        self
    }
}

/// Convert degrees to radians.
pub fn deg_to_rad(degrees: f32) -> f32 {
    degrees * PI / 180.0
}

/// Convert radians to degrees.
pub fn rad_to_deg(radians: f32) -> f32 {
    radians * 180.0 / PI
}

fn sin_approx(x: f32) -> f32 {
    libm::sinf(x)
}

fn cos_approx(x: f32) -> f32 {
    libm::cosf(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity() {
        let t = Transform::identity();
        let (x, y) = t.apply_point(10.0, 20.0);
        assert!((x - 10.0).abs() < 0.001);
        assert!((y - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_rotate_90() {
        let t = Transform::rotate_around((0.0, 0.0), PI / 2.0);
        let (x, y) = t.apply_point(1.0, 0.0);
        assert!(x.abs() < 0.01);
        assert!((y - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_scale() {
        let t = Transform::scale_around((0.0, 0.0), (2.0, 3.0));
        let (x, y) = t.apply_point(10.0, 10.0);
        assert!((x - 20.0).abs() < 0.001);
        assert!((y - 30.0).abs() < 0.001);
    }
}

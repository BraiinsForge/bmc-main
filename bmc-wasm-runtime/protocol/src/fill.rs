// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shape fill paints: solid colour, linear gradient, and radial gradient.

use crate::colors::Color;

/// Wire discriminant for a solid-colour paint.
pub const FILL_SOLID: u8 = 0;
/// Wire discriminant for a linear-gradient paint.
pub const FILL_LINEAR: u8 = 1;
/// Wire discriminant for a radial-gradient paint.
pub const FILL_RADIAL: u8 = 2;

/// Paint applied to a fillable 2D shape (rectangle, circle, filled polygon).
///
/// Orthogonal to shape: any variant works on any of the three shapes. A bare
/// [`Color`] converts via [`From`], so existing solid call sites stay source
/// compatible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    /// Single flat colour.
    Solid(Color),
    /// Linear gradient at `angle` degrees (`0` = top→bottom, `90` = left→right,
    /// increasing clockwise), spanning the shape's bounding box.
    Linear {
        angle: f32,
        start: Color,
        end: Color,
    },
    /// Radial gradient from the shape centre (`inner`) to its edge (`outer`).
    Radial { inner: Color, outer: Color },
}

impl From<Color> for Fill {
    fn from(c: Color) -> Self {
        Fill::Solid(c)
    }
}

impl Fill {
    /// Linear gradient, `angle` in degrees.
    #[must_use]
    pub const fn linear(angle: f32, start: Color, end: Color) -> Self {
        Fill::Linear { angle, start, end }
    }

    /// Radial gradient, centre to edge.
    #[must_use]
    pub const fn radial(inner: Color, outer: Color) -> Self {
        Fill::Radial { inner, outer }
    }

    /// Multiply the alpha of every colour stop by `factor` (canvas opacity).
    #[must_use]
    pub fn scale_alpha(self, factor: f32) -> Self {
        match self {
            Fill::Solid(c) => Fill::Solid(c.scale_alpha(factor)),
            Fill::Linear { angle, start, end } => Fill::Linear {
                angle,
                start: start.scale_alpha(factor),
                end: end.scale_alpha(factor),
            },
            Fill::Radial { inner, outer } => Fill::Radial {
                inner: inner.scale_alpha(factor),
                outer: outer.scale_alpha(factor),
            },
        }
    }

    /// A single representative colour, for systems that interpolate one colour
    /// (e.g. host transitions): the solid colour, the linear `start`, or the
    /// radial `inner`.
    #[must_use]
    pub const fn primary_color(self) -> Color {
        match self {
            Fill::Solid(c) | Fill::Linear { start: c, .. } | Fill::Radial { inner: c, .. } => c,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::Color;

    const RED: Color = Color::from_rgb(0xFF, 0x00, 0x00);
    const BLUE: Color = Color::from_rgb(0x00, 0x00, 0xFF);

    #[test]
    fn color_converts_to_solid_fill() {
        assert_eq!(Fill::from(RED), Fill::Solid(RED));
    }

    #[test]
    fn linear_constructor_sets_fields() {
        let f = Fill::linear(90.0, RED, BLUE);
        assert_eq!(
            f,
            Fill::Linear {
                angle: 90.0,
                start: RED,
                end: BLUE
            }
        );
    }

    #[test]
    fn radial_constructor_sets_fields() {
        let f = Fill::radial(RED, BLUE);
        assert_eq!(
            f,
            Fill::Radial {
                inner: RED,
                outer: BLUE
            }
        );
    }

    #[test]
    fn scale_alpha_scales_every_stop() {
        let half = Fill::linear(0.0, RED.with_alpha(1.0), BLUE.with_alpha(1.0)).scale_alpha(0.5);
        let Fill::Linear { start, end, .. } = half else {
            panic!("BUG: scale_alpha changed the variant");
        };
        assert_eq!(start.alpha(), 0x7F);
        assert_eq!(end.alpha(), 0x7F);
    }

    #[test]
    fn primary_color_picks_a_representative_stop() {
        assert_eq!(Fill::Solid(RED).primary_color(), RED);
        assert_eq!(Fill::linear(0.0, RED, BLUE).primary_color(), RED);
        assert_eq!(Fill::radial(RED, BLUE).primary_color(), RED);
    }
}

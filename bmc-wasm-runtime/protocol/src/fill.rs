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

    /// The solid colour, panicking if this is a gradient.
    ///
    /// Used on the stroke serialization path, where the `path!` macro
    /// guarantees a solid paint.
    #[must_use]
    pub fn expect_solid(self) -> Color {
        match self {
            Fill::Solid(c) => c,
            Fill::Linear { .. } | Fill::Radial { .. } => {
                panic!("BUG: stroke paths must carry a solid fill, got a gradient")
            }
        }
    }
}

/// Append `fill` to `out` in wire format.
pub fn encode_fill(out: &mut Vec<u8>, fill: &Fill) {
    match fill {
        Fill::Solid(c) => {
            out.push(FILL_SOLID);
            out.extend_from_slice(&c.to_u32().to_le_bytes());
        }
        Fill::Linear { angle, start, end } => {
            out.push(FILL_LINEAR);
            out.extend_from_slice(&start.to_u32().to_le_bytes());
            out.extend_from_slice(&end.to_u32().to_le_bytes());
            out.extend_from_slice(&angle.to_le_bytes());
        }
        Fill::Radial { inner, outer } => {
            out.push(FILL_RADIAL);
            out.extend_from_slice(&inner.to_u32().to_le_bytes());
            out.extend_from_slice(&outer.to_u32().to_le_bytes());
        }
    }
}

/// Read a `Fill` from `data` starting at `*pos`, advancing `*pos` past it.
///
/// Returns `None` on an unknown discriminant or truncated input.  On `None`,
/// `*pos` is left in an unspecified state; the partial parse may have advanced it.
#[must_use]
pub fn decode_fill(data: &[u8], pos: &mut usize) -> Option<Fill> {
    let kind = *data.get(*pos)?;
    *pos += 1;
    match kind {
        FILL_SOLID => Some(Fill::Solid(read_color(data, pos)?)),
        FILL_LINEAR => {
            let start = read_color(data, pos)?;
            let end = read_color(data, pos)?;
            let angle = read_f32(data, pos)?;
            Some(Fill::Linear { angle, start, end })
        }
        FILL_RADIAL => {
            let inner = read_color(data, pos)?;
            let outer = read_color(data, pos)?;
            Some(Fill::Radial { inner, outer })
        }
        _ => None,
    }
}

fn read_color(data: &[u8], pos: &mut usize) -> Option<Color> {
    Some(Color::from_raw(read_u32(data, pos)?))
}

fn read_u32(data: &[u8], pos: &mut usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(*pos..*pos + 4)?.try_into().ok()?;
    *pos += 4;
    Some(u32::from_le_bytes(bytes))
}

fn read_f32(data: &[u8], pos: &mut usize) -> Option<f32> {
    let bytes: [u8; 4] = data.get(*pos..*pos + 4)?.try_into().ok()?;
    *pos += 4;
    Some(f32::from_le_bytes(bytes))
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

    #[test]
    fn expect_solid_returns_the_colour() {
        assert_eq!(Fill::Solid(RED).expect_solid(), RED);
    }

    fn round_trip(fill: Fill) -> Fill {
        let mut buf = Vec::new();
        encode_fill(&mut buf, &fill);
        let mut pos = 0;
        let decoded =
            decode_fill(&buf, &mut pos).expect("BUG: encoded fill must decode successfully");
        assert_eq!(pos, buf.len(), "decode must consume every byte it wrote");
        decoded
    }

    #[test]
    fn solid_round_trips() {
        assert_eq!(round_trip(Fill::Solid(RED)), Fill::Solid(RED));
    }

    #[test]
    fn linear_round_trips() {
        let f = Fill::linear(45.0, RED, BLUE);
        assert_eq!(round_trip(f), f);
    }

    #[test]
    fn radial_round_trips() {
        let f = Fill::radial(RED, BLUE);
        assert_eq!(round_trip(f), f);
    }

    #[test]
    fn solid_layout_is_kind_then_colour() {
        let mut buf = Vec::new();
        encode_fill(&mut buf, &Fill::Solid(RED));
        assert_eq!(buf[0], FILL_SOLID);
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn decode_rejects_truncated_input() {
        let buf = [FILL_LINEAR, 0x00, 0x00];
        let mut pos = 0;
        assert!(decode_fill(&buf, &mut pos).is_none());
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "byte-layout test asserts exact bit pattern"
    )]
    fn linear_layout_is_start_end_angle() {
        let mut buf = Vec::new();
        encode_fill(&mut buf, &Fill::linear(90.0, RED, BLUE));
        assert_eq!(buf[0], FILL_LINEAR);
        assert_eq!(buf.len(), 13);
        assert_eq!(
            u32::from_le_bytes(
                buf[1..5]
                    .try_into()
                    .expect("BUG: start colour slice has four bytes"),
            ),
            RED.to_u32()
        );
        assert_eq!(
            u32::from_le_bytes(
                buf[5..9]
                    .try_into()
                    .expect("BUG: end colour slice has four bytes"),
            ),
            BLUE.to_u32()
        );
        assert_eq!(
            f32::from_le_bytes(
                buf[9..13]
                    .try_into()
                    .expect("BUG: angle slice has four bytes"),
            ),
            90.0_f32
        );
    }
}

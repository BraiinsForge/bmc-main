// Copyright (C) 2026  Braiins Systems s.r.o.

//! Arc stroke paints and segmentation for the `stroke_arc` primitive.
//!
//! Angles are in radians, `0` at 12 o'clock, increasing clockwise (screen
//! coordinates, y down). Colours and spans are parameterised over the full
//! `[start_angle, end_angle]` sweep so the gradient flows continuously across
//! segment gaps.

use crate::colors::Color;

/// Wire discriminant for a solid-colour arc stroke.
pub const ARC_FILL_SOLID: u8 = 0;
/// Wire discriminant for an along-arc gradient stroke.
pub const ARC_FILL_GRADIENT: u8 = 1;
/// Wire discriminant for one visible span covering the whole arc sweep.
pub const ARC_SEGMENTS_CONTINUOUS: u8 = 0;
/// Wire discriminant for explicit visible arc spans.
pub const ARC_SEGMENTS_EXPLICIT: u8 = 1;

/// Paint applied along an arc stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArcFill {
    /// Single flat colour.
    Solid(Color),
    /// Two-colour gradient interpolated `start` to `end` along the sweep.
    Gradient { start: Color, end: Color },
}

/// Visible angular spans of an arc within its `[start_angle, end_angle]` sweep.
#[derive(Clone, Debug, PartialEq)]
pub enum ArcSegments {
    /// One span covering the whole sweep; round caps on both ends.
    Continuous,
    /// Ordered visible spans `(a0, a1)`. Round caps appear only on the first
    /// span's start and the last span's end; interior ends are flat.
    Explicit(Vec<(f32, f32)>),
}

impl From<Color> for ArcFill {
    fn from(c: Color) -> Self {
        ArcFill::Solid(c)
    }
}

impl ArcFill {
    /// Along-arc gradient.
    #[must_use]
    pub const fn gradient(start: Color, end: Color) -> Self {
        ArcFill::Gradient { start, end }
    }

    /// Multiply the alpha of every colour stop by `factor` (canvas opacity).
    #[must_use]
    pub fn scale_alpha(self, factor: f32) -> Self {
        match self {
            ArcFill::Solid(c) => ArcFill::Solid(c.scale_alpha(factor)),
            ArcFill::Gradient { start, end } => ArcFill::Gradient {
                start: start.scale_alpha(factor),
                end: end.scale_alpha(factor),
            },
        }
    }

    /// A single representative colour for one-colour interpolators (host
    /// transitions): the solid colour or the gradient `start`.
    #[must_use]
    pub const fn primary_color(self) -> Color {
        match self {
            ArcFill::Solid(c) | ArcFill::Gradient { start: c, .. } => c,
        }
    }
}

impl ArcSegments {
    /// `count` equal segments separated by equal `gap` angular gaps.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "segment counts are UI geometry counts converted to f32 angles"
    )]
    pub fn uniform(start: f32, end: f32, count: usize, gap: f32) -> Self {
        if count == 0 {
            return ArcSegments::Explicit(Vec::new());
        }
        let gaps = count.saturating_sub(1);
        let seg = (end - start - gap * gaps as f32) / count as f32;
        let mut spans = Vec::with_capacity(count);
        let mut a = start;
        for _ in 0..count {
            spans.push((a, a + seg));
            a += seg + gap;
        }
        ArcSegments::Explicit(spans)
    }

    /// Equal interior segments with the first and last scaled by `end_scale`.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "segment counts are UI geometry counts converted to f32 angles"
    )]
    pub fn short_ends(start: f32, end: f32, count: usize, gap: f32, end_scale: f32) -> Self {
        match count {
            0 => return ArcSegments::Explicit(Vec::new()),
            1 => return ArcSegments::Explicit(vec![(start, end)]),
            _ => {}
        }
        let gaps = (count - 1) as f32;
        let interior = (count - 2) as f32;
        let interior_len = (end - start - gap * gaps) / (2.0 * end_scale + interior);
        let end_len = interior_len * end_scale;
        let mut spans = Vec::with_capacity(count);
        let mut a = start;
        for i in 0..count {
            let len = if i == 0 || i == count - 1 {
                end_len
            } else {
                interior_len
            };
            spans.push((a, a + len));
            a += len + gap;
        }
        ArcSegments::Explicit(spans)
    }
}

/// Append `fill` to `out` in wire format.
pub fn encode_arc_fill(out: &mut Vec<u8>, fill: &ArcFill) {
    match fill {
        ArcFill::Solid(c) => {
            out.push(ARC_FILL_SOLID);
            out.extend_from_slice(&c.to_u32().to_le_bytes());
        }
        ArcFill::Gradient { start, end } => {
            out.push(ARC_FILL_GRADIENT);
            out.extend_from_slice(&start.to_u32().to_le_bytes());
            out.extend_from_slice(&end.to_u32().to_le_bytes());
        }
    }
}

/// Read an `ArcFill` from `data` starting at `*pos`, advancing `*pos` past it.
///
/// Returns `None` on an unknown discriminant or truncated input. On `None`,
/// `*pos` is left in an unspecified state; the partial parse may have advanced it.
#[must_use]
pub fn decode_arc_fill(data: &[u8], pos: &mut usize) -> Option<ArcFill> {
    let kind = *data.get(*pos)?;
    *pos += 1;
    match kind {
        ARC_FILL_SOLID => Some(ArcFill::Solid(read_color(data, pos)?)),
        ARC_FILL_GRADIENT => {
            let start = read_color(data, pos)?;
            let end = read_color(data, pos)?;
            Some(ArcFill::Gradient { start, end })
        }
        _ => None,
    }
}

/// Append `segments` to `out` in wire format.
pub fn encode_arc_segments(out: &mut Vec<u8>, segments: &ArcSegments) {
    match segments {
        ArcSegments::Continuous => out.push(ARC_SEGMENTS_CONTINUOUS),
        ArcSegments::Explicit(spans) => {
            out.push(ARC_SEGMENTS_EXPLICIT);
            let count = u32::try_from(spans.len()).expect("BUG: arc segment count exceeds u32");
            out.extend_from_slice(&count.to_le_bytes());
            for (start, end) in spans {
                out.extend_from_slice(&start.to_le_bytes());
                out.extend_from_slice(&end.to_le_bytes());
            }
        }
    }
}

/// Read `ArcSegments` from `data` starting at `*pos`, advancing `*pos` past it.
///
/// Returns `None` on an unknown discriminant or truncated input. On `None`,
/// `*pos` is left in an unspecified state; the partial parse may have advanced it.
#[must_use]
pub fn decode_arc_segments(data: &[u8], pos: &mut usize) -> Option<ArcSegments> {
    let kind = *data.get(*pos)?;
    *pos += 1;
    match kind {
        ARC_SEGMENTS_CONTINUOUS => Some(ArcSegments::Continuous),
        ARC_SEGMENTS_EXPLICIT => {
            let count = usize::try_from(read_u32(data, pos)?).ok()?;
            let remaining_segments = data.get(*pos..)?.chunks_exact(8).len();
            if count > remaining_segments {
                return None;
            }
            let mut spans = Vec::with_capacity(count);
            for _ in 0..count {
                let start = read_f32(data, pos)?;
                let end = read_f32(data, pos)?;
                spans.push((start, end));
            }
            Some(ArcSegments::Explicit(spans))
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

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    fn spans(s: &ArcSegments) -> &[(f32, f32)] {
        match s {
            ArcSegments::Explicit(v) => v,
            ArcSegments::Continuous => panic!("BUG: expected Explicit"),
        }
    }

    fn round_trip_fill(fill: ArcFill) -> ArcFill {
        let mut buf = Vec::new();
        encode_arc_fill(&mut buf, &fill);
        let mut pos = 0;
        let decoded = decode_arc_fill(&buf, &mut pos).expect("BUG: encoded arc fill must decode");
        assert_eq!(pos, buf.len(), "decode must consume every byte it wrote");
        decoded
    }

    fn round_trip_segments(segments: &ArcSegments) -> ArcSegments {
        let mut buf = Vec::new();
        encode_arc_segments(&mut buf, segments);
        let mut pos = 0;
        let decoded =
            decode_arc_segments(&buf, &mut pos).expect("BUG: encoded arc segments must decode");
        assert_eq!(pos, buf.len(), "decode must consume every byte it wrote");
        decoded
    }

    #[test]
    fn uniform_yields_equal_segments_with_equal_gaps() {
        let s = ArcSegments::uniform(0.0, 1.0, 4, 0.04);
        let v = spans(&s);
        assert_eq!(v.len(), 4);
        let expected = [(0.0, 0.22), (0.26, 0.48), (0.52, 0.74), (0.78, 1.0)];
        for (got, want) in v.iter().zip(expected) {
            approx(got.0, want.0);
            approx(got.1, want.1);
        }
    }

    #[test]
    fn short_ends_scales_first_and_last_and_lands_on_end() {
        let s = ArcSegments::short_ends(0.0, 1.0, 4, 0.04, 0.5);
        let v = spans(&s);
        assert_eq!(v.len(), 4);
        let interior = v[1].1 - v[1].0;
        let first = v[0].1 - v[0].0;
        let last = v[3].1 - v[3].0;
        approx(first, interior * 0.5);
        approx(last, interior * 0.5);
        approx(v[3].1, 1.0);
        approx(v[0].0, 0.0);
    }

    #[test]
    fn solid_fill_round_trips() {
        assert_eq!(round_trip_fill(ArcFill::Solid(RED)), ArcFill::Solid(RED));
    }

    #[test]
    fn gradient_fill_round_trips() {
        let fill = ArcFill::gradient(RED, BLUE);
        assert_eq!(round_trip_fill(fill), fill);
    }

    #[test]
    fn continuous_segments_round_trip() {
        assert_eq!(
            round_trip_segments(&ArcSegments::Continuous),
            ArcSegments::Continuous
        );
    }

    #[test]
    fn explicit_segments_round_trip() {
        let segments = ArcSegments::Explicit(vec![(0.0, 1.0), (1.5, 2.0)]);
        assert_eq!(round_trip_segments(&segments), segments);
    }

    #[test]
    fn decode_segments_rejects_huge_count_without_payload() {
        let mut buf = Vec::new();
        buf.push(ARC_SEGMENTS_EXPLICIT);
        buf.extend_from_slice(&u32::MAX.to_le_bytes());

        let mut pos = 0;
        assert!(decode_arc_segments(&buf, &mut pos).is_none());
    }

    #[test]
    fn decode_fill_rejects_unknown_discriminant() {
        let mut pos = 0;
        assert!(decode_arc_fill(&[0xFF], &mut pos).is_none());
    }
}

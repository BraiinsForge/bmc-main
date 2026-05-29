// Copyright (C) 2026  Braiins Systems s.r.o.

//! Arc stroke paints and segmentation for the `stroke_arc` primitive.
//!
//! Angles are in radians, `0` at 12 o'clock, increasing clockwise (screen
//! coordinates, y down). Colours and spans are parameterised over the full
//! `[start_angle, end_angle]` sweep so the gradient flows continuously across
//! segment gaps.

use crate::colors::Color;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    fn spans(s: &ArcSegments) -> &[(f32, f32)] {
        match s {
            ArcSegments::Explicit(v) => v,
            ArcSegments::Continuous => panic!("BUG: expected Explicit"),
        }
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
}

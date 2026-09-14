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

//! Which layout a viewport gets, which face it draws and how the hands move.

use bmc_wasm_sdk::{Draw, Easing, SizeVariant, ViewportShape, WidgetSize, WidgetViewport};

use crate::manifest_params::ClockStyle;

/// The frames the faces lay out for: the four BMC100 slots,
/// and BMM101's 480×320 as a frame of its own rather than a Large scaled down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Full,
    Large,
    Medium,
    Small,
    Bmm101,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (u32, u32) {
        match self {
            Self::Full => (1_280, 480),
            Self::Large => (638, 480),
            Self::Medium => (638, 238),
            Self::Small => (317, 238),
            Self::Bmm101 => (480, 320),
        }
    }

    /// The BMC100 variant a face without a layout of its own scales down from.
    #[must_use]
    pub const fn variant(self) -> SizeVariant {
        match self {
            Self::Full => SizeVariant::Full,
            Self::Large | Self::Bmm101 => SizeVariant::Large,
            Self::Medium => SizeVariant::Medium,
            Self::Small => SizeVariant::Small,
        }
    }
}

/// A viewport classified once for every face:
/// its pixels and the bucket that picks the layout.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub size: WidgetSize,
    pub bucket: SizeBucket,
}

impl Frame {
    #[must_use]
    pub fn of(viewport: WidgetViewport) -> Self {
        Self {
            size: WidgetSize::from_dimensions(viewport.width, viewport.height),
            bucket: size_bucket(viewport.width, viewport.height),
        }
    }
}

/// The two BMM frames the narrow bucket has to tell apart.
const BMM100_HEIGHT: u32 = 240;
const BMM101_HEIGHT: u32 = 320;
/// Split at their midpoint, so either frame keeps its bucket a few pixels either way.
const BMM101_MIN_HEIGHT: u32 = u32::midpoint(BMM100_HEIGHT, BMM101_HEIGHT);
const BMM101_MAX_WIDTH: u32 = 480;

/// A landscape frame no wider than BMM101 and at least as tall as the BMM split
/// is BMM101; everything else takes the closest BMC100 variant, as the SDK does.
/// The square BFM100 falls to the second rule,
/// so it stays the Large the round face already lays out for.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width <= BMM101_MAX_WIDTH && height < width && height >= BMM101_MIN_HEIGHT {
        return SizeBucket::Bmm101;
    }
    match SizeVariant::closest(width, height) {
        SizeVariant::Full => SizeBucket::Full,
        SizeVariant::Large => SizeBucket::Large,
        SizeVariant::Medium => SizeBucket::Medium,
        SizeVariant::Small => SizeBucket::Small,
    }
}

/// The face a viewport draws. A round viewport forces the round dial — a
/// rectangular dial on round hardware clips at the corners — while a
/// rectangular viewport honours the operator's configured style.
#[must_use]
pub fn face(configured: ClockStyle, shape: ViewportShape) -> ClockStyle {
    match shape {
        ViewportShape::Round => ClockStyle::AnalogRound,
        ViewportShape::Rectangular => configured,
    }
}

/// Widgets get no entering/visible lifecycle hook, so a render gap this long
/// stands in for one: the hands snap to the current time
/// instead of sweeping from where the page left them.
/// Keyed on the render delta rather than wall-clock time,
/// so a DST or NTP step still sweeps the hands.
const MAX_ANIMATED_RENDER_GAP_MS: u32 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockHandTransition {
    Animate,
    Snap,
}

impl ClockHandTransition {
    #[must_use]
    pub fn for_render_gap(delta_ms: u32) -> Self {
        if delta_ms > MAX_ANIMATED_RENDER_GAP_MS {
            Self::Snap
        } else {
            Self::Animate
        }
    }

    #[must_use]
    pub fn apply(self, draw: Draw, id: &str, duration_ms: u32) -> Draw {
        let duration_ms = match self {
            Self::Animate => duration_ms,
            Self::Snap => 0,
        };
        draw.transition(id, duration_ms, Easing::EaseOut)
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::WHITE;

    use super::*;

    #[test]
    fn hand_transitions_animate_through_five_second_render_gaps() {
        for delta_ms in [0, 1_000, 4_999, 5_000] {
            assert_eq!(
                ClockHandTransition::for_render_gap(delta_ms),
                ClockHandTransition::Animate
            );
        }
    }

    #[test]
    fn hand_transitions_snap_after_longer_render_gaps() {
        for delta_ms in [5_001, 30_000, u32::MAX] {
            assert_eq!(
                ClockHandTransition::for_render_gap(delta_ms),
                ClockHandTransition::Snap
            );
        }
    }

    #[test]
    fn snapping_preserves_hand_identity_and_resumes_normal_duration() {
        for (id, duration_ms) in [
            ("hour-hand", 500),
            ("minute-hand", 500),
            ("second-hand", 200),
        ] {
            let mut identity = None;
            for (delta_ms, expected_duration) in
                [(1_000, duration_ms), (5_001, 0), (1_000, duration_ms)]
            {
                let draw = ClockHandTransition::for_render_gap(delta_ms).apply(
                    Draw::rect(0.0, 0.0, 1.0, 1.0, WHITE),
                    id,
                    duration_ms,
                );
                let Draw::Modified {
                    transition: Some(transition),
                    ..
                } = draw
                else {
                    panic!("each hand must retain its transition across a render gap");
                };
                assert_eq!(transition.duration_ms, expected_duration);
                assert_eq!(transition.easing, Easing::EaseOut);
                assert_eq!(
                    *identity.get_or_insert(transition.id_hash),
                    transition.id_hash
                );
            }
        }
    }

    #[test]
    fn every_design_size_lands_in_its_own_bucket() {
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let (width, height) = bucket.design_size();
            assert_eq!(size_bucket(width, height), bucket, "{width}x{height}");
        }
    }

    /// A frame that misses the BMM101 rule by a pixel falls to the SDK's closest variant.
    #[test]
    fn the_bmm101_bucket_ends_at_the_midpoint_of_the_bmm_heights_and_at_its_width() {
        assert_eq!(size_bucket(320, 240), SizeBucket::Small, "BMM100");
        assert_eq!(size_bucket(480, 280), SizeBucket::Bmm101);
        assert_eq!(size_bucket(480, 279), SizeBucket::Medium);
        assert_eq!(size_bucket(481, 320), SizeBucket::Large);
    }

    #[test]
    fn the_square_bfm100_stays_large() {
        assert_eq!(size_bucket(480, 480), SizeBucket::Large);
    }

    #[test]
    fn round_viewport_forces_round_analog() {
        assert_eq!(
            face(ClockStyle::Digital, ViewportShape::Round),
            ClockStyle::AnalogRound
        );
        assert_eq!(
            face(ClockStyle::AnalogRect, ViewportShape::Round),
            ClockStyle::AnalogRound
        );
    }

    #[test]
    fn rectangular_viewport_honors_configured_style() {
        assert_eq!(
            face(ClockStyle::Digital, ViewportShape::Rectangular),
            ClockStyle::Digital
        );
        assert_eq!(
            face(ClockStyle::AnalogRect, ViewportShape::Rectangular),
            ClockStyle::AnalogRect
        );
    }
}

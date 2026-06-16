// Copyright (C) 2026  Braiins Systems s.r.o.

//! Pure dismiss-gesture classification and brightness mapping for the
//! settings-tray, kept GPU-free so the hit/gesture behaviour is unit-testable.

/// A touch point in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

/// Upward travel (px) required for a swipe-up dismiss.
const DISMISS_DY: f32 = 60.0;

/// Whether a finished touch (`start`→`end`) should dismiss the tray: an upward
/// swipe past `DISMISS_DY` that is predominantly vertical, distinct from the
/// horizontal slider drag. The tray is full-screen, so every touch is on it.
#[must_use]
pub fn classify(start: Pt, end: Pt) -> bool {
    let dx = (end.x - start.x).abs();
    let upward = start.y - end.y; // positive = upward
    upward >= DISMISS_DY && upward > dx
}

/// Map a slider drag fraction (0.0..1.0) to a brightness percentage in 10..100,
/// matching the original `settings-stub` slider (`MIN=10`, `MAX=100`).
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 10.0..=100.0 before the cast; there is no TryFrom<f32> for u8"
)]
pub fn brightness_from_fraction(frac: f32) -> u8 {
    let pct = (10.0 + frac.clamp(0.0, 1.0) * 90.0).round();
    pct.clamp(10.0, 100.0) as u8
}

/// Map a brightness percentage (0..100, possibly a sub-floor night-mode value)
/// to the slider's display fraction, clamped to 0.0..1.0 so a value below the
/// 10 floor renders at 0 rather than underflowing (`(b-10)/90` would go
/// negative).
#[must_use]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "retained for its unit-test contract; no longer on the render call path"
    )
)]
pub fn brightness_display_fraction(brightness: u8) -> f32 {
    let b = f32::from(brightness).clamp(10.0, 100.0);
    ((b - 10.0) / 90.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Pt {
        Pt { x, y }
    }

    #[test]
    fn upward_swipe_dismisses() {
        // travel up past the threshold, mostly vertical.
        assert!(classify(pt(240.0, 150.0), pt(250.0, 60.0)));
    }

    #[test]
    fn horizontal_drag_is_not_dismiss() {
        // a slider drag: large dx, small upward dy -> the tree owns it.
        assert!(!classify(pt(100.0, 120.0), pt(300.0, 118.0)));
    }

    #[test]
    fn downward_swipe_is_not_dismiss() {
        assert!(!classify(pt(240.0, 60.0), pt(245.0, 150.0)));
    }

    #[test]
    fn short_upward_swipe_is_not_dismiss() {
        // upward but short of DISMISS_DY -> not a dismiss.
        assert!(!classify(pt(240.0, 150.0), pt(242.0, 120.0)));
    }

    #[test]
    fn brightness_maps_fraction_to_ten_to_hundred() {
        assert_eq!(brightness_from_fraction(0.0), 10);
        assert_eq!(brightness_from_fraction(0.5), 55);
        assert_eq!(brightness_from_fraction(1.0), 100);
        assert_eq!(brightness_from_fraction(2.0), 100, "clamped");
    }

    #[test]
    fn display_fraction_clamps_sub_ten_night_value_to_zero() {
        // Exact constants: compare bit patterns to satisfy the float-cmp lint.
        assert_eq!(brightness_display_fraction(10).to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            brightness_display_fraction(100).to_bits(),
            1.0_f32.to_bits()
        );
        // A night-mode value below the slider floor must not go negative.
        assert_eq!(brightness_display_fraction(3).to_bits(), 0.0_f32.to_bits());
        assert_eq!(brightness_display_fraction(0).to_bits(), 0.0_f32.to_bits());
    }
}

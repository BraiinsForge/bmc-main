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
/// Maximum movement (px) for a touch to count as a tap.
const TAP_SLOP: f32 = 12.0;

/// Whether a finished touch (`start`→`end`) should dismiss the tray. Two cases:
/// an upward swipe that begins on the panel (predominantly vertical, past the
/// threshold), or a tap below the panel (`tap-outside`).
#[must_use]
pub fn classify(start: Pt, end: Pt, panel_height: f32) -> bool {
    let dx = (end.x - start.x).abs();
    let dy = end.y - start.y; // negative = upward

    let on_panel = start.y <= panel_height;
    let upward_swipe = on_panel && (-dy) >= DISMISS_DY && (-dy) > dx;

    let below_panel = start.y > panel_height;
    let tap_outside = below_panel && dx <= TAP_SLOP && dy.abs() <= TAP_SLOP;

    upward_swipe || tap_outside
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

    // panel occupies the top 200px in these cases.
    #[test]
    fn upward_swipe_on_panel_dismisses() {
        // start inside the panel, travel up past the threshold, mostly vertical.
        assert!(classify(pt(240.0, 150.0), pt(250.0, 60.0), 200.0));
    }

    #[test]
    fn horizontal_drag_on_panel_is_not_dismiss() {
        // a slider drag: large dx, small upward dy -> the tree owns it.
        assert!(!classify(pt(100.0, 120.0), pt(300.0, 118.0), 200.0));
    }

    #[test]
    fn downward_swipe_on_panel_is_not_dismiss() {
        assert!(!classify(pt(240.0, 60.0), pt(245.0, 150.0), 200.0));
    }

    #[test]
    fn tap_below_panel_dismisses() {
        // tap-outside: starts below panel_height, negligible movement.
        assert!(classify(pt(240.0, 400.0), pt(242.0, 404.0), 200.0));
    }

    #[test]
    fn drag_below_panel_is_not_a_tap() {
        // a moving touch outside the panel is not a tap-outside.
        assert!(!classify(pt(240.0, 400.0), pt(360.0, 410.0), 200.0));
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

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Pure dismiss-gesture classification for the settings-tray, kept GPU-free
//! so the hit/gesture behaviour is unit-testable.

/// A touch point in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

/// Upward travel (px) required for a swipe-up dismiss.
const DISMISS_DY: f32 = 60.0;

/// Whether a finished touch (`start`→`end`) should dismiss the tray: an upward
/// swipe past `DISMISS_DY` that is predominantly vertical, distinct from a
/// horizontal drag across the buttons. The tray is full-screen, so every touch
/// is on it.
#[must_use]
pub fn classify(start: Pt, end: Pt) -> bool {
    let dx = (end.x - start.x).abs();
    let upward = start.y - end.y; // positive = upward
    upward >= DISMISS_DY && upward > dx
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
}

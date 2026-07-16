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

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

//! Types for interaction handling.

/// Touch event from the host.
#[derive(Debug, Clone, Copy)]
pub enum TouchEvent {
    Down {
        x: f32,
        y: f32,
    },
    /// Finger lifted. Coordinates are not included because the Wayland
    /// `wl_touch::up` event does not carry them. `InteractionState` uses
    /// the last known position from the preceding `Move` or `Down` for
    /// hit-testing the release.
    Up,
    /// Current touch stream was canceled by the host compositor.
    Cancel,
    Move {
        x: f32,
        y: f32,
    },
    /// Mouse wheel scroll event. delta_y is positive for scroll down, negative for scroll up.
    Scroll {
        x: f32,
        y: f32,
        delta_y: f32,
    },
}

impl TouchEvent {
    #[must_use]
    pub fn is_motion_like(&self) -> bool {
        matches!(self, Self::Move { .. } | Self::Scroll { .. })
    }
}

/// High-level interaction event consumed during a render frame.
///
/// Recorded by `InteractionState` when widgets consume pointer input (e.g. a
/// button click is detected). Useful for dev-tool action logs.
#[derive(Debug, Clone)]
pub enum ActionEvent {
    Click {
        key: String,
        pos: Option<(f32, f32)>,
    },
    Scroll {
        key: String,
        delta: i32,
    },
}

/// Rectangle for hit testing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Check if a point is inside the rectangle.
    #[must_use]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }

    /// Area for hit-test specificity (smaller area = more specific target).
    #[must_use]
    pub fn area(&self) -> f32 {
        self.w * self.h
    }
}

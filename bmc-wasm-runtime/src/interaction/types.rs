// Copyright (C) 2026  Braiins Systems s.r.o.

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

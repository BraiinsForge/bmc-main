// Copyright (C) 2025  Braiins Systems s.r.o.

//! Types for interaction handling.

/// Touch event from the host.
#[derive(Debug, Clone, Copy)]
pub enum TouchEvent {
    Down { x: i32, y: i32 },
    Up { x: i32, y: i32 },
    Move { x: i32, y: i32 },
}

/// Rectangle for hit testing.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[must_use]
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Check if a point is inside the rectangle.
    #[must_use]
    #[expect(clippy::cast_possible_wrap)]
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w as i32 && py < self.y + self.h as i32
    }
}

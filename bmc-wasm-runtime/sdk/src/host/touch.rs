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

/// Touch interaction result (click or drag position + element dimensions).
#[derive(Debug, Clone, Copy)]
pub struct TouchHit {
    /// Local x position (relative to element left edge)
    pub x: f32,
    /// Local y position (relative to element top edge)
    pub y: f32,
    /// Element layout width
    pub width: f32,
    /// Element layout height
    pub height: f32,
}

impl TouchHit {
    /// Horizontal fraction (0.0 = left edge, 1.0 = right edge), clamped.
    #[must_use]
    pub fn frac_x(&self) -> f32 {
        if self.width > 0.0 {
            (self.x / self.width).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::TouchHit;

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_get_touch_click(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;
        fn host_get_touch_drag(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;
    }

    impl TouchHit {
        fn from_buf(buf: &[u8; 16]) -> Self {
            Self {
                x: f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
                y: f32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
                width: f32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
                height: f32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            }
        }
    }

    /// Get the click position for an interactive canvas (one-shot, on finger-up).
    ///
    /// Returns `None` if the canvas was not clicked this frame.
    #[must_use]
    pub fn get_touch_click(key: &str) -> Option<TouchHit> {
        let mut buf = [0u8; 16];
        let clicked =
            unsafe { host_get_touch_click(key.as_ptr(), key.len() as u32, buf.as_mut_ptr()) };
        if clicked != 0 {
            Some(TouchHit::from_buf(&buf))
        } else {
            None
        }
    }

    /// Get the drag position for an interactive canvas (continuous, while finger is down).
    ///
    /// Returns `None` if the canvas is not being dragged this frame.
    #[must_use]
    pub fn get_touch_drag(key: &str) -> Option<TouchHit> {
        let mut buf = [0u8; 16];
        let dragging =
            unsafe { host_get_touch_drag(key.as_ptr(), key.len() as u32, buf.as_mut_ptr()) };
        if dragging != 0 {
            Some(TouchHit::from_buf(&buf))
        } else {
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{get_touch_click, get_touch_drag};

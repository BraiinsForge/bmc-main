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

use bmc_wasm_protocol::ButtonStyle;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_fill_rect(x: i32, y: i32, w: u32, h: u32, color: u32);
    fn host_draw_text(text_ptr: *const u8, text_len: u32, x: i32, y: i32, size: u32, color: u32);
    fn host_request_frame();
    fn host_request_frame_after(delay_ms: u32);
    fn host_button(
        key_ptr: *const u8,
        key_len: u32,
        label_ptr: *const u8,
        label_len: u32,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: u32,
    ) -> i32;

    // New tree-based API
    fn host_submit_tree(ptr: *const u8, len: u32, width: u32, height: u32);
}

/// Fill a rectangle with a solid color.
pub fn fill_rect(x: i32, y: i32, w: u32, h: u32, color: u32) {
    unsafe { host_fill_rect(x, y, w, h, color) }
}

/// Draw text at position.
pub fn draw_text(text: &[u8], x: i32, y: i32, size: u32, color: u32) {
    unsafe { host_draw_text(text.as_ptr(), text.len() as u32, x, y, size, color) }
}

/// Request next frame immediately.
pub fn request_frame() {
    unsafe { host_request_frame() }
}

/// Request next frame after delay.
pub fn request_frame_after(delay_ms: u32) {
    unsafe { host_request_frame_after(delay_ms) }
}

/// Draw a styled button with label and check for click.
/// Returns `true` the frame the button was clicked.
#[must_use]
pub fn button(
    key: &[u8],
    label: &[u8],
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    style: ButtonStyle,
) -> bool {
    unsafe {
        host_button(
            key.as_ptr(),
            key.len() as u32,
            label.as_ptr(),
            label.len() as u32,
            x,
            y,
            w,
            h,
            style as u32,
        ) != 0
    }
}

/// Submit a serialized tree for host-side layout and rendering.
pub fn submit_tree(data: &[u8], width: u32, height: u32) {
    unsafe { host_submit_tree(data.as_ptr(), data.len() as u32, width, height) }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Host function bindings and types.

/// Button style variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ButtonStyle {
    Primary = 0,
    Secondary = 1,
    Ghost = 2,
    Danger = 3,
    Tertiary = 4,
}

// Host function imports
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

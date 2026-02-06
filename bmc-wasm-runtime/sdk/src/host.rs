// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host function bindings and types.

/// Button style variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ButtonStyle {
    Primary = 0,
    Secondary = 1,
    Danger = 2,
    Tertiary = 3,
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

    // System time
    fn host_get_system_time(out_ptr: *mut u8);

    // New tree-based API
    fn host_submit_tree(ptr: *const u8, len: u32, width: u32, height: u32);
    fn host_get_button_count() -> u32;
    fn host_get_click(index: u32) -> i32;
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

// ============================================================================
// System time
// ============================================================================

/// Date-time with timezone, provided by the host.
///
/// 20-byte wire format (little-endian):
/// - `[0..8]`   `i64`  unix seconds since epoch
/// - `[8..12]`  `i32`  UTC offset in seconds
/// - `[12..14]` `u16`  year
/// - `[14]`     `u8`   month (1–12)
/// - `[15]`     `u8`   day (1–31)
/// - `[16]`     `u8`   hour (0–23)
/// - `[17]`     `u8`   minute (0–59)
/// - `[18]`     `u8`   second (0–59)
/// - `[19]`     `u8`   weekday (0=Mon … 6=Sun)
#[derive(Debug, Clone, Copy)]
pub struct SystemTime {
    pub unix_secs: i64,
    pub utc_offset_secs: i32,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 = Monday, 6 = Sunday.
    pub weekday: u8,
}

impl SystemTime {
    /// Get current system time from the host.
    pub fn now() -> Self {
        let mut buf = [0u8; 20];
        unsafe { host_get_system_time(buf.as_mut_ptr()) }
        Self {
            unix_secs: i64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]),
            utc_offset_secs: i32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            year: u16::from_le_bytes([buf[12], buf[13]]),
            month: buf[14],
            day: buf[15],
            hour: buf[16],
            minute: buf[17],
            second: buf[18],
            weekday: buf[19],
        }
    }

    /// Seconds elapsed since midnight (local time).
    pub fn seconds_since_midnight(&self) -> u32 {
        self.hour as u32 * 3_600 + self.minute as u32 * 60 + self.second as u32
    }
}

// ============================================================================
// New tree-based API
// ============================================================================

/// Submit a serialized tree for host-side layout and rendering.
pub fn submit_tree(data: &[u8], width: u32, height: u32) {
    unsafe { host_submit_tree(data.as_ptr(), data.len() as u32, width, height) }
}

/// Get number of buttons in the last submitted tree.
pub fn get_button_count() -> u32 {
    unsafe { host_get_button_count() }
}

/// Check if button at index was clicked.
pub fn get_click(index: u32) -> bool {
    unsafe { host_get_click(index) != 0 }
}

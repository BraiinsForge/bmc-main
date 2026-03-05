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
    /// Transparent background, no border. Pressed state shows a subtle rectangular fill.
    Ghost = 4,
}

/// Button size variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ButtonSize {
    Small = 0,
    Normal = 1,
    Large = 2,
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

    // Date parsing
    fn host_parse_date(str_ptr: *const u8, str_len: u32) -> i64;

    // New tree-based API
    fn host_submit_tree(ptr: *const u8, len: u32, width: u32, height: u32);
    fn host_get_button_count() -> u32;
    fn host_get_click(index: u32) -> i32;
    fn host_get_touch_click(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;
    fn host_get_touch_drag(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;

    // Icon registration
    fn host_register_icon(data_ptr: *const u8, data_len: u32) -> u32;

    // Bitmap registration
    fn host_register_bitmap(data_ptr: *const u8, data_len: u32) -> u32;

    // Image decoding (returns RGBA pixels)
    fn host_decode_image(
        data_ptr: *const u8,
        data_len: u32,
        rgba_out_ptr: *mut u8,
        rgba_out_cap: u32,
    ) -> i64;
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

// ============================================================================
// Widget size
// ============================================================================

/// Known widget size variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeVariant {
    Full,   // 1280×480
    Large,  //  638×480
    Medium, //  638×238
    Small,  //  317×238
}

/// Widget viewport dimensions and size variant.
///
/// Created from the raw `(width, height)` the host passes to `init()`.
/// Carries both the classified variant (for layout matching) and the
/// actual pixel dimensions (for `render_ui`).
#[derive(Debug, Clone, Copy)]
pub struct WidgetSize {
    pub variant: SizeVariant,
    pub width: u32,
    pub height: u32,
}

impl WidgetSize {
    #[must_use]
    pub fn from_dimensions(w: u32, h: u32) -> Self {
        let variant = match (w, h) {
            (1_280, 480) => SizeVariant::Full,
            (638, 480) => SizeVariant::Large,
            (638, 238) => SizeVariant::Medium,
            _ => SizeVariant::Small,
        };
        Self {
            variant,
            width: w,
            height: h,
        }
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
    #[must_use]
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
    #[must_use]
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
#[must_use]
pub fn get_button_count() -> u32 {
    unsafe { host_get_button_count() }
}

/// Check if button at index was clicked.
#[must_use]
pub fn get_click(index: u32) -> bool {
    unsafe { host_get_click(index) != 0 }
}

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
    fn from_buf(buf: &[u8; 16]) -> Self {
        Self {
            x: f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            y: f32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            width: f32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            height: f32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        }
    }

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

/// Get the click position for an interactive canvas (one-shot, on finger-up).
///
/// Returns `None` if the canvas was not clicked this frame.
#[must_use]
pub fn get_touch_click(key: &str) -> Option<TouchHit> {
    let mut buf = [0u8; 16];
    let clicked = unsafe { host_get_touch_click(key.as_ptr(), key.len() as u32, buf.as_mut_ptr()) };
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
    let dragging = unsafe { host_get_touch_drag(key.as_ptr(), key.len() as u32, buf.as_mut_ptr()) };
    if dragging != 0 {
        Some(TouchHit::from_buf(&buf))
    } else {
        None
    }
}

/// Register icon data with the host, returns an opaque icon ID.
#[expect(clippy::cast_possible_truncation)]
#[must_use]
pub fn register_icon(data: &[u8]) -> u16 {
    unsafe { host_register_icon(data.as_ptr(), data.len() as u32) as u16 }
}

/// Parse an ISO 8601 date string (e.g. "2026-02-13T10:15:56Z") into a unix timestamp.
///
/// Returns `None` if the string is not a valid date.
#[must_use]
pub fn parse_date(s: &str) -> Option<i64> {
    let val = unsafe { host_parse_date(s.as_ptr(), s.len() as u32) };
    if val == i64::MIN { None } else { Some(val) }
}

/// Register bitmap data (PNG bytes) with the host, returns an opaque bitmap ID.
///
/// The host decodes the image and uploads it to the GPU as a texture.
/// This is a one-time cost; subsequent renders use the cached texture.
#[expect(clippy::cast_possible_truncation)]
#[must_use]
pub fn register_bitmap(data: &[u8]) -> u16 {
    unsafe { host_register_bitmap(data.as_ptr(), data.len() as u32) as u16 }
}

/// Get the dimensions of an image (PNG, JPEG, etc.) without decoding the full pixel data.
///
/// Returns `Some((width, height))` on success, `None` on decode error.
/// This is much cheaper than [`decode_image`] as it only probes the header — no RGBA
/// buffer is allocated.
pub fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let packed =
        unsafe { host_decode_image(data.as_ptr(), data.len() as u32, core::ptr::null_mut(), 0) };
    if packed < 0 {
        return None;
    }
    let w = (packed >> 32) as u32;
    let h = (packed & 0xFFFF_FFFF) as u32;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Decode image data (PNG, JPEG, etc.) to RGBA pixels on the host.
///
/// Returns `Some((rgba_bytes, width, height))` on success, `None` on decode error.
/// The RGBA buffer is allocated in WASM memory and contains `width * height * 4` bytes.
///
/// Useful for color extraction (e.g., palette from album art) without pulling
/// an image decoder into the WASM binary.
///
/// If you only need the aspect ratio, use [`image_dimensions`] instead — it avoids
/// the large RGBA allocation and is much cheaper on fuel.
pub fn decode_image(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h) = image_dimensions(data)?;
    let needed = w * h * 4;

    // Allocate buffer and decode into it
    let mut buf = vec![0u8; needed as usize];
    let packed2 =
        unsafe { host_decode_image(data.as_ptr(), data.len() as u32, buf.as_mut_ptr(), needed) };
    if packed2 < 0 {
        return None;
    }
    Some((buf, w, h))
}

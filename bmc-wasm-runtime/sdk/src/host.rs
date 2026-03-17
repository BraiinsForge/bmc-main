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
    fn host_get_touch_click(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;
    fn host_get_touch_drag(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;

    // Icon registration
    fn host_register_icon(data_ptr: *const u8, data_len: u32) -> u32;

    // Bitmap registration
    fn host_register_bitmap(data_ptr: *const u8, data_len: u32) -> u32;
    fn host_register_bitmap_nearest(data_ptr: *const u8, data_len: u32) -> u32;

    // Mesh registration
    fn host_register_mesh(data_ptr: *const u8, data_len: u32) -> u32;

    // Audio registration and playback
    fn host_register_audio(
        data_ptr: *const u8,
        data_len: u32,
        name_ptr: *const u8,
        name_len: u32,
    ) -> u32;
    fn host_audio_play(sound_id: u32, volume: u32);
    fn host_audio_stop(sound_id: u32);

    // Image decoding (returns RGBA pixels)
    fn host_decode_image(
        data_ptr: *const u8,
        data_len: u32,
        rgba_out_ptr: *mut u8,
        rgba_out_cap: u32,
    ) -> i64;

    // Bitmap sampling (average color of a region)
    fn host_bitmap_sample(bitmap_id: u32, x: u32, y: u32, w: u32, h: u32) -> u32;

    // Random number generation (host-seeded for deterministic replay)
    fn host_random_u32() -> u32;
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

impl SizeVariant {
    /// Canonical pixel width for this variant.
    #[must_use]
    pub const fn width(self) -> u32 {
        match self {
            Self::Full => 1_280,
            Self::Large | Self::Medium => 638,
            Self::Small => 317,
        }
    }

    /// Canonical pixel height for this variant.
    #[must_use]
    pub const fn height(self) -> u32 {
        match self {
            Self::Full | Self::Large => 480,
            Self::Medium | Self::Small => 238,
        }
    }
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

/// Register mesh data (optimized binary format) with the host, returns an opaque mesh ID.
///
/// The host uploads VBO, IBO, and texture to GPU. One-time cost.
#[expect(clippy::cast_possible_truncation)]
#[must_use]
pub fn register_mesh(data: &[u8]) -> u16 {
    unsafe { host_register_mesh(data.as_ptr(), data.len() as u32) as u16 }
}

/// Register audio data (WAV/OGG/MP3 bytes) with the host, returns an opaque audio ID.
///
/// The host detects format by magic bytes, decodes to PCM once, and caches
/// decoded samples. Subsequent `audio_play` calls use the cached PCM.
#[expect(clippy::cast_possible_truncation)]
#[must_use]
pub fn register_audio(data: &[u8], name: &str) -> u16 {
    unsafe {
        host_register_audio(
            data.as_ptr(),
            data.len() as u32,
            name.as_ptr(),
            name.len() as u32,
        ) as u16
    }
}

/// Audio playback volume, in 0–100.
///
/// Values are clamped at construction, so the host never sees an
/// out-of-range byte. Use [`Volume::SILENT`] / [`Volume::FULL`] for the
/// canonical endpoints, [`Volume::new`] for arbitrary values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume(u8);

impl Volume {
    pub const SILENT: Volume = Volume(0);
    pub const FULL: Volume = Volume(100);

    /// Build a volume in 0–100. Values above 100 are clamped down to 100.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(if v > 100 { 100 } else { v })
    }
}

impl From<Volume> for u32 {
    fn from(v: Volume) -> Self {
        Self::from(v.0)
    }
}

/// Play a registered audio sample at the given [`Volume`].
///
/// Fire-and-forget: the host mixes and plays asynchronously.
/// No-op if the audio ID is invalid or if audio output is unavailable.
pub fn audio_play(sound_id: u16, volume: Volume) {
    unsafe { host_audio_play(u32::from(sound_id), u32::from(volume)) }
}

/// Stop playback of a registered audio sample.
pub fn audio_stop(sound_id: u16) {
    unsafe { host_audio_stop(u32::from(sound_id)) }
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

/// Register bitmap data with nearest-neighbor filtering (no bilinear interpolation).
///
/// Use for pixel-art assets (9-patch skins) where bilinear filtering would cause
/// color bleeding across sub-rect boundaries.
#[expect(clippy::cast_possible_truncation)]
#[must_use]
pub fn register_bitmap_nearest(data: &[u8]) -> u16 {
    unsafe { host_register_bitmap_nearest(data.as_ptr(), data.len() as u32) as u16 }
}

/// Sample the average color of a rectangular region within a registered bitmap.
///
/// Returns the average RGBA as a packed `u32` (`0xRRGGBBAA`), or `None` if the bitmap
/// is not registered or the sampled region is empty.
///
/// The region is clamped to bitmap dimensions — passing `(0, 0, u32::MAX, u32::MAX)`
/// samples the entire image.
#[must_use]
pub fn bitmap_sample(bitmap_id: u16, x: u32, y: u32, w: u32, h: u32) -> Option<u32> {
    let result = unsafe { host_bitmap_sample(u32::from(bitmap_id), x, y, w, h) };
    if result == 0 { None } else { Some(result) }
}

/// Get the dimensions of an image (PNG, JPEG, etc.) without decoding the full pixel data.
///
/// Returns `Some((width, height))` on success, `None` on decode error.
/// This is much cheaper than [`decode_image`] as it only probes the header — no RGBA
/// buffer is allocated. The host also rejects images whose decoded size exceeds
/// its configured pixel budget.
#[must_use]
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
/// the large RGBA allocation and is much cheaper on fuel. Returns `None` when the
/// host rejects the image for exceeding its decoded pixel budget or decoder
/// allocation budget.
#[must_use]
pub fn decode_image(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h) = image_dimensions(data)?;
    let needed = w.checked_mul(h)?.checked_mul(4)?;

    // Allocate buffer and decode into it
    let mut buf = vec![0u8; needed as usize];
    let packed2 =
        unsafe { host_decode_image(data.as_ptr(), data.len() as u32, buf.as_mut_ptr(), needed) };
    if packed2 < 0 {
        return None;
    }
    Some((buf, w, h))
}

/// Get a random `u32` from the host.
///
/// The host controls the RNG seed, so this is deterministic in capture/replay mode.
/// Each call advances the host's internal PRNG state.
#[must_use]
pub fn random_u32() -> u32 {
    unsafe { host_random_u32() }
}

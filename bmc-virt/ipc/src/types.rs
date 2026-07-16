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

// Domain types shared between guest and host.
// These are the public API — neither side ever deals with wire format.

/// Framebuffer dimensions and pixel format.
pub const FB_WIDTH: u32 = 480;
pub const FB_HEIGHT: u32 = 1_280;

/// Number of LEDs in the strip.
pub const LED_COUNT: usize = 10;

/// Bits per pixel of a framebuffer. 16 = RGB565; 32 = 8-bit-per-channel —
/// see `PixelFormat` for the actual byte order in the latter case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bpp(pub u32);

/// Row stride in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stride(pub u32);

/// Pixel byte order in the framebuffer payload.
///
/// The compositor's GL readback uses `glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE)`,
/// so the bytes are R, G, B, A — even though the SHM buffer is labelled
/// `Xrgb8888`/`Argb8888` (BGRA byte order) for Wayland protocol reasons. We
/// pass the actual byte order over IPC so the console can upload the texture
/// with the correct source format and avoid a per-frame CPU swizzle on the
/// guest. (`GL_BGRA_EXT` readback would avoid the mismatch but virgl's
/// macOS backend rejects it.)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelFormat {
    /// Bytes laid out B, G, R, A.
    #[default]
    Bgra8888 = 0,
    /// Bytes laid out R, G, B, A.
    Rgba8888 = 1,
}

impl PixelFormat {
    /// Wire encoding (single byte). Inverse of [`Self::from_wire`].
    #[must_use]
    pub fn as_wire(self) -> u8 {
        self as u8
    }

    /// Decode from the wire byte. Returns `None` for unknown discriminants.
    #[must_use]
    pub fn from_wire(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Bgra8888),
            1 => Some(Self::Rgba8888),
            _ => None,
        }
    }
}

/// Framebuffer metadata sent with each frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub stride: Stride,
    pub bpp: Bpp,
    pub format: PixelFormat,
    /// Display backlight brightness, normalized to 0–255 (0 = off, 255 = full).
    /// The relay reads the raw hardware value and scales it before sending.
    pub brightness: u8,
}

/// State of a single LED in the APA102 strip.
#[derive(Debug, Clone, Copy, Default)]
pub struct LedState {
    /// APA102 brightness (5-bit, 0–31).
    pub brightness: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// LED strip update.
#[derive(Debug, Clone)]
pub struct LedUpdate {
    pub seq: u64,
    pub leds: [LedState; LED_COUNT],
}

/// Log entry source.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    BmcLog = 0,
    Syslog = 1,
    Dmesg = 2,
    RelayLog = 3,
}

/// All log sources in order, for iterating in UI tabs.
pub const ALL_LOG_SOURCES: &[LogSource] = &[
    LogSource::BmcLog,
    LogSource::Syslog,
    LogSource::Dmesg,
    LogSource::RelayLog,
];

impl LogSource {
    /// Display name for UI tabs and log tailer identification.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::BmcLog => "bmc.log",
            Self::Syslog => "syslog",
            Self::Dmesg => "dmesg",
            Self::RelayLog => "relay",
        }
    }

    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::BmcLog),
            1 => Some(Self::Syslog),
            2 => Some(Self::Dmesg),
            3 => Some(Self::RelayLog),
            _ => None,
        }
    }
}

/// A single input event from the host to the guest.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    TouchDown { x: u16, y: u16 },
    TouchMove { x: u16, y: u16 },
    TouchUp,
    ButtonPress { button: u8, data: u8 },
}

/// Button IDs for `InputEvent::ButtonPress`.
pub mod buttons {
    pub const WIFI_TOGGLE: u8 = 0;
    /// Set LED effect preset. `data` byte = preset index (0–5).
    pub const LED_EFFECT_SET: u8 = 1;
    /// Clear LED effect (turn off test override).
    pub const LED_EFFECT_CLEAR: u8 = 2;
    /// Set volume override. `data` byte = volume (0–100).
    pub const VOLUME_SET: u8 = 3;
    /// Clear volume override — revert to app's configured volume.
    pub const VOLUME_RESET: u8 = 4;
}

/// Severity level for relay notifications.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyLevel {
    Info = 0,
    Warning = 1,
    Error = 2,
}

impl NotifyLevel {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Info),
            1 => Some(Self::Warning),
            2 => Some(Self::Error),
            _ => None,
        }
    }
}

/// Availability state for a relay feature.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    Waiting = 0,
    Ready = 1,
    Unavailable = 2,
}

impl FeatureState {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Waiting),
            1 => Some(Self::Ready),
            2 => Some(Self::Unavailable),
            _ => None,
        }
    }
}

/// Message from guest → host.
#[derive(Debug)]
pub enum GuestMessage {
    /// New framebuffer frame. `data` is raw pixel bytes.
    Frame { header: FrameHeader, data: Vec<u8> },
    /// LED strip state update.
    Leds(LedUpdate),
    /// Log line from a source.
    Log { source: LogSource, line: String },
    /// Active LED effect preset index (0xFF = off).
    ActiveEffect(u8),
    /// Display capture availability.
    CaptureStatus {
        state: FeatureState,
        reason: Option<String>,
    },
    /// Reply to a host Ping — used for connection liveness detection.
    Pong,
    /// Current volume state: app's configured volume + optional console override.
    VolumeLevel {
        /// The app's own volume setting (from gRPC GetSoundVolumeSettings).
        app: u8,
        /// Console override (None = using app's value, Some = override active).
        override_vol: Option<u8>,
    },
    /// Web API / control-path availability.
    ControlsStatus {
        state: FeatureState,
        reason: Option<String>,
    },
    /// Notification from the relay — either a response to a failed action
    /// or an unsolicited push message.
    Notify { level: NotifyLevel, message: String },
}

/// Message from host → guest.
#[derive(Debug)]
pub enum HostMessage {
    Input(InputEvent),
    /// Run a shell command on the guest (`sh -c <cmd>`).
    RunCommand(String),
    /// Simulate GPIO reset button press/release via netlink uevent injection.
    GpioButton {
        pressed: bool,
    },
    /// Connection liveness probe — guest must reply with `GuestMessage::Pong`.
    Ping,
}

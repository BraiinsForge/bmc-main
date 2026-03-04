// Copyright (C) 2026  Braiins Systems s.r.o.

//! Icon assets for the media control widget.
//!
//! Both solid and outline styles available for transport controls.
//! Shared icons (repeat, shuffle, devices) at root level.

use bmc_wasm_sdk::{Icon, include_icon};

/// Solid (filled) icon variants — for active/primary UI elements.
#[allow(dead_code)]
pub mod solid {
    use super::{Icon, include_icon};

    pub const PLAY: Icon = include_icon!("assets/solid/play.svg");
    pub const PAUSE: Icon = include_icon!("assets/solid/pause.svg");
    pub const STOP: Icon = include_icon!("assets/solid/stop.svg");
    pub const SKIP_BACK: Icon = include_icon!("assets/solid/skip-back.svg");
    pub const SKIP_FORWARD: Icon = include_icon!("assets/solid/skip-forward.svg");
    pub const VOLUME_UP: Icon = include_icon!("assets/solid/volume-up.svg");
    pub const VOLUME_DOWN: Icon = include_icon!("assets/solid/volume-down.svg");
    pub const VOLUME_MUTE: Icon = include_icon!("assets/solid/volume-mute.svg");
}

/// Outline (stroked) icon variants — for secondary/inactive UI elements.
#[allow(dead_code)]
pub mod outline {
    use super::{Icon, include_icon};

    pub const PLAY: Icon = include_icon!("assets/outline/play.svg");
    pub const PAUSE: Icon = include_icon!("assets/outline/pause.svg");
    pub const STOP: Icon = include_icon!("assets/outline/stop-outline.svg");
    pub const SKIP_BACK: Icon = include_icon!("assets/outline/skip-back.svg");
    pub const SKIP_FORWARD: Icon = include_icon!("assets/outline/skip-forward.svg");
    pub const VOLUME_UP: Icon = include_icon!("assets/outline/volume-up.svg");
    pub const VOLUME_DOWN: Icon = include_icon!("assets/outline/volume-down.svg");
    pub const VOLUME_MUTE: Icon = include_icon!("assets/outline/volume-mute.svg");
}

// ── Protocol icons ──────────────────────────────────────────────

#[allow(dead_code)]
pub const PROTO_GOOGLE_CAST: Icon = include_icon!("assets/proto-google-cast.svg");
#[allow(dead_code)]
pub const PROTO_DLNA: Icon = include_icon!("assets/proto-dlna.svg");
#[allow(dead_code)]
pub const PROTO_KODI: Icon = include_icon!("assets/proto-kodi.svg");
// ── Shared icons (no solid/outline distinction) ──────────────────

pub const MUSIC: Icon = include_icon!("assets/music.svg");
pub const VIDEO: Icon = include_icon!("assets/video.svg");
#[allow(dead_code)]
pub const REPEAT: Icon = include_icon!("assets/repeat.svg");
#[allow(dead_code)]
pub const REPEAT_ONE: Icon = include_icon!("assets/repeat-one.svg");
#[allow(dead_code)]
pub const SHUFFLE: Icon = include_icon!("assets/shuffle.svg");
#[allow(dead_code)]
pub const DEVICES: Icon = include_icon!("assets/devices.svg");
#[allow(dead_code)]
pub const DEVICES_APPS: Icon = include_icon!("assets/devices-apps.svg");

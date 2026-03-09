// Copyright (C) 2026  Braiins Systems s.r.o.

//! Icon assets for the media control widget.

use bmc_wasm_sdk::{Icon, include_icon};

/// Solid (filled) icon variants — for active/primary UI elements.
pub const PLAY: Icon = include_icon!("assets/icons/solid/play.svg");
pub const PAUSE: Icon = include_icon!("assets/icons/solid/pause.svg");
// pub const STOP: Icon = include_icon!("assets/icons/solid/stop.svg");
pub const SKIP_BACK: Icon = include_icon!("assets/icons/solid/skip-back.svg");
pub const SKIP_FORWARD: Icon = include_icon!("assets/icons/solid/skip-forward.svg");
pub const VOLUME_UP: Icon = include_icon!("assets/icons/solid/volume-up.svg");
pub const VOLUME_DOWN: Icon = include_icon!("assets/icons/solid/volume-down.svg");
pub const VOLUME_MUTE: Icon = include_icon!("assets/icons/solid/volume-mute.svg");

// ── Protocol icons

pub const PROTO_GOOGLE_CAST: Icon = include_icon!("assets/icons/proto-google-cast.svg");
pub const PROTO_DLNA: Icon = include_icon!("assets/icons/proto-dlna.svg");
pub const PROTO_KODI: Icon = include_icon!("assets/icons/proto-kodi.svg");
pub const PROTO_JELLYFIN: Icon = include_icon!("assets/icons/proto-jellyfin.svg");
pub const PROTO_EMBY: Icon = include_icon!("assets/icons/proto-emby.svg");
pub const PROTO_MPD: Icon = include_icon!("assets/icons/proto-mpd.svg");

// ── Shared icons

pub const MUSIC: Icon = include_icon!("assets/icons/music.svg");
pub const VIDEO: Icon = include_icon!("assets/icons/video.svg");
// pub const REPEAT: Icon = include_icon!("assets/icons/repeat.svg");
// pub const REPEAT_ONE: Icon = include_icon!("assets/icons/repeat-one.svg");
// pub const SHUFFLE: Icon = include_icon!("assets/icons/shuffle.svg");
// pub const DEVICES: Icon = include_icon!("assets/icons/devices.svg");
pub const DEVICES_APPS: Icon = include_icon!("assets/icons/devices-apps.svg");
pub const SKIN: Icon = include_icon!("assets/icons/skin.svg");

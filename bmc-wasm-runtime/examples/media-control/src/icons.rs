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

//! Icon assets for the media control widget.

use bmc_wasm_sdk::{Svg, include_svg};

/// Solid (filled) icon variants — for active/primary UI elements.
pub const PLAY: Svg = include_svg!("assets/icons/solid/play.svg");
pub const PAUSE: Svg = include_svg!("assets/icons/solid/pause.svg");
// pub const STOP: Svg = include_svg!("assets/icons/solid/stop.svg");
pub const SKIP_BACK: Svg = include_svg!("assets/icons/solid/skip-back.svg");
pub const SKIP_FORWARD: Svg = include_svg!("assets/icons/solid/skip-forward.svg");
pub const VOLUME_UP: Svg = include_svg!("assets/icons/solid/volume-up.svg");
pub const VOLUME_DOWN: Svg = include_svg!("assets/icons/solid/volume-down.svg");
pub const VOLUME_MUTE: Svg = include_svg!("assets/icons/solid/volume-mute.svg");

// ── Protocol icons

pub const PROTO_GOOGLE_CAST: Svg = include_svg!("assets/icons/proto-google-cast.svg");
pub const PROTO_DLNA: Svg = include_svg!("assets/icons/proto-dlna.svg");
pub const PROTO_KODI: Svg = include_svg!("assets/icons/proto-kodi.svg");
pub const PROTO_JELLYFIN: Svg = include_svg!("assets/icons/proto-jellyfin.svg");
pub const PROTO_EMBY: Svg = include_svg!("assets/icons/proto-emby.svg");
pub const PROTO_MPD: Svg = include_svg!("assets/icons/proto-mpd.svg");

// ── Shared icons

pub const MUSIC: Svg = include_svg!("assets/icons/music.svg");
pub const VIDEO: Svg = include_svg!("assets/icons/video.svg");
// pub const REPEAT: Svg = include_svg!("assets/icons/repeat.svg");
// pub const REPEAT_ONE: Svg = include_svg!("assets/icons/repeat-one.svg");
// pub const SHUFFLE: Svg = include_svg!("assets/icons/shuffle.svg");
// pub const DEVICES: Svg = include_svg!("assets/icons/devices.svg");
pub const DEVICES_APPS: Svg = include_svg!("assets/icons/devices-apps.svg");
pub const SKIN: Svg = include_svg!("assets/icons/skin.svg");

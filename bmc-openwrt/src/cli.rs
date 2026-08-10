// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::net::SocketAddr;
use std::path::PathBuf;

pub use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    #[clap(long)]
    pub log_to_file: bool,

    /// Path to widget directories (can be specified multiple times)
    #[clap(long = "widgets-path")]
    pub widgets_paths: Option<Vec<PathBuf>>,

    /// Run compositor without EGL/DRM — Wayland protocol loop only.
    /// Required for rr time-travel debugging (rr hides /dev/dri devices).
    #[clap(long)]
    pub headless_compositor: bool,

    /// HTTP server bind address (default 0.0.0.0).
    /// The port defaults to 80 on the BMC100, and to 81 on the BMM/BFM units
    /// where boser's web UI already holds port 80.
    #[clap(long)]
    pub address: Option<SocketAddr>,

    /// Path to frontend web files (default: build-time BMC_WEB_FRONTEND_DIR or /run/current-profile/www/bmc)
    #[clap(long = "www-path")]
    pub www_path: Option<PathBuf>,

    /// Hardware profile override for development: BMC100|BMM100|BMM101|BFM100.
    /// Defaults to `auto`, which uses the platform from /etc/bos_platform.
    #[clap(long = "hardware-profile", default_value = "auto")]
    pub hardware_profile: String,
}

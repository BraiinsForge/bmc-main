// Copyright (C) 2025  Braiins Systems s.r.o.

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

    /// HTTP server bind address (default: 0.0.0.0:80)
    #[clap(long)]
    pub address: Option<SocketAddr>,

    /// Path to frontend web files (default: build-time BMC_WEB_FRONTEND_DIR or /run/current-profile/www/bmc)
    #[clap(long = "www-path")]
    pub www_path: Option<PathBuf>,
}

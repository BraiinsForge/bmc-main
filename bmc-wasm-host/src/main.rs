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

use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use bmc_wasm_host::startup::{StartupDecision, prepare_listener};
use bmc_wasm_thin_protocol::{default_socket_path, derive_log_path};
use clap::Parser;

#[cfg(feature = "profiling")]
#[global_allocator]
static GLOBAL_ALLOCATOR: bmc_render::profile::DeallocationTrackingAllocator =
    bmc_render::profile::DeallocationTrackingAllocator;

// Fixed Deck-maximum staging FBO size. This intentionally stays a process-startup
// constant for now; BMM/BFM viewports fit inside it and render into a sub-region.
const STAGING_MAX_WIDTH: u32 = 1280;
const STAGING_MAX_HEIGHT: u32 = 480;

#[derive(Parser, Debug)]
#[command(about = "bmc-wasm-host - multi-widget WASM daemon")]
struct Args {
    #[arg(long, value_name = "PATH")]
    host_socket: Option<PathBuf>,

    #[arg(long, hide = true, value_name = "FD")]
    release_lock_fd: Option<i32>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let socket_path = args.host_socket.unwrap_or_else(default_socket_path);
    let log_path = derive_log_path(&socket_path);
    bmc_wasm_host::logging::init(&log_path)
        .with_context(|| format!("initialize host file logging at {}", log_path.display()))?;

    tracing::info!(
        socket = %socket_path.display(),
        release_lock_fd = ?args.release_lock_fd,
        "starting bmc-wasm-host"
    );

    let (listener, release_lock) = match prepare_listener(&socket_path, args.release_lock_fd)? {
        StartupDecision::Run {
            listener,
            release_lock,
        } => (listener, release_lock),
        StartupDecision::AnotherHostAlive => {
            tracing::info!("another bmc-wasm-host is already accepting connections");
            std::process::exit(1);
        }
    };
    tracing::info!(socket = %socket_path.display(), "listening");

    let (mut shared, mut renderer) =
        bmc_wasm_host::host::SharedHost::init(STAGING_MAX_WIDTH, STAGING_MAX_HEIGHT)?;

    if let Some(lock) = release_lock {
        lock.release()?;
        tracing::info!("released host readiness lock");
    }

    let exit = bmc_wasm_host::main_loop::run(&mut shared, &mut renderer, &listener);
    if let Err(e) = exit {
        tracing::error!(?e, "host exited with FatalError");
        std::process::exit(1);
    }
    tracing::info!("bmc-wasm-host exiting cleanly");
    Ok(())
}

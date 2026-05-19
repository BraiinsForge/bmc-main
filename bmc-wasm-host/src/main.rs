// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::PathBuf;

use anyhow::Result;
use bmc_wasm_host::startup::{StartupDecision, prepare_listener};
use bmc_wasm_thin_protocol::default_socket_path;
use clap::Parser;

// Device display maximum — the staging FBO is sized to this so any slot's surface fits without
// reallocation. Match the Braiins Deck's physical resolution; widgets render at smaller sizes
// and the scratch is reused across them.
const DECK_DISPLAY_MAX_WIDTH: u32 = 1280;
const DECK_DISPLAY_MAX_HEIGHT: u32 = 480;

#[derive(Parser, Debug)]
#[command(about = "bmc-wasm-host - multi-widget WASM daemon")]
struct Args {
    #[arg(long, value_name = "PATH")]
    host_socket: Option<PathBuf>,

    #[arg(long, hide = true, value_name = "FD")]
    release_lock_fd: Option<i32>,
}

fn main() -> Result<()> {
    bmc_wasm_host::logging::init();

    let args = Args::parse();
    let socket_path = args.host_socket.unwrap_or_else(default_socket_path);
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
        bmc_wasm_host::host::SharedHost::init(DECK_DISPLAY_MAX_WIDTH, DECK_DISPLAY_MAX_HEIGHT)?;

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

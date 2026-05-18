// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::PathBuf;

use anyhow::Result;
use bmc_wasm_host::control::{DEFAULT_SOCKET_PATH, ListenSocket};
use clap::Parser;

// Device display maximum — the staging FBO is sized to this so any slot's surface fits without
// reallocation. Match the Braiins Deck's physical resolution; widgets render at smaller sizes
// and the scratch is reused across them.
const DECK_DISPLAY_MAX_WIDTH: u32 = 1280;
const DECK_DISPLAY_MAX_HEIGHT: u32 = 480;

#[derive(Parser, Debug)]
#[command(about = "bmc-wasm-host — multi-widget WASM daemon (Stage 5)")]
struct Args {
    #[arg(long, value_name = "PATH")]
    host_socket: Option<PathBuf>,
}

fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let socket_path = args
        .host_socket
        .unwrap_or_else(|| DEFAULT_SOCKET_PATH.into());

    let listener = ListenSocket::bind(&socket_path)
        .map_err(|e| anyhow::anyhow!("bind {}: {e}", socket_path.display()))?;
    tracing::info!(socket = %socket_path.display(), "listening");

    let mut shared =
        bmc_wasm_host::host::SharedHost::init(DECK_DISPLAY_MAX_WIDTH, DECK_DISPLAY_MAX_HEIGHT)?;
    let exit = bmc_wasm_host::main_loop::run(&mut shared, &listener);
    if let Err(e) = exit {
        tracing::error!(?e, "host exited with FatalError");
        std::process::exit(1);
    }
    Ok(())
}

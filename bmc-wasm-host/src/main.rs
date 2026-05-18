// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use bmc_wasm_host::control::{DEFAULT_SOCKET_PATH, ListenSocket, try_handshake};
use clap::Parser;

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

    for incoming in listener.as_listener().incoming() {
        match incoming {
            Ok(client) => {
                tracing::info!("accepted connection");
                match try_handshake(&client) {
                    Ok(msg) => {
                        tracing::info!(?msg, "handshake parsed; replying Err (stage-5 stub)");
                    }
                    Err(e) => tracing::warn!(?e, "handshake failed"),
                }
            }
            Err(e) => {
                tracing::warn!(?e, "accept error, sleeping briefly");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Ok(())
}

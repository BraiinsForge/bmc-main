// Copyright (C) 2026  Braiins Systems s.r.o.

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "bmc-wasm-host — multi-widget WASM daemon (Stage 5)")]
struct Args {
    /// Override the control socket path. Default is `/run/bmc/wasm-host-v1.sock`.
    #[arg(long, value_name = "PATH")]
    host_socket: Option<std::path::PathBuf>,
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "will propagate errors once host logic is added"
)]
fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    tracing::info!(?args.host_socket, "bmc-wasm-host stage-5 scaffold");
    Ok(())
}

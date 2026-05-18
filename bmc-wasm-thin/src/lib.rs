// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod args;
pub mod host_client;
pub mod logging;
pub mod paths;
pub mod signal;
pub mod spawn;
pub mod wayland_fd;

use anyhow::Result;

use crate::args::Config;

#[expect(
    clippy::needless_pass_by_value,
    reason = "ownership transfer is intentional; later tasks move config fields into idle loop"
)]
pub fn run(config: Config) -> Result<()> {
    tracing::info!(
        wasm = %config.wasm.display(),
        host_socket = %config.host_socket.display(),
        host_bin = %config.host_bin.display(),
        lockfile = %config.lockfile.display(),
        host_wait_ms = config.host_wait.as_millis(),
        ack_wait_ms = config.ack_wait.as_millis(),
        "starting bmc-wasm-thin"
    );
    let wayland = wayland_fd::connect_from_env()?;
    tracing::info!("connected to Wayland; connecting to wasm host");
    let control = spawn::connect_or_spawn(&config)?;
    tracing::info!("connected to wasm host; sending load request");
    let control =
        host_client::send_load_and_wait_ack(control, &config.wasm, wayland, config.ack_wait)?;
    tracing::info!("host acknowledged widget load; idling as lifetime witness");
    let idle_exit = host_client::idle_until_exit(control)?;
    tracing::info!(?idle_exit, "bmc-wasm-thin exiting");
    Ok(())
}

// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod args;
pub mod host_client;
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
    let wayland = wayland_fd::connect_from_env()?;
    let control = spawn::connect_or_spawn(&config)?;
    let control =
        host_client::send_load_and_wait_ack(control, &config.wasm, wayland, config.ack_wait)?;
    host_client::idle_until_exit(control).map(|_| ())
}

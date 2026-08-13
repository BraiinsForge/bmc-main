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

pub mod args;
pub mod host_client;
pub mod logging;
pub mod ownership;
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
        asset_root = ?config.asset_root,
        host_socket = %config.host_socket.display(),
        host_bin = %config.host_bin.display(),
        lockfile = %config.lockfile.display(),
        owner_record = %config.owner_record.display(),
        host_wait_ms = config.host_wait.as_millis(),
        ack_wait_ms = config.ack_wait.as_millis(),
        "starting bmc-wasm-thin"
    );
    let wayland = wayland_fd::connect_from_env()?;
    tracing::info!("connected to Wayland; connecting to wasm host");
    let control = spawn::connect_or_spawn(&config)?;
    tracing::info!("connected to wasm host; sending load request");
    let control = host_client::send_load_and_wait_ack(
        control,
        &config.wasm,
        config.asset_root.as_deref(),
        wayland,
        config.ack_wait,
    )?;
    tracing::info!("host acknowledged widget load; idling as lifetime witness");
    let idle_exit = host_client::idle_until_exit(control)?;
    tracing::info!(?idle_exit, "bmc-wasm-thin exiting");
    Ok(())
}

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

use std::sync::atomic::{AtomicBool, Ordering};

use bmc::shutdown::UPGRADE_HOLD;
use bmc_support::SupportArchiveFormat;
use tokio::task;
use tracing::info;

use crate::{signal, sys};

const REBOOT_COMMAND: &str = "reboot";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Sys error: {0}")]
    Sys(#[from] sys::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Support archive error: `{0}`")]
    SupportArchive(String),
}

pub async fn call_command<T>(command_name: T, args: &[T]) -> Result<(), Error>
where
    T: ToString + Sync + Send,
{
    sys::call_command_to_string(command_name, args)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

pub async fn system_reboot() -> Result<(), Error> {
    call_command(REBOOT_COMMAND, &[]).await
}

// During a system upgrade the shutdown of the Axum web server is delayed by
// sleeping, so the server survives sysupgrade's SIGTERM long enough to
// deliver the last progress events to clients. Outside an upgrade the server
// shuts down immediately.
pub async fn handle_graceful_shutdown(upgrade_in_progress: &AtomicBool) {
    let signal = signal::wait_for_first_signal(signal::SHUTDOWN_SIGNALS).await;

    if !upgrade_in_progress.load(Ordering::SeqCst) {
        info!("{:?} signal received. Shutting down", signal);
        return;
    }

    info!(
        "{:?} signal received. Waiting for {:?}s, then shutting down",
        signal,
        UPGRADE_HOLD.as_secs()
    );
    tokio::time::sleep(UPGRADE_HOLD).await;
    info!("Timeout reached. Forcefully shutting down...");
}

pub async fn get_support_archive(format: SupportArchiveFormat) -> Result<Vec<u8>, Error> {
    let result = task::spawn_blocking(move || {
        let mut buf = Vec::new();
        bmc_support::collect(&mut buf, format, false)?;
        Ok::<_, anyhow::Error>(buf)
    })
    .await;

    match result {
        Ok(Ok(buf)) => Ok(buf),
        // JoinError
        Err(err) => Err(Error::SupportArchive(err.to_string())),
        // anyhow::Error
        Ok(Err(err)) => Err(Error::SupportArchive(err.to_string())),
    }
}

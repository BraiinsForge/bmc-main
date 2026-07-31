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

use std::{
    net::IpAddr,
    process::{Output, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use bmc::shutdown::UPGRADE_HOLD;
use bmc::utils::read_to_string;
use bmc_support::SupportArchiveFormat;
use get_if_addrs::IfAddr;
use tokio::{io::AsyncWriteExt, process::Command, task};
use tracing::{debug, info};

use crate::{signal, sys};

const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";
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

pub async fn call_command_to_string<T>(command_name: T, args: &[T]) -> Result<String, Error>
where
    T: ToString + Sync + Send,
{
    sys::call_command_to_string(command_name, args)
        .await
        .map_err(Into::into)
}

pub async fn call_command_stdin<T>(command_name: T, args: &[T], stdin: T) -> Result<Output, Error>
where
    T: ToString + Sync + Send,
{
    let mut child = Command::new(command_name.to_string())
        .args(args.iter().map(T::to_string))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    if let Some(child_stdin) = child.stdin.as_mut() {
        child_stdin.write_all(stdin.to_string().as_bytes()).await?;
        child_stdin.flush().await?;
    }

    Ok(child.wait_with_output().await?)
}

pub async fn get_hostname() -> Option<String> {
    read_to_string(HOSTNAME_PATH).await
}

pub fn get_ip_address() -> Option<IpAddr> {
    let all_interfaces = get_if_addrs::get_if_addrs().ok()?;
    let default_interface_opt = all_interfaces.iter().find(|e| !e.is_loopback());
    // We want to stick to standard IP address type (std::net::IpAddr)
    let ip_address_opt = default_interface_opt.map(|iface| match &iface.addr {
        IfAddr::V4(addr) => addr.ip.into(),
        IfAddr::V6(addr) => addr.ip.into(),
    });
    debug!(
        "All interfaces: {:?}, selected default: {:?}, extracted IP address: {:?}",
        all_interfaces, default_interface_opt, ip_address_opt
    );
    ip_address_opt
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

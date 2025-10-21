// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    net::IpAddr,
    process::{Output, Stdio},
    time::Duration,
};

use bmc::utils::read_to_string;
use get_if_addrs::IfAddr;
use tokio::{io::AsyncWriteExt, process::Command};
use tracing::{debug, info};

use crate::{signal, sys};

const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";
const REBOOT_COMMAND: &str = "reboot";
const SHUTDOWN_SLEEP_DURATION: Duration = Duration::from_secs(5);

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Sys error: {0}")]
    Sys(#[from] sys::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
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

// HACK: this function only delays the shutdown by sleeping
// It is necessary when doing a system upgrade to delay the shutdown of Axum web server.
pub async fn handle_graceful_shutdown() {
    let signal = signal::wait_for_first_signal(signal::SHUTDOWN_SIGNALS).await;

    info!(
        "{:?} signal received. Waiting for {:?}s, then shutting down",
        signal,
        SHUTDOWN_SLEEP_DURATION.as_secs()
    );
    tokio::time::sleep(SHUTDOWN_SLEEP_DURATION).await;
    info!("Timeout reached. Forcefully shutting down...");
}

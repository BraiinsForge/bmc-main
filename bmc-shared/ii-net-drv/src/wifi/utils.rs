// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use std::path::{Path, PathBuf};

use anyhow::{Error, Result, anyhow};
use bstr::ByteSlice;
use log::debug;
use strum::{Display, EnumString};
use tokio::process::Command;

#[derive(Debug)]
pub struct WifiUtils;

impl WifiUtils {
    pub async fn get_device_by_syspath(syspath: &str) -> Result<String> {
        tokio::fs::read_dir(Path::new(syspath).join("net"))
            .await
            .map_err(|e| anyhow!("Could not access `net` under {syspath}: {e}"))?
            .next_entry()
            .await
            .map_err(|e| anyhow!("Could not access entries under {syspath}/net: {e}"))?
            .ok_or(anyhow!("No wifi device in specified syspath: {syspath}"))?
            .file_name()
            .into_string()
            .map_err(|e| Error::msg(format!("{e:?}")))
    }

    pub async fn get_phy_path_by_syspath(syspath: &str) -> Result<PathBuf> {
        Ok(tokio::fs::read_dir(Path::new(syspath).join("ieee80211"))
           .await
           .map_err(|e| anyhow!("Could not access `ieee80211` under {syspath}: {e}"))?
           .next_entry()
           .await
           .map_err(|e| anyhow!("Could not access entries under {syspath}/ieee80211: {e}"))?
           .ok_or(anyhow!("No phy device in specified syspath: {syspath}"))?
           .path())
    }
}

#[derive(Debug)]
pub struct CommandUtils;

impl CommandUtils {
    const DEFAULT_DELAY: u64 = 20;

    async fn call_command_to_string(
        command_name: &str,
        args: &[&str],
        timeout: u64,
    ) -> Result<String> {
        let mut command = Command::new(command_name);
        for arg in args {
            command.arg(arg);
        }

        tokio::time::timeout(tokio::time::Duration::from_secs(timeout), command.output())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for {command_name} result!"))?
            .map(|res| {
                debug!(
                    "Command '{} {:?}' returned:{:?}\nstdout:\n{:?}\nstderr:\n{:?}\n",
                    command_name,
                    args,
                    res.status.code(),
                    res.stdout.to_str(),
                    res.stderr.to_str()
                );

                res.status
                    .success()
                    .then(|| String::from_utf8_lossy(&res.stdout).to_string())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Command {command_name} returned error {:?}",
                            res.status.code()
                        )
                    })
            })?
    }

    pub async fn call_iw_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/usr/sbin/iw", args, Self::DEFAULT_DELAY).await
    }

    pub async fn call_ifconfig_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/sbin/ifconfig", args, Self::DEFAULT_DELAY).await
    }

    pub async fn call_ubus_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/bin/ubus", args, Self::DEFAULT_DELAY).await
    }

    pub async fn call_wifi_cmd(args: &[&str]) -> Result<String> {
        Self::call_command_to_string("/sbin/wifi", args, Self::DEFAULT_DELAY).await
    }
}

#[derive(Display, EnumString, Debug)]
pub enum WifiCommand {
    #[strum(serialize = "up")]
    Up,
    #[strum(serialize = "down")]
    Down,
    #[strum(serialize = "reload")]
    Reload,
    #[strum(serialize = "config")]
    Config,
}

impl WifiCommand {
    pub async fn restart() -> Result<()> {
        WifiCommand::down().await?;
        WifiCommand::up().await
    }

    pub async fn up() -> Result<()> {
        Self::run(Self::Up).await
    }

    pub async fn down() -> Result<()> {
        Self::run(Self::Down).await
    }

    pub async fn reload() -> Result<()> {
        Self::run(Self::Reload).await
    }

    pub async fn config() -> Result<()> {
        Self::run(Self::Config).await
    }

    async fn run(command: WifiCommand) -> Result<()> {
        _ = CommandUtils::call_wifi_cmd(&[&command.to_string()]).await?;
        Ok(())
    }
}

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

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser;

use crate::paths::{derive_lockfile_path, derive_owner_record_path};

pub const DEFAULT_HOST_WAIT: Duration = Duration::from_secs(10);
pub const DEFAULT_ACK_WAIT: Duration = Duration::from_secs(10);

#[derive(Parser, Debug, Clone)]
#[command(about = "bmc-wasm-thin - per-widget WASM wrapper")]
pub struct RawArgs {
    #[arg(long)]
    pub wasm: PathBuf,

    #[arg(long, value_name = "PATH")]
    pub host_socket: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    pub host_bin: Option<PathBuf>,

    #[arg(long = "host-wait-ms", value_name = "MS")]
    pub host_wait_ms: Option<u64>,

    #[arg(long = "ack-wait-ms", value_name = "MS")]
    pub ack_wait_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub wasm: PathBuf,
    pub host_socket: PathBuf,
    pub lockfile: PathBuf,
    pub owner_record: PathBuf,
    pub host_bin: PathBuf,
    pub host_wait: Duration,
    pub ack_wait: Duration,
}

impl Config {
    pub fn from_raw(raw: RawArgs) -> Result<Self> {
        let env_overrides = [
            env::var("BMC_WASM_HOST_WAIT_MS")
                .ok()
                .map(|v| ("BMC_WASM_HOST_WAIT_MS", v)),
            env::var("BMC_WASM_HOST_ACK_WAIT_MS")
                .ok()
                .map(|v| ("BMC_WASM_HOST_ACK_WAIT_MS", v)),
        ];
        let env_overrides: Vec<(&str, String)> = env_overrides.into_iter().flatten().collect();
        Self::from_raw_with_env(raw, &env_overrides)
    }

    pub fn from_raw_with_env(raw: RawArgs, env_overrides: &[(&str, String)]) -> Result<Self> {
        let lookup = |key: &str| {
            env_overrides
                .iter()
                .find_map(|(k, v)| (*k == key).then_some(v.clone()))
        };
        let host_socket = raw
            .host_socket
            .unwrap_or_else(bmc_wasm_thin_protocol::default_socket_path);
        let default_socket = bmc_wasm_thin_protocol::default_socket_path();
        let lockfile = if host_socket == default_socket {
            bmc_wasm_thin_protocol::default_lockfile_path()
        } else {
            derive_lockfile_path(&host_socket)
        };
        let owner_record = if host_socket == default_socket {
            bmc_wasm_thin_protocol::default_owner_record_path()
        } else {
            derive_owner_record_path(&host_socket)
        };
        let host_wait = parse_duration_override(
            "BMC_WASM_HOST_WAIT_MS",
            raw.host_wait_ms,
            lookup("BMC_WASM_HOST_WAIT_MS"),
            DEFAULT_HOST_WAIT,
        )?;
        let ack_wait = parse_duration_override(
            "BMC_WASM_HOST_ACK_WAIT_MS",
            raw.ack_wait_ms,
            lookup("BMC_WASM_HOST_ACK_WAIT_MS"),
            DEFAULT_ACK_WAIT,
        )?;
        let host_bin = raw.host_bin.unwrap_or_else(default_host_bin);
        Ok(Self {
            wasm: raw.wasm,
            host_socket,
            lockfile,
            owner_record,
            host_bin,
            host_wait,
            ack_wait,
        })
    }
}

fn parse_duration_override(
    name: &str,
    cli_ms: Option<u64>,
    env_ms: Option<String>,
    default: Duration,
) -> Result<Duration> {
    if let Some(ms) = cli_ms {
        return Ok(Duration::from_millis(ms));
    }
    if let Some(value) = env_ms {
        let ms = value
            .parse::<u64>()
            .with_context(|| format!("{name} must be an integer number of milliseconds"))?;
        return Ok(Duration::from_millis(ms));
    }
    Ok(default)
}

fn default_host_bin() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("bmc-wasm-host")))
        .unwrap_or_else(|| PathBuf::from("bmc-wasm-host"))
}

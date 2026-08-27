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

use std::io;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::args::Config;

const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(25);

pub fn connect(config: &Config) -> Result<UnixStream> {
    let deadline = Instant::now()
        .checked_add(config.host_wait)
        .context("host-wait duration exceeds the monotonic clock range")?;
    loop {
        match UnixStream::connect(&config.host_socket) {
            Ok(stream) => return Ok(stream),
            Err(error) if is_host_pending(&error) => {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    bail!(
                        "timed out waiting for bmc-wasm-host on {}: {error}",
                        config.host_socket.display()
                    );
                };
                std::thread::sleep(CONNECT_RETRY_INTERVAL.min(remaining));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("connect {}", config.host_socket.display()));
            }
        }
    }
}

fn is_host_pending(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ECONNREFUSED)
    )
}

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

use std::io::Read as _;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

use anyhow::{Context, Result};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::low_level::pipe;

pub struct SignalPipe {
    read: UnixStream,
    sigterm_id: signal_hook::SigId,
    sigint_id: signal_hook::SigId,
}

impl std::fmt::Debug for SignalPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalPipe")
            .field("read_fd", &self.read.as_raw_fd())
            .finish_non_exhaustive()
    }
}

impl SignalPipe {
    pub fn new() -> Result<Self> {
        let (read, write) = UnixStream::pair().context("create signal self-pipe")?;
        read.set_nonblocking(true)
            .context("set signal self-pipe read end nonblocking")?;
        // signal-hook's `register` takes ownership of the write end (it will
        // close the fd on unregister) so we hand each registration a distinct
        // clone of the write side.
        let write_for_term = write
            .try_clone()
            .context("clone signal self-pipe write end for SIGTERM")?;
        let write_for_int = write
            .try_clone()
            .context("clone signal self-pipe write end for SIGINT")?;
        drop(write);
        let sigterm_id =
            pipe::register(SIGTERM, write_for_term).context("register SIGTERM self-pipe writer")?;
        let sigint_id = match pipe::register(SIGINT, write_for_int) {
            Ok(id) => id,
            Err(e) => {
                signal_hook::low_level::unregister(sigterm_id);
                return Err(e).context("register SIGINT self-pipe writer");
            }
        };
        Ok(Self {
            read,
            sigterm_id,
            sigint_id,
        })
    }

    #[must_use]
    pub fn read_fd(&self) -> i32 {
        self.read.as_raw_fd()
    }

    pub fn drain(&self) {
        let mut buf = [0_u8; 64];
        loop {
            match (&self.read).read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
}

impl Drop for SignalPipe {
    fn drop(&mut self) {
        signal_hook::low_level::unregister(self.sigterm_id);
        signal_hook::low_level::unregister(self.sigint_id);
    }
}

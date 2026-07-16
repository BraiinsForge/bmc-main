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

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

pub fn run<F>(path: &str, mut on_write: F) -> io::Result<()>
where
    F: FnMut(&[u8]),
{
    // Open non-blocking so we can drain backlog that accumulated in the
    // kernel FIFO before anyone was reading. The drained bytes are still
    // passed to the decoder/caller so it can keep the newest complete LED
    // state without replaying the full history later.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;

    let mut buf = [0_u8; 4096];
    let mut drained = 0_usize;
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                drained += n;
                on_write(&buf[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    if drained > 0 {
        eprintln!("bmc-virt-leds: drained {drained} stale bytes from {path}");
    }

    // Switch to blocking mode for the real capture loop.
    let fd = file.as_raw_fd();
    // SAFETY: fd is valid and owned by `file`.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut logged_first_read = false;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("capture stream closed: {path}"),
            ));
        }
        if !logged_first_read {
            eprintln!("bmc-virt-leds: received first SPI capture chunk ({n} bytes)");
            logged_first_read = true;
        }
        on_write(&buf[..n]);
    }
}

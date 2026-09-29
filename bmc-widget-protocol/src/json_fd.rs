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

use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;

/// Read the UTF-8 JSON behind a `deck_widget_surface_v2` payload fd,
/// which must hold exactly `size` bytes.
///
/// The fd shares the sender's open file description,
/// and the sender's write left its offset at the end,
/// so the read is positional from offset 0.
pub fn read_json_fd(fd: OwnedFd, size: u32) -> io::Result<String> {
    let file = File::from(fd);
    let actual = file.metadata()?.len();
    if actual != u64::from(size) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("payload fd holds {actual} bytes, the event announced {size}"),
        ));
    }
    let len = usize::try_from(size).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut bytes = vec![0; len];
    file.read_exact_at(&mut bytes, 0)?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

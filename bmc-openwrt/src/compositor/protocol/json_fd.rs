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
use std::io::Write;
use std::os::fd::OwnedFd;

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};

/// A JSON payload ready for a `deck_widget_surface_v2` fd event.
pub struct JsonFd {
    pub fd: OwnedFd,
    pub size: u32,
}

/// Write `json` into a fresh memfd, sealed so the receiver can trust `size`
/// for as long as it holds the fd.
/// `name` labels the memfd in `/proc/<pid>/fd`.
pub fn sealed(name: &str, json: &str) -> io::Result<JsonFd> {
    let size = u32::try_from(json.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{name} JSON of {} bytes overflows the u32 size argument",
                json.len()
            ),
        )
    })?;
    let mut file = File::from(memfd_create(
        name,
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?);
    file.write_all(json.as_bytes())?;
    fcntl_add_seals(
        &file,
        SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
    )?;
    Ok(JsonFd {
        fd: file.into(),
        size,
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileExt;

    use super::*;
    use bmc_widget_protocol::read_json_fd;

    const JSON: &str = r#"{"label":"Žluťoučký kůň","symbols":["NVDA","AAPL"]}"#;

    #[test]
    fn a_sealed_payload_reads_back_whole_despite_the_write_offset() {
        let payload = sealed("test-params", JSON).expect("BUG: memfd must be creatable in tests");

        assert_eq!(
            Ok(payload.size),
            u32::try_from(JSON.len()),
            "size counts bytes, not chars"
        );
        assert_eq!(
            read_json_fd(payload.fd, payload.size).expect("BUG: sealed payload must read back"),
            JSON
        );
    }

    fn sealed_file() -> File {
        let payload = sealed("test-params", JSON).expect("BUG: memfd must be creatable in tests");
        File::from(payload.fd)
    }

    #[test]
    fn a_sealed_payload_refuses_rewriting_its_bytes() {
        assert!(
            sealed_file().write_all_at(b"tamper", 0).is_err(),
            "the write seal must stop the sender mutating a payload the receiver holds"
        );
    }

    #[test]
    fn a_sealed_payload_refuses_shrinking() {
        assert!(
            sealed_file().set_len(0).is_err(),
            "the shrink seal must stop a payload losing bytes the receiver was promised"
        );
    }

    #[test]
    fn a_sealed_payload_refuses_growing() {
        let grown = u64::try_from(JSON.len() + 1).expect("BUG: the fixture length fits u64");
        assert!(
            sealed_file().set_len(grown).is_err(),
            "the grow seal must stop a payload outgrowing the size the receiver was sent"
        );
    }

    #[test]
    fn a_size_mismatch_is_rejected_rather_than_misparsed() {
        let payload = sealed("test-params", JSON).expect("BUG: memfd must be creatable in tests");

        let error = read_json_fd(payload.fd, payload.size + 1)
            .expect_err("a size the fd does not hold must fail the read");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

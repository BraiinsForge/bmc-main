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
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Widget install intent for a firmware-carried widget install. Written by
/// bmc when a firmware upgrade starts with a pending install; read by the
/// sysupgrade sequence via `bmc-nix-cli upgrade --install-from` before the
/// flash in the same boot, not across the reboot.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingInstall {
    #[serde(default)]
    pub install: Vec<String>,
}

#[derive(Debug)]
pub enum PendingInstallError {
    Io(io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for PendingInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "reading pending-install file: {err}"),
            Self::Parse(err) => write!(f, "parsing pending-install file: {err}"),
        }
    }
}

impl std::error::Error for PendingInstallError {}

pub fn write_pending_install(path: &Path, pending: &PendingInstall) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(pending).expect("BUG: PendingInstall serializes");
    std::fs::write(path, json)
}

pub fn read_pending_install(path: &Path) -> Result<PendingInstall, PendingInstallError> {
    let bytes = std::fs::read(path).map_err(PendingInstallError::Io)?;
    serde_json::from_slice(&bytes).map_err(PendingInstallError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_install_names() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("pending.json");
        let pending = PendingInstall {
            install: vec!["widget-weather".to_owned(), "widget-ticker".to_owned()],
        };
        write_pending_install(&path, &pending).expect("BUG: write");
        let read = read_pending_install(&path).expect("BUG: read");
        assert_eq!(read.install, pending.install);
    }

    #[test]
    fn tolerates_unknown_fields() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("pending.json");
        std::fs::write(&path, r#"{"install":["widget-weather"],"future":"ok"}"#)
            .expect("BUG: seed");
        let read = read_pending_install(&path).expect("BUG: read");
        assert_eq!(read.install, vec!["widget-weather".to_owned()]);
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

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

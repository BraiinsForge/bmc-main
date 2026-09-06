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
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

/// Read-only assessment of the persistent Nix installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallationStatus {
    /// The configured store root does not exist.
    Absent,
    /// The root exists but lacks a populated store, database, or BMC profile.
    Incomplete,
    /// The installation is complete but the Nix mount has a different identity.
    NotMounted,
    /// The installation is complete and mounted at the configured Nix root.
    Ready,
}

/// Inspect a persistent store root and its expected Nix mount without modifying either.
pub fn inspect_installation(
    store_root: &Path,
    nix_mount_root: &Path,
) -> io::Result<InstallationStatus> {
    let Some(root_metadata) = metadata_if_present(store_root)? else {
        return Ok(InstallationStatus::Absent);
    };
    if !root_metadata.is_dir() || !store_is_initialized(store_root)? {
        return Ok(InstallationStatus::Incomplete);
    }
    if !same_file(store_root, nix_mount_root)? {
        return Ok(InstallationStatus::NotMounted);
    }
    Ok(InstallationStatus::Ready)
}

/// Check whether a store root contains the files required by CLI initialization.
pub fn store_is_initialized(store_root: &Path) -> io::Result<bool> {
    let Some(mut entries) = read_dir_if_present(&store_root.join("store"))? else {
        return Ok(false);
    };
    if entries.next().transpose()?.is_none() {
        return Ok(false);
    }

    let database_exists = path_is_file(&store_root.join("var/nix/db/db.sqlite"))?;
    let profile_exists = path_is_dir(&store_root.join("var/nix/gcroots/profiles/bmc"))?;
    Ok(database_exists && profile_exists)
}

/// Return whether two paths resolve to the same Unix device and inode.
pub fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    let Some(left) = metadata_if_present(left)? else {
        return Ok(false);
    };
    let Some(right) = metadata_if_present(right)? else {
        return Ok(false);
    };
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

fn read_dir_if_present(path: &Path) -> io::Result<Option<std::fs::ReadDir>> {
    match std::fs::read_dir(path) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn metadata_if_present(path: &Path) -> io::Result<Option<std::fs::Metadata>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn path_is_file(path: &Path) -> io::Result<bool> {
    Ok(metadata_if_present(path)?.is_some_and(|metadata| metadata.is_file()))
}

fn path_is_dir(path: &Path) -> io::Result<bool> {
    Ok(metadata_if_present(path)?.is_some_and(|metadata| metadata.is_dir()))
}

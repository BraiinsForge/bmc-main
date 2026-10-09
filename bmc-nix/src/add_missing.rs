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

//! Install packages only when their name is not in the profile yet.

use std::io;
use std::path::Path;

use crate::types::{InstallResult, ResolvedPackage};
use crate::upgrade::{self, ActivationMode, InstallError, UpgradeProgress};
use crate::{activation, profile, store};

/// What [`add_missing_packages`] did.
#[derive(Debug)]
pub enum AddMissingOutcome {
    /// At least one package was missing; a generation was built and activated.
    Installed(Box<InstallResult>),
    /// Every requested name is already in the profile; nothing changed.
    AlreadyInstalled,
    /// A deferred-boot marker exists; nothing changed so the staged
    /// generation boots as planned.
    NextBootStaged,
    /// `current` is not the expected generation; nothing changed.
    ProfileChanged { current: Option<usize> },
}

#[derive(Debug, thiserror::Error)]
pub enum AddMissingError {
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error("failed to scan the profile for next-boot markers: {0}")]
    ScanMarkers(#[source] io::Error),
}

/// Add each of `packages` whose name the profile does not have yet, under
/// one profile-lock acquisition. Nothing is applied when `current` is not
/// `expected_current` (if given) or a next-boot generation is staged.
///
/// Unlike `apply_profile_change`, the "nothing to do" outcomes leave
/// `next.*` markers alone.
pub async fn add_missing_packages(
    store: &impl store::StoreOperations,
    profile_dir: &Path,
    packages: &[ResolvedPackage],
    expected_current: Option<usize>,
    progress: Option<&dyn UpgradeProgress>,
    hooks_dir: &str,
    hooks_override_path: Option<&Path>,
) -> Result<AddMissingOutcome, AddMissingError> {
    let lock = profile::lock_profile(profile_dir)
        .await
        .map_err(InstallError::Lock)?;

    if let Some(expected) = expected_current {
        let current = upgrade::current_generation_number(profile_dir)?;
        if current != Some(expected) {
            return Ok(AddMissingOutcome::ProfileChanged { current });
        }
    }

    if has_next_marker(profile_dir).map_err(AddMissingError::ScanMarkers)? {
        return Ok(AddMissingOutcome::NextBootStaged);
    }

    let (base, _) = upgrade::resolve_base_manifest(profile_dir, None)?;
    let missing: Vec<ResolvedPackage> = packages
        .iter()
        .filter(|package| !base.packages.contains_key(&package.name))
        .cloned()
        .collect();
    if missing.is_empty() {
        return Ok(AddMissingOutcome::AlreadyInstalled);
    }

    let result = upgrade::apply_locked_profile_change(
        &lock,
        store,
        profile_dir,
        None,
        None,
        &missing,
        &[],
        ActivationMode::Activate,
        None,
        progress,
        hooks_dir,
        hooks_override_path,
    )
    .await?;
    Ok(AddMissingOutcome::Installed(Box::new(result)))
}

/// Whether `profile_dir` holds a deferred-activation marker. Like the
/// sweep, only symlinks count.
fn has_next_marker(profile_dir: &Path) -> io::Result<bool> {
    let entries = match std::fs::read_dir(profile_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let is_marker_name = entry
            .file_name()
            .to_str()
            .is_some_and(activation::is_next_marker_name);
        if is_marker_name && entry.file_type()?.is_symlink() {
            return Ok(true);
        }
    }
    Ok(false)
}

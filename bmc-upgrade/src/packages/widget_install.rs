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

use std::path::Path;

use bmc_nix::store::{AddToStoreError, StoreCommandError, required_with_headroom};

use super::{PackageUpgrader, WidgetInstallError, WidgetInstallOutcome};
use crate::widget_package::{
    ArchiveStream, MAX_ARCHIVE_BYTES, WASM_SDK_MAJOR, WidgetPackage, WidgetPackageError,
    limit_archive,
};

pub(super) fn install_outcome(
    result: &bmc_nix::types::InstallResult,
    package_name: &str,
) -> WidgetInstallOutcome {
    if result.added.iter().any(|added| added.name == package_name) {
        WidgetInstallOutcome::Installed
    } else if let Some(change) = result.changed.iter().find(|c| c.name == package_name) {
        if same_version(&change.from_version, &change.to_version) {
            WidgetInstallOutcome::Reinstalled
        } else {
            WidgetInstallOutcome::Replaced {
                previous_version: change.from_version.clone(),
            }
        }
    } else {
        WidgetInstallOutcome::AlreadyInstalled
    }
}

/// Read the way the downgrade check reads them, so `1.0` and `1.0.0` are one version.
pub(super) fn same_version(installed: &str, uploaded: &str) -> bool {
    bmc_nix::index::parse_package_version(installed)
        .zip(bmc_nix::index::parse_package_version(uploaded))
        .map_or(installed == uploaded, |(installed, uploaded)| {
            installed == uploaded
        })
}

/// The profile plan keeps an installed package when an explicit add would
/// downgrade it, silently from the uploader's point of view; refuse up front
/// so they learn why nothing changed.
fn refuse_downgrade(profile_dir: &Path, package: &WidgetPackage) -> Result<(), WidgetInstallError> {
    let installed = match bmc_nix::manifest::read_current_manifest(profile_dir) {
        Ok(manifest) => manifest,
        Err(bmc_nix::manifest::ReadManifestError::CurrentNotFound { .. }) => return Ok(()),
        Err(error) => return Err(WidgetInstallError::ReadProfile(error)),
    };
    let package_name = package.package_name();
    let Some(current) = installed.packages.get(&package_name) else {
        return Ok(());
    };
    let is_downgrade = bmc_nix::index::parse_package_version(&current.version)
        .is_some_and(|installed| package.manifest().version < installed);
    if is_downgrade {
        return Err(WidgetInstallError::Downgrade {
            package_name,
            installed: current.version.clone(),
            uploaded: package.manifest().version.to_string(),
        });
    }
    Ok(())
}

fn refuse_folder_clash(
    profile_dir: &Path,
    package: &WidgetPackage,
) -> Result<(), WidgetInstallError> {
    let current = bmc_nix::profile::current_generation_link(profile_dir)
        .map_err(|error| WidgetInstallError::Failed(error.to_string()))?;
    let clash = current.and_then(|current| package.folder_clash(&current.join("lib/bmc-widgets")));
    match clash {
        Some(installed) => Err(WidgetInstallError::FolderTaken {
            uploaded: package.relative_dir(),
            installed,
        }),
        None => Ok(()),
    }
}

fn refuse_uid_clash(profile_dir: &Path, package: &WidgetPackage) -> Result<(), WidgetInstallError> {
    let current = bmc_nix::profile::current_generation_link(profile_dir)
        .map_err(|error| WidgetInstallError::Failed(error.to_string()))?;
    let clash = current.and_then(|current| package.uid_clash(&current.join("lib/bmc-widgets")));
    match clash {
        Some(installed) => Err(WidgetInstallError::UidTaken {
            uploaded: package.relative_dir(),
            installed,
        }),
        None => Ok(()),
    }
}

/// A checked upload in the store, with the profile lock
/// that keeps the GC off it until the profile roots it.
pub(super) struct StagedUpload {
    pub(super) package: WidgetPackage,
    pub(super) store_path: String,
    pub(super) lock: bmc_nix::profile::ProfileLock,
    pub(super) generations_before: Option<usize>,
}

impl<N: bmc_nix::store::StoreOperations> PackageUpgrader<N> {
    /// Unpack the upload under `staging` and check it,
    /// then lock the profile and add it to the store.
    pub(super) async fn add_upload(
        &self,
        staging: &Path,
        nar: ArchiveStream,
    ) -> Result<StagedUpload, WidgetInstallError> {
        let unpacked = staging.join("package");
        self.nix
            .restore_nar(limit_archive(nar), &unpacked)
            .await
            .map_err(restore_error)?;
        let checked = unpacked.clone();
        let package = blocking(move || WidgetPackage::from_dir(&checked, WASM_SDK_MAJOR))
            .await
            .map_err(package_error)?;

        // The store path is named after the directory added, so the name
        // matches what the index-built package would have.
        let tree = staging.join(format!("bmc-{}", package.package_name()));
        std::fs::rename(&unpacked, &tree).map_err(WidgetInstallError::Staging)?;
        let (lock, generations_before) = self.lock_for_install(&package).await?;
        let store_path = self
            .nix
            .add_to_store(&tree)
            .await
            .map_err(|error| match error {
                AddToStoreError::Command(error) => WidgetInstallError::Store(error),
                AddToStoreError::NoPath => WidgetInstallError::Failed(error.to_string()),
            })?;
        Ok(StagedUpload {
            package,
            store_path,
            lock,
            generations_before,
        })
    }

    /// The installed widgets and the floor are read under the lock the build
    /// holds, so another writer's install meanwhile can neither slip past
    /// the checks nor be mistaken for this one's generation.
    async fn lock_for_install(
        &self,
        package: &WidgetPackage,
    ) -> Result<(bmc_nix::profile::ProfileLock, Option<usize>), WidgetInstallError> {
        let lock = bmc_nix::profile::lock_profile_with_timeout(
            &self.config.profile_dir,
            self.profile_lock_timeout,
        )
        .await
        .map_err(|error| WidgetInstallError::Install(bmc_nix::upgrade::InstallError::Lock(error)))?
        .ok_or(WidgetInstallError::ProfileBusy)?;
        refuse_downgrade(&self.config.profile_dir, package)?;
        refuse_folder_clash(&self.config.profile_dir, package)?;
        refuse_uid_clash(&self.config.profile_dir, package)?;
        let generations_before = bmc_nix::profile::max_generation(&self.config.profile_dir)
            .map_err(|error| WidgetInstallError::Failed(error.to_string()))?;
        Ok((lock, generations_before))
    }

    /// Refuse an upload the filesystem cannot hold before any of it is staged:
    /// the archive lands once unpacked in staging and once more in the store,
    /// and an unannounced size is taken to be the cap.
    pub(super) fn refuse_unless_it_fits(
        &self,
        announced_bytes: Option<u64>,
    ) -> Result<(), WidgetInstallError> {
        let announced = announced_bytes.unwrap_or(MAX_ARCHIVE_BYTES);
        let unpacked_bytes = announced.saturating_mul(2);
        let required_bytes = required_with_headroom(unpacked_bytes);
        let free_bytes = self
            .nix
            .store_free_bytes(&self.config.profile_dir)
            .map_err(WidgetInstallError::Staging)?;
        if free_bytes < required_bytes {
            return Err(WidgetInstallError::Install(
                bmc_nix::upgrade::InstallError::NotEnoughSpace {
                    free_bytes,
                    required_bytes,
                    unpacked_bytes,
                },
            ));
        }
        Ok(())
    }
}

/// Nix refusing the bytes is the uploader's archive; any other failure is
/// the device's store, unless the feed was cut off at the size cap, the
/// deadline or the client going away.
pub(super) fn restore_error(error: StoreCommandError) -> WidgetInstallError {
    match error {
        StoreCommandError::Exited { ref stderr, .. } if nix_refused_the_archive(stderr) => {
            WidgetInstallError::Rejected(WidgetPackageError::Archive(stderr.clone()))
        }
        StoreCommandError::Io { ref source, .. }
            if source.kind() == std::io::ErrorKind::FileTooLarge =>
        {
            WidgetInstallError::Rejected(WidgetPackageError::TooLarge)
        }
        StoreCommandError::Io { ref source, .. }
            if source.kind() == std::io::ErrorKind::TimedOut =>
        {
            WidgetInstallError::UploadStalled
        }
        StoreCommandError::Io { ref source, .. }
            if source.kind() == std::io::ErrorKind::ConnectionAborted =>
        {
            WidgetInstallError::UploadAborted
        }
        StoreCommandError::Exited { .. } | StoreCommandError::Io { .. } => {
            WidgetInstallError::Store(error)
        }
    }
}

/// An upload's tree can hold enough entries to stall an async worker
/// for seconds while it is walked or deleted.
pub(super) async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("BUG: blocking filesystem work does not panic")
}

/// Symlinks are refused before any read, so a tree nix just unpacked
/// failing to read is the device's fault, not the package's.
#[must_use]
pub fn package_error(error: WidgetPackageError) -> WidgetInstallError {
    match error {
        WidgetPackageError::Io { .. } => WidgetInstallError::Failed(error.to_string()),
        WidgetPackageError::TooLarge
        | WidgetPackageError::Archive(_)
        | WidgetPackageError::FileTooLarge { .. }
        | WidgetPackageError::NotOneWidget(_)
        | WidgetPackageError::NotADirectory(_)
        | WidgetPackageError::BadWidgetName(_)
        | WidgetPackageError::NotUtf8Name(_)
        | WidgetPackageError::Missing(_)
        | WidgetPackageError::NativeBinary(_)
        | WidgetPackageError::Symlink(_)
        | WidgetPackageError::TooDeep(_)
        | WidgetPackageError::TooManyEntries(_)
        | WidgetPackageError::Unexpected(_)
        | WidgetPackageError::UnexpectedExecutable(_)
        | WidgetPackageError::Manifest(_)
        | WidgetPackageError::BinaryPath { .. }
        | WidgetPackageError::Icon(_)
        | WidgetPackageError::WrapperNotExecutable(_)
        | WidgetPackageError::SdkMajor { .. }
        | WidgetPackageError::Wrapper(_)
        | WidgetPackageError::NotWasm(_) => WidgetInstallError::Rejected(error),
    }
}

/// Nix reports malformed input with these messages and no error kind,
/// in plain and JSON log formats alike (checked with nix 2.28 and 2.34).
/// Were they reworded, a bad archive would read as a device fault:
/// a less helpful answer, but one that leaks nothing.
fn nix_refused_the_archive(stderr: &str) -> bool {
    stderr.contains("bad archive:") || stderr.contains("unexpected end-of-file")
}

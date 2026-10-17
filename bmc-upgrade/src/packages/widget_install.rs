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

use std::path::{Path, PathBuf};

use bmc_nix::store::{AddToStoreError, required_with_headroom};
use nix_archive::nar;

use super::{PackageUpgrader, WidgetInstallError, WidgetInstallOutcome};
use crate::widget_package::{
    ArchiveStream, MAX_ARCHIVE_BYTES, MAX_ARCHIVE_ENTRIES, WASM_SDK_MAJOR, WidgetPackage,
    WidgetPackageError, limit_archive,
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
    /// Save the upload under `staging`, unpack it if it fits and check it,
    /// then lock the profile and add it to the store.
    pub(super) async fn add_upload(
        &self,
        staging: &Path,
        nar: ArchiveStream,
    ) -> Result<StagedUpload, WidgetInstallError> {
        let saved = SavedUpload::save(staging, nar).await?;
        // The tree lands once in staging and once more in the store.
        self.refuse_unless_free(saved.unpacked_bytes().saturating_mul(2))?;
        let unpacked = saved.unpack().await?;
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
        self.refuse_unless_free(announced.saturating_mul(2))
    }

    fn refuse_unless_free(&self, unpacked_bytes: u64) -> Result<(), WidgetInstallError> {
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

/// The block size of the device's ext4 data partition;
/// no entry unpacks into less than one block.
const BLOCK_BYTES: u64 = 4096;

/// An upload saved whole under staging and measured, none of it unpacked yet.
#[derive(Debug)]
pub struct SavedUpload {
    archive: PathBuf,
    unpacked_bytes: u64,
}

impl SavedUpload {
    /// Save an upload under `staging` and measure the tree it unpacks to,
    /// counting its entries before any is written out.
    pub async fn save(staging: &Path, nar: ArchiveStream) -> Result<Self, WidgetInstallError> {
        let archive = staging.join("upload.nar");
        write_archive(nar, &archive).await?;
        let measured = archive.clone();
        let unpacked_bytes = blocking(move || {
            let unpacked_bytes = measure(&measured);
            if unpacked_bytes.is_err() {
                std::fs::remove_file(&measured).map_err(WidgetInstallError::Staging)?;
            }
            unpacked_bytes
        })
        .await?;
        Ok(Self {
            archive,
            unpacked_bytes,
        })
    }

    /// What the unpacked tree takes on disk, each file's contents
    /// in whole blocks and every other entry in one.
    #[must_use]
    pub fn unpacked_bytes(&self) -> u64 {
        self.unpacked_bytes
    }

    /// Unpack to `package` beside the saved archive,
    /// which is gone before the tree can reach the store.
    pub async fn unpack(self) -> Result<PathBuf, WidgetInstallError> {
        let unpacked = self.archive.with_file_name("package");
        let target = unpacked.clone();
        blocking(move || {
            let restored = std::fs::File::open(&self.archive)
                .map(std::io::BufReader::new)
                .map_err(WidgetInstallError::Staging)
                .and_then(|mut archive| {
                    nar::restore_reader(&mut archive, &target, nar::CaseHack::Disabled)
                        .map_err(archive_error)
                });
            let removed = std::fs::remove_file(&self.archive).map_err(WidgetInstallError::Staging);
            restored.and(removed)
        })
        .await?;
        Ok(unpacked)
    }
}

async fn write_archive(nar: ArchiveStream, path: &Path) -> Result<(), WidgetInstallError> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut nar = limit_archive(nar);
    let mut file = tokio::fs::File::create_new(path)
        .await
        .map_err(WidgetInstallError::Staging)?;
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let read = nar.read(&mut chunk).await.map_err(upload_error)?;
        if read == 0 {
            break;
        }
        file.write_all(&chunk[..read])
            .await
            .map_err(WidgetInstallError::Staging)?;
    }
    file.flush().await.map_err(WidgetInstallError::Staging)
}

/// The size cap, the deadline and the client going away end the upload as
/// read errors of their own kinds; any other is the device's.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "io::ErrorKind is non-exhaustive"
)]
fn upload_error(error: std::io::Error) -> WidgetInstallError {
    match error.kind() {
        std::io::ErrorKind::FileTooLarge => {
            WidgetInstallError::Rejected(WidgetPackageError::TooLarge)
        }
        std::io::ErrorKind::TimedOut => WidgetInstallError::UploadStalled,
        std::io::ErrorKind::ConnectionAborted => WidgetInstallError::UploadAborted,
        _ => WidgetInstallError::Staging(error),
    }
}

fn measure(archive: &Path) -> Result<u64, WidgetInstallError> {
    let mut archive = std::fs::File::open(archive)
        .map(std::io::BufReader::new)
        .map_err(WidgetInstallError::Staging)?;
    let mut entries = 0_usize;
    let mut unpacked_bytes = 0_u64;
    let decoded = nar::decode_events_reader(&mut archive, |event| {
        let entry_bytes = match event {
            nar::Event::DirectoryStart { .. } | nar::Event::Symlink { .. } => BLOCK_BYTES,
            nar::Event::Regular { contents, .. } => contents
                .size()
                .checked_next_multiple_of(BLOCK_BYTES)
                .unwrap_or(u64::MAX)
                .max(BLOCK_BYTES),
            nar::Event::DirectoryEnd { .. } => return Ok(()),
        };
        entries += 1;
        unpacked_bytes = unpacked_bytes.saturating_add(entry_bytes);
        if entries > MAX_ARCHIVE_ENTRIES {
            return Err(nar::Error::Io(std::io::Error::other("entry bound reached")));
        }
        Ok(())
    });
    if entries > MAX_ARCHIVE_ENTRIES {
        return Err(WidgetInstallError::Rejected(
            WidgetPackageError::TooManyArchiveEntries,
        ));
    }
    decoded.map_err(archive_error)?;
    Ok(unpacked_bytes)
}

/// Failing to read the saved archive or write the tree is the device's
/// fault; anything else nix-archive refuses is the uploader's archive.
fn archive_error(error: nar::Error) -> WidgetInstallError {
    if let nar::Error::Io(source) = error {
        WidgetInstallError::Staging(source)
    } else {
        WidgetInstallError::Rejected(WidgetPackageError::Archive(error.to_string()))
    }
}

/// An upload's tree can hold enough entries to stall an async worker
/// for seconds while it is walked or deleted.
pub(super) async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("BUG: blocking filesystem work does not panic")
}

/// Symlinks are refused before any read, so failing to read
/// the unpacked tree is the device's fault, not the package's.
#[must_use]
pub fn package_error(error: WidgetPackageError) -> WidgetInstallError {
    match error {
        WidgetPackageError::Io { .. } => WidgetInstallError::Failed(error.to_string()),
        WidgetPackageError::TooLarge
        | WidgetPackageError::Archive(_)
        | WidgetPackageError::TooManyArchiveEntries
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

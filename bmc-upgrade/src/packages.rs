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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::widget_package::{
    ArchiveStream, FolderClash, MAX_ARCHIVE_BYTES, UPLOAD_DEADLINE_MINS, WASM_SDK_MAJOR,
    WidgetPackage, WidgetPackageError, WidgetUpload, limit_archive, shown,
};
use bmc_nix::store::{
    AddToStoreError, ESTIMATE_TIMEOUT, StoreCommandError, required_with_headroom,
};
use tracing::{info, warn};

pub use bmc_upgrade_types::{
    InstallablePackage, PackageChange as SystemPackageChange, PackagesPreview,
};

/// Package-index fetches run under the upgrade run gate: a hung index
/// server must time out instead of wedging every check/start in
/// `UpgradeInProgress`.
const INDEX_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// A widget install waits this long for another writer of the profile,
/// such as the CLI, while it holds the upgrade run gate.
const PROFILE_LOCK_TIMEOUT: Duration = Duration::from_mins(1);

#[derive(Clone, Debug)]
pub struct NixUpgradeConfig {
    pub servers_config_path: PathBuf,
    pub gc_config_path: PathBuf,
    pub profile_dir: PathBuf,
    /// Where an uploaded archive unpacks before the store takes it; on the
    /// store's filesystem, since tmpfs would hold it in RAM.
    pub staging_dir: PathBuf,
    pub hooks_dir: String,
    pub hooks_override_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug)]
pub enum EstimateMode {
    Estimate,
    Skip,
}

#[derive(Debug)]
pub enum PackageProbe {
    Available(crate::offers::PackageOffer),
    /// All package preconditions succeeded and the plan is empty.
    UpToDate,
    /// The package check could not produce a trustworthy answer.
    Failed(PackageProbeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageProbeError {
    NoEnabledServers,
    ServersConfigUnavailable(String),
    IndexFetchFailed(String),
    IndexUnusable(String),
    ManifestReadFailed(String),
    PlanFailed(PackagePlanFailure),
    Unrealizable(String),
    InstallTargetUnavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackagePlanFailure {
    MissingSystemPackages { names: Vec<String> },
    Other(String),
}

impl PackageProbeError {
    /// Only a transient index fetch is worth an autoupgrade retry.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::IndexFetchFailed(_))
    }
}

impl std::fmt::Display for PackageProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoEnabledServers => write!(f, "no enabled package servers are configured"),
            Self::ServersConfigUnavailable(_) => {
                write!(f, "package server configuration is unavailable")
            }
            Self::IndexFetchFailed(msg) => write!(f, "package index fetch failed: {msg}"),
            Self::IndexUnusable(msg) => write!(f, "package index is unusable: {msg}"),
            Self::ManifestReadFailed(msg) => write!(f, "package manifest could not be read: {msg}"),
            Self::PlanFailed(failure) => write!(f, "{failure}"),
            Self::Unrealizable(paths) => {
                write!(
                    f,
                    "package upgrade cannot be realized; no substituter provides: {paths}"
                )
            }
            Self::InstallTargetUnavailable(detail) => {
                write!(f, "requested package to install is unavailable: {detail}")
            }
        }
    }
}

impl std::error::Error for PackageProbeError {}

/// Classify a dry-run estimate failure. An unsubstitutable store path means
/// the upgrade could never realize, so the probe must fail loud instead of
/// offering a doomed upgrade. Every other estimate failure is transient — a
/// dead substituter, a killed dry-run, an unparsed summary — and only costs
/// the optional download-size preview.
fn unrealizable_estimate(err: &bmc_nix::store::StorePathError) -> Option<String> {
    match err {
        bmc_nix::store::StorePathError::UnsubstitutablePaths { paths } => Some(paths.join(", ")),
        bmc_nix::store::StorePathError::CheckValidityFailed(_)
        | bmc_nix::store::StorePathError::MissingStorePath { .. }
        | bmc_nix::store::StorePathError::RealiseFailed(_)
        | bmc_nix::store::StorePathError::RealiseExited { .. }
        | bmc_nix::store::StorePathError::EstimateFailed(_)
        | bmc_nix::store::StorePathError::EstimateExited { .. }
        | bmc_nix::store::StorePathError::EstimateSummaryUnparsed { .. } => None,
    }
}

impl std::fmt::Display for PackagePlanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSystemPackages { names } => {
                let (noun, verb) = if names.len() == 1 {
                    ("package", "is")
                } else {
                    ("packages", "are")
                };
                let quoted: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
                let list = match quoted.as_slice() {
                    [] => String::new(),
                    [single] => single.clone(),
                    [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                };
                write!(
                    f,
                    "package source is incomplete; required system {noun} {list} {verb} missing"
                )
            }
            Self::Other(msg) => write!(f, "package upgrade plan failed: {msg}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("resolving requested packages failed")]
    Resolve(#[source] PackageProbeError),
    #[error("installing packages failed")]
    Install(#[source] bmc_nix::upgrade::InstallError),
    #[error("{0}")]
    Failed(String),
}

/// How an uploaded widget package ended up in the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetInstallOutcome {
    Installed,
    Replaced {
        previous_version: String,
    },
    /// The same version was in the profile with different contents.
    Reinstalled,
    /// The same version at the same store path was already in the profile.
    AlreadyInstalled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledWidget {
    pub package_name: String,
    pub display_name: String,
    pub version: String,
    pub outcome: WidgetInstallOutcome,
}

#[derive(Debug, thiserror::Error)]
pub enum WidgetInstallError {
    #[error("{0}")]
    Rejected(#[source] WidgetPackageError),
    #[error(
        "{} {} is installed; uploading the older {} would downgrade it",
        shown(.package_name),
        shown(.installed),
        shown(.uploaded)
    )]
    Downgrade {
        package_name: String,
        installed: String,
        uploaded: String,
    },
    #[error("{} cannot be installed next to {installed}: only one of them would show", .uploaded.display())]
    FolderTaken {
        uploaded: PathBuf,
        installed: FolderClash,
    },
    #[error("upload did not finish within {UPLOAD_DEADLINE_MINS} minutes")]
    UploadStalled,
    #[error("another package change kept the profile locked")]
    ProfileBusy,
    #[error("the upload ended before the archive was complete")]
    UploadAborted,
    #[error("reading the installed packages failed: {0}")]
    ReadProfile(#[source] bmc_nix::manifest::ReadManifestError),
    #[error("staging the widget package failed: {0}")]
    Staging(#[source] std::io::Error),
    #[error("importing the widget package into the store failed: {0}")]
    Store(#[source] StoreCommandError),
    #[error("installing the widget package failed: {0}")]
    Install(#[source] bmc_nix::upgrade::InstallError),
    #[error("{0}")]
    Failed(String),
}

pub type PackageGcOutcome = bmc_nix::gc::ProfileGcOutcome;
pub type PackageGcRequest = bmc_nix::gc::GcRequest;

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum PackageGcError {
    #[error("failed to read gc configuration: {0}")]
    ConfigRead(String),
    #[error("failed to parse gc configuration: {0}")]
    ConfigParse(String),
    /// Cleanup removed entries and the run then failed before a sweep
    /// completed, so the next run must sweep unconditionally.
    #[error("garbage collection failed after removing {removed} profile entries: {message}")]
    UnsweptRemovals { removed: usize, message: String },
    #[error("garbage collection failed: {0}")]
    Operational(String),
}

impl PackageGcError {
    /// Entries removed that no completed sweep accounted for.
    #[must_use]
    pub fn unswept_removals(&self) -> usize {
        match self {
            Self::ConfigRead(_) | Self::ConfigParse(_) | Self::Operational(_) => 0,
            Self::UnsweptRemovals { removed, .. } => *removed,
        }
    }
}

/// Mockability seam for the package half of system upgrades: probing for
/// available package changes and applying them.
#[async_trait::async_trait]
pub trait PackageBackend: Send + Sync + std::fmt::Debug + 'static {
    async fn gc(&self, request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError>;
    async fn probe(
        &self,
        firmware: Option<&str>,
        estimate: EstimateMode,
        install: &[String],
    ) -> PackageProbe;
    async fn apply(
        &self,
        merged: bmc_nix::types::MergedIndex,
        install: Vec<String>,
        progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError>;
    async fn list_installable_packages(
        &self,
        firmware: Option<&str>,
    ) -> Result<Vec<InstallablePackage>, PackageProbeError>;
    /// Validate an uploaded widget archive and install it into the profile.
    /// A rejected or failed upload leaves the profile as it was,
    /// unless undoing a failed activation fails as well.
    async fn install_widget_package(
        &self,
        upload: WidgetUpload,
    ) -> Result<InstalledWidget, WidgetInstallError>;
    /// Free bytes on the filesystem holding the package store.
    fn store_free_bytes(&self) -> std::io::Result<u64>;
}

fn install_outcome(
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
fn same_version(installed: &str, uploaded: &str) -> bool {
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

/// The nix-backed [`PackageBackend`]: fetches package indexes over HTTP,
/// reads the profile manifest, and applies profile changes through
/// `bmc-nix`.
#[derive(Debug)]
pub struct PackageUpgrader<N = bmc_nix::store::Nix> {
    config: NixUpgradeConfig,
    client: reqwest::Client,
    /// Estimates dry-run realization. The default shells out to `nix-store`;
    /// tests inject a stub to drive the probe's estimate-routing branches
    /// (transient error keeps offering, unsubstitutable paths fail the probe).
    nix: N,
    profile_lock_timeout: Duration,
}

impl PackageUpgrader {
    #[must_use]
    pub fn new(config: NixUpgradeConfig) -> Self {
        Self::with_store(config, bmc_nix::store::Nix)
    }
}

impl<N> PackageUpgrader<N> {
    fn with_store(config: NixUpgradeConfig, nix: N) -> Self {
        let client = reqwest::Client::builder()
            .timeout(INDEX_FETCH_TIMEOUT)
            .build()
            .expect("BUG: client builder failed");
        Self {
            config,
            client,
            nix,
            profile_lock_timeout: PROFILE_LOCK_TIMEOUT,
        }
    }

    /// Fetch merged indexes and the current manifest, falling back to the latest manifest.
    /// Scope feeds to the explicit target firmware when supplied,
    /// otherwise read the running firmware only if an enabled feed needs it.
    /// Direct index servers need no firmware scope or local version file.
    async fn fetch_index_and_manifest(
        &self,
        firmware: Option<&str>,
    ) -> Result<(bmc_nix::types::MergedIndex, bmc_nix::types::Manifest), PackageProbeError> {
        // Same convention as the CLI: the read-only shipped default lives
        // next to the runtime config under a literal ".default" suffix.
        let default_path = {
            let mut derived = self.config.servers_config_path.as_os_str().to_owned();
            derived.push(".default");
            PathBuf::from(derived)
        };
        let config = bmc_nix::servers_config::load_servers_config(
            &self.config.servers_config_path,
            &default_path,
        )
        .map_err(|err| {
            warn!(error = %err, "Servers config unavailable");
            PackageProbeError::ServersConfigUnavailable(err.to_string())
        })?;
        let servers = config.servers;

        if !servers.iter().any(|server| server.enabled) {
            warn!("No enabled package servers");
            return Err(PackageProbeError::NoEnabledServers);
        }

        let firmware_scope = if let Some(version) = firmware {
            Some(version.to_owned())
        } else if servers.iter().any(|server| {
            server.enabled && matches!(server.source, bmc_nix::types::ServerSource::Feed { .. })
        }) {
            let version = std::fs::read_to_string("/etc/bos_version").map_err(|err| {
                warn!(error = %err, "Cannot read the BOS version for feed resolution");
                PackageProbeError::IndexUnusable(format!(
                    "failed to read the BOS version from /etc/bos_version: {err}"
                ))
            })?;
            Some(version.trim().to_owned())
        } else {
            None
        };

        let merged = match bmc_nix::index::fetch_and_merge_indexes(
            &self.client,
            &servers,
            &[],
            firmware_scope.as_deref(),
        )
        .await
        {
            Ok(merged) => merged,
            Err(err @ bmc_nix::index::FetchIndexesError::Fetch { .. }) => {
                warn!(error = %err, "Package index fetch failed");
                return Err(PackageProbeError::IndexFetchFailed(err.to_string()));
            }
            Err(err) => {
                warn!(error = %err, "Package index unusable");
                return Err(PackageProbeError::IndexUnusable(err.to_string()));
            }
        };

        let base = match bmc_nix::manifest::read_current_manifest(&self.config.profile_dir) {
            Ok(manifest) => manifest,
            Err(bmc_nix::manifest::ReadManifestError::CurrentNotFound { .. }) => {
                bmc_nix::manifest::read_latest_manifest(&self.config.profile_dir).map_err(
                    |err| {
                        warn!(error = %err, "Failed to read the profile manifest");
                        PackageProbeError::ManifestReadFailed(err.to_string())
                    },
                )?
            }
            Err(err) => {
                warn!(error = %err, "Failed to read the profile manifest");
                return Err(PackageProbeError::ManifestReadFailed(err.to_string()));
            }
        };

        Ok((merged, base))
    }
}

/// A checked upload in the store, with the profile lock
/// that keeps the GC off it until the profile roots it.
struct StagedUpload {
    package: WidgetPackage,
    store_path: String,
    lock: bmc_nix::profile::ProfileLock,
    generations_before: Option<usize>,
}

impl<N: bmc_nix::store::StoreOperations> PackageUpgrader<N> {
    /// Unpack the upload under `staging` and check it,
    /// then lock the profile and add it to the store.
    async fn add_upload(
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
        let generations_before = bmc_nix::profile::max_generation(&self.config.profile_dir)
            .map_err(|error| WidgetInstallError::Failed(error.to_string()))?;
        Ok((lock, generations_before))
    }

    /// Refuse an upload the filesystem cannot hold before any of it is staged:
    /// the archive lands once unpacked in staging and once more in the store,
    /// and an unannounced size is taken to be the cap.
    fn refuse_unless_it_fits(
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
fn restore_error(error: StoreCommandError) -> WidgetInstallError {
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
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
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

#[async_trait::async_trait]
impl<N: bmc_nix::store::StoreOperations> PackageBackend for PackageUpgrader<N> {
    async fn gc(&self, request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
        // Loaded per call, so a retention policy edit applies to the next
        // collection without a restart.
        let config =
            bmc_nix::gc::load_gc_config(&self.config.gc_config_path).map_err(|err| match err {
                bmc_nix::gc::LoadGcConfigError::Read { .. } => {
                    PackageGcError::ConfigRead(err.to_string())
                }
                bmc_nix::gc::LoadGcConfigError::Parse { .. } => {
                    PackageGcError::ConfigParse(err.to_string())
                }
            })?;

        bmc_nix::gc::collect_profile_garbage(
            &self.nix,
            &self.config.profile_dir,
            &config,
            request,
            None,
        )
        .await
        .map_err(|err| {
            let removed = err.unswept_removals();
            let message = err.to_string();
            if removed > 0 {
                PackageGcError::UnsweptRemovals { removed, message }
            } else {
                PackageGcError::Operational(message)
            }
        })
    }

    async fn probe(
        &self,
        firmware: Option<&str>,
        estimate: EstimateMode,
        install: &[String],
    ) -> PackageProbe {
        let (merged, base) = match self.fetch_index_and_manifest(firmware).await {
            Ok(pair) => pair,
            Err(err) => return PackageProbe::Failed(err),
        };

        let installs = match resolve_installs(&merged, install) {
            Ok(installs) => installs,
            Err(err) => return PackageProbe::Failed(err),
        };

        let plan =
            match bmc_nix::manifest::compute_upgrade_plan(&base, Some(&merged), &installs, &[]) {
                Ok(plan) => plan,
                Err(bmc_nix::manifest::ComputeUpgradePlanError::MissingSystemPackages {
                    names,
                }) => {
                    warn!(
                        ?names,
                        "Package plan failed: missing required system packages"
                    );
                    return PackageProbe::Failed(PackageProbeError::PlanFailed(
                        PackagePlanFailure::MissingSystemPackages { names },
                    ));
                }
                Err(err) => {
                    warn!(error = %err, "Failed to plan the package upgrade");
                    return PackageProbe::Failed(PackageProbeError::PlanFailed(
                        PackagePlanFailure::Other(err.to_string()),
                    ));
                }
            };

        if plan.changed.is_empty() && plan.added.is_empty() && plan.removed.is_empty() {
            info!("Packages are up to date");
            return PackageProbe::UpToDate;
        }

        let realize_estimate = match estimate {
            EstimateMode::Estimate => {
                match tokio::time::timeout(
                    ESTIMATE_TIMEOUT,
                    self.nix.estimate_realization(&plan.packages),
                )
                .await
                {
                    Ok(Ok(realize_estimate)) => Some(realize_estimate),
                    Ok(Err(err)) => {
                        if let Some(paths) = unrealizable_estimate(&err) {
                            warn!(%paths, "Package upgrade requires unsubstitutable store paths");
                            return PackageProbe::Failed(PackageProbeError::Unrealizable(paths));
                        }
                        warn!(error = %err, "Failed to estimate the package download size");
                        None
                    }
                    Err(_) => {
                        warn!(
                            timeout_secs = ESTIMATE_TIMEOUT.as_secs(),
                            "Package download size estimate timed out"
                        );
                        None
                    }
                }
            }
            EstimateMode::Skip => None,
        };

        PackageProbe::Available(crate::offers::PackageOffer {
            index: merged,
            manifest: base,
            preview: build_packages_preview(&plan, realize_estimate),
        })
    }

    async fn apply(
        &self,
        merged: bmc_nix::types::MergedIndex,
        install: Vec<String>,
        progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        // Unlike the probe estimate (ESTIMATE_TIMEOUT) and the firmware path
        // (DOWNLOAD_IDLE_TIMEOUT), the real realization has no tokio deadline
        // by design: a slow substituter on a large upgrade is legitimate and a
        // wall-clock cap would kill it. nix's own `stalled-download-timeout`
        // plus `kill_on_drop(true)` on the child bound a genuinely stuck fetch.
        let installs = resolve_installs(&merged, &install).map_err(ApplyError::Resolve)?;
        bmc_nix::upgrade::apply_profile_change(
            &self.nix,
            &self.config.profile_dir,
            None, // base manifest is re-read under the profile lock
            Some(&merged),
            &installs,
            &[],
            bmc_nix::upgrade::ActivationMode::Activate,
            None, // GC is disabled on the packages path
            Some(progress.as_ref()),
            &self.config.hooks_dir,
            self.config.hooks_override_path.as_deref(),
        )
        .await
        .map(|_| ())
        .map_err(ApplyError::Install)
    }

    async fn list_installable_packages(
        &self,
        firmware: Option<&str>,
    ) -> Result<Vec<InstallablePackage>, PackageProbeError> {
        let (merged, base) = self.fetch_index_and_manifest(firmware).await?;
        let installed = base.packages.keys().cloned().collect();
        Ok(installable_packages_from(&merged, &installed))
    }

    fn store_free_bytes(&self) -> std::io::Result<u64> {
        // The profile directory lives on the same filesystem as the store.
        bmc_nix::store::statvfs_free_bytes(&self.config.profile_dir)
    }

    async fn install_widget_package(
        &self,
        upload: WidgetUpload,
    ) -> Result<InstalledWidget, WidgetInstallError> {
        self.refuse_unless_it_fits(upload.announced_bytes)?;
        std::fs::create_dir_all(&self.config.staging_dir).map_err(WidgetInstallError::Staging)?;
        let staging =
            tempfile::tempdir_in(&self.config.staging_dir).map_err(WidgetInstallError::Staging)?;
        let added = self.add_upload(staging.path(), upload.nar).await;
        blocking(move || drop(staging)).await;
        let StagedUpload {
            package,
            store_path,
            lock,
            generations_before,
        } = added?;
        let package_name = package.package_name();

        let result = bmc_nix::upgrade::apply_locked_profile_change(
            &lock,
            &self.nix,
            &self.config.profile_dir,
            None,
            None,
            &[package.resolved(store_path.clone())],
            &[],
            bmc_nix::upgrade::ActivationMode::Activate,
            None,
            None,
            &self.config.hooks_dir,
            self.config.hooks_override_path.as_deref(),
        )
        .await;
        // A crash between the add and here leaves the path unreferenced
        // as well; the periodic GC reclaims it.
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                // A failed activation leaves its generation behind, still
                // pinning the store path; drop the generation first or the
                // delete refuses the path as live.
                match bmc_nix::gc::discard_generations_above(
                    &lock,
                    &self.config.profile_dir,
                    generations_before,
                ) {
                    Ok(discarded) if !discarded.is_empty() => {
                        info!(
                            ?discarded,
                            "discarded the generation the failed widget install built"
                        );
                    }
                    Ok(_) => {}
                    Err(discard_error) => {
                        warn!(%discard_error, "could not discard the failed widget install's generation");
                    }
                }
                if let Err(delete_error) = self.nix.delete_store_path(&store_path).await {
                    warn!(store_path, %delete_error, "could not discard the rejected widget package");
                }
                return Err(WidgetInstallError::Install(error));
            }
        };

        let outcome = install_outcome(&result, &package_name);
        info!(package = package_name, version = %package.manifest().version, ?outcome, "widget package installed");
        Ok(InstalledWidget {
            package_name,
            display_name: package.manifest().name.clone(),
            version: package.manifest().version.to_string(),
            outcome,
        })
    }
}

/// Resolve uninstalled names using the same index selection as installation.
#[must_use]
pub fn installable_packages_from(
    merged: &bmc_nix::types::MergedIndex,
    installed: &std::collections::BTreeSet<String>,
) -> Vec<InstallablePackage> {
    merged
        .by_name
        .keys()
        .filter(|name| !installed.contains(*name))
        .filter_map(|name| {
            let resolved = bmc_nix::index::resolve_new_package(
                merged,
                name,
                None,
                bmc_nix::types::InstalledBy::User,
            )
            .ok()?;
            Some(InstallablePackage {
                name: resolved.name,
                version: resolved.version,
                category: resolved.category,
                description: resolved.description,
                metadata: resolved.metadata,
            })
        })
        .collect()
}

/// Resolve requested install names against the merged index into
/// user-installed [`ResolvedPackage`]s for the plan/apply add-set.
pub fn resolve_installs(
    merged: &bmc_nix::types::MergedIndex,
    names: &[String],
) -> Result<Vec<bmc_nix::types::ResolvedPackage>, PackageProbeError> {
    names
        .iter()
        .map(|name| {
            bmc_nix::index::resolve_new_package(
                merged,
                name,
                None,
                bmc_nix::types::InstalledBy::User,
            )
            .map_err(|err| PackageProbeError::InstallTargetUnavailable(err.to_string()))
        })
        .collect()
}

#[must_use]
pub fn build_packages_preview(
    plan: &bmc_nix::types::UpgradePlan,
    realize_estimate: Option<bmc_nix::store::RealizeEstimate>,
) -> PackagesPreview {
    let resolved_by_name: HashMap<&str, &bmc_nix::types::ResolvedPackage> = plan
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let mut changes = Vec::new();
    for change in &plan.changed {
        let resolved = resolved_by_name.get(change.name.as_str());
        changes.push(SystemPackageChange {
            name: change.name.clone(),
            version_from: Some(change.from_version.clone()),
            version_to: Some(change.to_version.clone()),
            category: resolved.and_then(|package| package.category.clone()),
            changelog: resolved.and_then(|package| {
                package
                    .metadata
                    .get("changelog")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            }),
        });
    }
    for added in &plan.added {
        let resolved = resolved_by_name.get(added.name.as_str());
        changes.push(SystemPackageChange {
            name: added.name.clone(),
            version_from: None,
            version_to: Some(added.version.clone()),
            category: resolved.and_then(|package| package.category.clone()),
            changelog: resolved.and_then(|package| {
                package
                    .metadata
                    .get("changelog")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            }),
        });
    }
    for removed in &plan.removed {
        changes.push(SystemPackageChange {
            name: removed.name.clone(),
            version_from: Some(removed.version.clone()),
            version_to: None,
            category: None,
            changelog: None,
        });
    }

    let core = resolved_by_name.get("core");
    let bmc_version = core.and_then(|package| {
        package
            .metadata
            .get("bmc_version")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    });
    let bmc_changelog = core.and_then(|package| {
        package
            .metadata
            .get("changelog")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    });

    PackagesPreview {
        changes,
        download_size_bytes: realize_estimate.map(|estimate| estimate.download_bytes),
        unpacked_size_bytes: realize_estimate.map(|estimate| estimate.unpacked_bytes),
        bmc_version,
        bmc_changelog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn unsubstitutable_estimate_is_unrealizable() {
        let err = bmc_nix::store::StorePathError::UnsubstitutablePaths {
            paths: vec!["/nix/store/aaa".to_owned(), "/nix/store/bbb".to_owned()],
        };
        assert_eq!(
            unrealizable_estimate(&err),
            Some("/nix/store/aaa, /nix/store/bbb".to_owned())
        );
    }

    #[test]
    fn transient_estimate_errors_only_omit_the_size() {
        let unparsed = bmc_nix::store::StorePathError::EstimateSummaryUnparsed {
            line: "garbage".to_owned(),
        };
        assert_eq!(unrealizable_estimate(&unparsed), None);

        let start_failed = bmc_nix::store::StorePathError::EstimateFailed(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "nix-store missing",
        ));
        assert_eq!(unrealizable_estimate(&start_failed), None);
    }

    fn test_nix_config(root: &Path, servers: &Path) -> NixUpgradeConfig {
        NixUpgradeConfig {
            servers_config_path: servers.to_path_buf(),
            gc_config_path: root.join("gc.json"),
            profile_dir: root.join("profile"),
            staging_dir: root.join("upload"),
            hooks_dir: "hooks".to_owned(),
            hooks_override_path: None,
        }
    }

    #[derive(Debug)]
    struct GcStore {
        collect_calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl GcStore {
        fn successful() -> (Self, Arc<AtomicUsize>) {
            Self::new(false)
        }

        fn failing() -> (Self, Arc<AtomicUsize>) {
            Self::new(true)
        }

        fn new(fail: bool) -> (Self, Arc<AtomicUsize>) {
            let collect_calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    collect_calls: Arc::clone(&collect_calls),
                    fail,
                },
                collect_calls,
            )
        }
    }

    impl bmc_nix::store::StoreOperations for GcStore {
        async fn estimate_realization(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }

        async fn restore_nar(
            &self,
            _nar: bmc_nix::store::InputStream,
            _target: &Path,
        ) -> Result<(), bmc_nix::store::StoreCommandError> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }

        async fn delete_store_path(
            &self,
            _store_path: &str,
        ) -> Result<(), bmc_nix::store::StoreCommandError> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }

        fn store_free_bytes(&self, _profile_dir: &std::path::Path) -> std::io::Result<u64> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }

        async fn realize_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
            _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }

        async fn verify_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }

        async fn collect_garbage(
            &self,
            _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
        ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
            self.collect_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(bmc_nix::gc::CollectGarbageError::NixCommand(
                    std::io::Error::other("store gc failed"),
                ))
            } else {
                Ok(())
            }
        }

        async fn add_to_store(
            &self,
            _path: &Path,
        ) -> Result<String, bmc_nix::store::AddToStoreError> {
            unreachable!("BUG: GcStore only serves garbage collection")
        }
    }

    /// What the periodic job asks for: give up on a busy profile and sweep
    /// only when cleanup freed something.
    fn periodic_request() -> PackageGcRequest {
        PackageGcRequest {
            on_busy: bmc_nix::gc::OnBusy::Skip,
            sweep: bmc_nix::gc::Sweep::WhenGenerationsRemoved,
        }
    }

    /// What the upgrade preflight asks for: wait for the profile and sweep
    /// unconditionally.
    fn forced_request() -> PackageGcRequest {
        PackageGcRequest {
            on_busy: bmc_nix::gc::OnBusy::Wait,
            sweep: bmc_nix::gc::Sweep::Always,
        }
    }

    /// Three generations with the newest current: the default policy keeps two
    /// and so has exactly one generation to remove.
    fn profile_with_a_removable_generation(profile_dir: &Path) {
        for number in 1..=3_usize {
            let generation = profile_dir.join(format!("{number}-link"));
            std::fs::create_dir_all(&generation).expect("BUG: create generation");
            std::fs::write(generation.join("manifest"), r#"{"packages":{}}"#)
                .expect("BUG: write manifest");
        }
        std::os::unix::fs::symlink("3-link", profile_dir.join("current"))
            .expect("BUG: link current generation");
    }

    #[tokio::test]
    async fn gc_missing_config_uses_defaults() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

        let outcome = upgrader
            .gc(forced_request())
            .await
            .expect("missing gc config must use default policy");

        assert_eq!(outcome, PackageGcOutcome::Collected);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gc_forwards_a_conditional_sweep() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

        let outcome = upgrader
            .gc(periodic_request())
            .await
            .expect("an empty profile is a successful gc outcome");

        assert_eq!(outcome, PackageGcOutcome::NothingToCollect);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn gc_sweeps_when_cleanup_removed_a_generation() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let config = test_nix_config(dir.path(), &servers);
        std::fs::create_dir(&config.profile_dir).expect("BUG: create profile");
        profile_with_a_removable_generation(&config.profile_dir);
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(config, store);

        let outcome = upgrader
            .gc(periodic_request())
            .await
            .expect("a removed generation must trigger the sweep");

        assert_eq!(outcome, PackageGcOutcome::Collected);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gc_forwards_an_unconditional_sweep() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);
        upgrader
            .gc(forced_request())
            .await
            .expect("the first forced gc must collect");

        let outcome = upgrader
            .gc(forced_request())
            .await
            .expect("a forced gc must collect with nothing to remove");

        assert_eq!(outcome, PackageGcOutcome::Collected);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_disabling_configuration_does_not_stop_backend_collection() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        std::fs::write(dir.path().join("gc.json"), r#"{"periodic":"disabled"}"#)
            .expect("BUG: write gc config");
        let config = test_nix_config(dir.path(), &servers);
        std::fs::create_dir(&config.profile_dir).expect("BUG: create profile");
        profile_with_a_removable_generation(&config.profile_dir);
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(config, store);

        let outcome = upgrader
            .gc(periodic_request())
            .await
            .expect("a disabling configuration must not fail the request");

        assert_eq!(
            outcome,
            PackageGcOutcome::Collected,
            "the toggle is enforced by the periodic job, not the backend"
        );
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gc_reports_a_config_read_failure() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        std::fs::create_dir(dir.path().join("gc.json")).expect("BUG: create config directory");
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

        let err = upgrader
            .gc(forced_request())
            .await
            .expect_err("a directory passed as config must fail to read");

        assert!(matches!(err, PackageGcError::ConfigRead(_)));
        assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn gc_reports_a_config_parse_failure() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        std::fs::write(dir.path().join("gc.json"), "{").expect("BUG: write malformed config");
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

        let err = upgrader
            .gc(forced_request())
            .await
            .expect_err("malformed gc config must fail to parse");

        assert!(matches!(err, PackageGcError::ConfigParse(_)));
        assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn gc_returns_busy_without_collecting() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let config = test_nix_config(dir.path(), &servers);
        let _lock = bmc_nix::profile::try_lock_profile(&config.profile_dir)
            .expect("profile lock attempt must succeed")
            .expect("profile lock must be available");
        let (store, collect_calls) = GcStore::successful();
        let upgrader = PackageUpgrader::with_store(config, store);

        let outcome = upgrader
            .gc(periodic_request())
            .await
            .expect("busy profile is a successful gc outcome");

        assert_eq!(outcome, PackageGcOutcome::Busy);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn gc_reports_a_store_failure_as_operational() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let (store, collect_calls) = GcStore::failing();
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

        let err = upgrader
            .gc(forced_request())
            .await
            .expect_err("store collection failure must be reported");

        assert!(matches!(err, PackageGcError::Operational(_)));
        assert_eq!(err.unswept_removals(), 0);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gc_reports_removals_left_unswept_by_a_store_failure() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let config = test_nix_config(dir.path(), &servers);
        std::fs::create_dir(&config.profile_dir).expect("BUG: create profile");
        profile_with_a_removable_generation(&config.profile_dir);
        let (store, collect_calls) = GcStore::failing();
        let upgrader = PackageUpgrader::with_store(config, store);

        let err = upgrader
            .gc(forced_request())
            .await
            .expect_err("store collection failure must be reported");

        assert!(matches!(
            err,
            PackageGcError::UnsweptRemovals { removed: 1, .. }
        ));
        assert_eq!(err.unswept_removals(), 1);
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
    }

    use std::collections::BTreeMap;

    use bmc_nix::types::{InstalledBy, Manifest, ManifestPackage};

    /// A servers.json holding only a disabled factory and no `servers`,
    /// so the probe short-circuits on "no enabled servers".
    const FACTORY_ONLY: &str = r#"{"factory":{"id":"forge","base_url":"http://x","known_public_key":"k","priority":0,"enabled":false},"servers":[]}"#;

    /// Spawn a throwaway HTTP server that answers every request with the
    /// given JSON body, and return its base URL.
    async fn spawn_index_server(body: String) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock index server");
        let addr = listener.local_addr().expect("BUG: no local addr");
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0_u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// Bind then immediately drop a listener to obtain a port that refuses
    /// connections — a deterministic "dead URL".
    async fn dead_base_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind for dead url");
        let addr = listener.local_addr().expect("BUG: no local addr");
        drop(listener);
        format!("http://{addr}")
    }

    /// Serialize a package index whose `packages` list holds the given
    /// `(name, version, store_path)` entries.
    fn index_json(entries: &[(&str, &str, &str)]) -> String {
        let packages: Vec<String> = entries
            .iter()
            .map(|(name, version, store_path)| {
                format!(r#"{{"name":"{name}","version":"{version}","store_path":"{store_path}"}}"#)
            })
            .collect();
        format!(
            r#"{{"version":{},"provenance":null,"indexes":[],"caches":[],"packages":[{}]}}"#,
            bmc_nix::index::PACKAGE_INDEX_VERSION,
            packages.join(",")
        )
    }

    /// Write a servers.json with one enabled required server pointed at
    /// `base_url`.
    fn write_enabled_server(path: &Path, base_url: &str) {
        let json = format!(
            r#"{{"factory":{{"id":"forge","base_url":"{base_url}","known_public_key":"k","priority":0,"enabled":false}},"servers":[{{"id":"srv","index_url":"{base_url}","known_public_key":"k","priority":10,"enabled":true,"required":true}}]}}"#
        );
        std::fs::write(path, json).expect("BUG: write servers.json");
    }

    /// Write a base manifest holding the given `InstalledBy::System`
    /// packages into generation `1-link` under `profile_dir`.
    fn write_base_manifest(profile_dir: &Path, packages: &[(&str, &str, &str)]) {
        let mut map = BTreeMap::new();
        for (name, version, store_path) in packages {
            map.insert(
                (*name).to_owned(),
                ManifestPackage {
                    version: (*version).to_owned(),
                    store_path: (*store_path).to_owned(),
                    category: None,
                    description: None,
                    upgrade_strategy: None,
                    install_strategy: None,
                    installed_by: InstalledBy::System,
                    installed_from: "srv".to_owned(),
                    pinned: None,
                },
            );
        }
        let generation_dir = profile_dir.join("1-link");
        std::fs::create_dir_all(&generation_dir).expect("BUG: create generation dir");
        bmc_nix::manifest::write_manifest(&generation_dir, &Manifest { packages: map })
            .expect("BUG: write base manifest");
    }

    fn merged_with(
        entries: &[(&str, &str, Option<serde_json::Value>)],
    ) -> bmc_nix::types::MergedIndex {
        let packages: Vec<String> = entries
            .iter()
            .map(|(name, category, metadata)| {
                let meta = metadata
                    .clone()
                    .map_or_else(|| "{}".to_owned(), |m| m.to_string());
                format!(
                    r#"{{"name":"{name}","version":"1.0.0","store_path":"/nix/store/{name}","category":"{category}","metadata":{meta}}}"#
                )
            })
            .collect();
        let json = format!(
            r#"{{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{}]}}"#,
            packages.join(",")
        );
        let raw: bmc_nix::types::PackageIndex =
            serde_json::from_str(&json).expect("BUG: parse index");
        bmc_nix::index::merge_indexes(vec![bmc_nix::types::FetchedIndex {
            server_id: "srv".to_owned(),
            server_priority: 10,
            index: raw,
        }])
    }

    #[test]
    fn resolve_installs_maps_names_to_resolved_packages() {
        let merged = merged_with(&[("widget-weather", "widget", None)]);
        let resolved =
            resolve_installs(&merged, &["widget-weather".to_owned()]).expect("BUG: resolve failed");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "widget-weather");
        assert_eq!(resolved[0].installed_by, bmc_nix::types::InstalledBy::User);
    }

    #[test]
    fn installable_packages_preserves_arbitrary_metadata_and_non_widgets() {
        let metadata =
            serde_json::json!({"future": {"nested": [true, 7, null]}, "service": "metrics"});
        let merged = merged_with(&[
            ("metrics", "service", Some(metadata.clone())),
            ("installed", "widget", None),
        ]);
        let packages = installable_packages_from(
            &merged,
            &std::collections::BTreeSet::from(["installed".to_owned()]),
        );
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "metrics");
        assert_eq!(packages[0].category.as_deref(), Some("service"));
        assert_eq!(serde_json::json!(packages[0].metadata), metadata);
    }

    #[test]
    fn resolve_installs_reports_unknown_target() {
        let merged = merged_with(&[("widget-weather", "widget", None)]);
        let err = resolve_installs(&merged, &["widget-nope".to_owned()])
            .expect_err("BUG: unknown install target must fail");
        assert!(
            matches!(err, PackageProbeError::InstallTargetUnavailable(_)),
            "got {err:?}"
        );
    }

    async fn target_feed_upgrader() -> (tempfile::TempDir, PackageUpgrader) {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let current_index = spawn_index_server(index_json(&[])).await;
        let mut target: serde_json::Value = serde_json::from_str(&index_json(&[
            ("nix", "2.0.0", "/nix/store/target-nix"),
            ("widget-target", "1.0.0", "/nix/store/widget-target"),
        ]))
        .expect("BUG: valid index");
        target["packages"][1]["category"] = serde_json::json!("widget");
        target["packages"][1]["metadata"] = serde_json::json!({"widget": {"uid": "target-only"}});
        let target_index = spawn_index_server(target.to_string()).await;
        let feed = serde_json::json!({
            "version": bmc_nix::feed::PACKAGE_FEED_VERSION,
            "entries": [
                {"bos_version": "current", "download_url": "http://unused/current",
                 "profile_path": "/nix/store/current", "index_url": current_index},
                {"bos_version": "target", "download_url": "http://unused/target",
                 "profile_path": "/nix/store/target", "index_url": target_index}
            ]
        });
        let feed_url = spawn_index_server(feed.to_string()).await;
        let servers = serde_json::json!({
            "factory": {"id": "forge", "base_url": "http://unused", "known_public_key": "k", "priority": 0, "enabled": false},
            "servers": [{
            "id": "release", "feed_url": feed_url, "known_public_key": "k", "priority": 10,
            "enabled": true, "required": true
        }]});
        std::fs::write(&path, servers.to_string()).expect("BUG: write servers");
        write_base_manifest(
            &dir.path().join("profile"),
            &[("nix", "1.0.0", "/nix/store/old-nix")],
        );
        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        (dir, upgrader)
    }

    #[tokio::test]
    async fn probe_selects_target_firmware_packages() {
        let (_dir, upgrader) = target_feed_upgrader().await;
        let PackageProbe::Available(offer) = upgrader
            .probe(
                Some("target"),
                EstimateMode::Skip,
                &["widget-target".to_owned()],
            )
            .await
        else {
            panic!("the target feed must offer the incoming packages");
        };
        let package = offer
            .index
            .packages
            .iter()
            .find(|package| package.name == "nix")
            .expect("BUG: the target must contain nix");
        assert_eq!(package.version.to_string(), "2.0.0");
        assert_eq!(package.store_path, "/nix/store/target-nix");
    }

    #[tokio::test]
    async fn missing_target_feed_fails_the_probe() {
        let (_dir, upgrader) = target_feed_upgrader().await;
        let failure = upgrader
            .probe(Some("missing"), EstimateMode::Skip, &[])
            .await;
        assert!(
            matches!(failure, PackageProbe::Failed(PackageProbeError::IndexUnusable(ref message)) if message.contains("no package feed entry for BOS version 'missing'")),
            "a missing target must fail instead of selecting another firmware: {failure:?}"
        );
    }

    #[tokio::test]
    async fn installable_packages_come_from_target_firmware() {
        let (_dir, upgrader) = target_feed_upgrader().await;
        let widgets = upgrader
            .list_installable_packages(Some("target"))
            .await
            .expect("BUG: target feed must resolve");
        assert_eq!(
            widgets
                .iter()
                .map(|widget| widget.name.as_str())
                .collect::<Vec<_>>(),
            vec!["widget-target"]
        );
        assert!(
            upgrader
                .list_installable_packages(Some("current"))
                .await
                .expect("BUG: current feed must resolve")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn missing_target_feed_fails_package_discovery() {
        let (_dir, upgrader) = target_feed_upgrader().await;
        assert!(
            matches!(upgrader.list_installable_packages(Some("missing")).await, Err(PackageProbeError::IndexUnusable(message)) if message.contains("no package feed entry for BOS version 'missing'"))
        );
    }
    #[tokio::test]
    async fn probe_reports_no_enabled_servers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, FACTORY_ONLY).expect("BUG: write servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        assert!(matches!(
            upgrader.probe(None, EstimateMode::Skip, &[]).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
    }

    #[tokio::test]
    async fn probe_reports_config_unavailable_when_absent() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::ServersConfigUnavailable(_))
            ),
            "got {probe:?}"
        );
        assert!(
            !dir.path().join("servers.json.bcp").exists(),
            "an absent config must not be backed up"
        );
    }

    #[tokio::test]
    async fn probe_recovers_missing_config_from_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(dir.path().join("servers.json.default"), FACTORY_ONLY)
            .expect("BUG: write default");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        assert!(matches!(
            upgrader.probe(None, EstimateMode::Skip, &[]).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
        assert!(
            !dir.path().join("servers.json.bcp").exists(),
            "recovering a missing (not corrupt) config must not back anything up"
        );
        assert!(
            !path.exists(),
            "the default is served in memory and never persisted"
        );
    }

    #[tokio::test]
    async fn probe_recovers_corrupt_config_from_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, "{ not json").expect("BUG: write corrupt");
        std::fs::write(dir.path().join("servers.json.default"), FACTORY_ONLY)
            .expect("BUG: write default");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        assert!(matches!(
            upgrader.probe(None, EstimateMode::Skip, &[]).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
        assert!(
            dir.path().join("servers.json.bcp").exists(),
            "a corrupt config must be backed up to .bcp"
        );
    }

    #[tokio::test]
    async fn probe_reports_config_unavailable_on_corrupt_without_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, "{ not json").expect("BUG: write corrupt");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::ServersConfigUnavailable(_))
            ),
            "got {probe:?}"
        );
        assert!(
            dir.path().join("servers.json.bcp").exists(),
            "a corrupt config must still be backed up to .bcp"
        );
    }

    #[tokio::test]
    async fn probe_reports_missing_system_package() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let base_url = spawn_index_server(index_json(&[])).await;
        write_enabled_server(&path, &base_url);
        write_base_manifest(
            &dir.path().join("profile"),
            &[("nix", "1.0.0", "/nix/store/nix")],
        );

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let PackageProbe::Failed(PackageProbeError::PlanFailed(
            PackagePlanFailure::MissingSystemPackages { names },
        )) = upgrader.probe(None, EstimateMode::Skip, &[]).await
        else {
            panic!("expected a missing-system-package failure");
        };
        assert_eq!(names, vec!["nix".to_owned()]);
        assert_eq!(
            PackagePlanFailure::MissingSystemPackages { names }.to_string(),
            "package source is incomplete; required system package \"nix\" is missing"
        );
    }

    #[tokio::test]
    async fn probe_reports_all_missing_system_packages() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let base_url = spawn_index_server(index_json(&[])).await;
        write_enabled_server(&path, &base_url);
        write_base_manifest(
            &dir.path().join("profile"),
            &[
                ("nix", "1.0.0", "/nix/store/nix"),
                ("core", "1.0.0", "/nix/store/core"),
            ],
        );

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let PackageProbe::Failed(PackageProbeError::PlanFailed(
            PackagePlanFailure::MissingSystemPackages { names },
        )) = upgrader.probe(None, EstimateMode::Skip, &[]).await
        else {
            panic!("expected a missing-system-package failure");
        };
        assert_eq!(names, vec!["core".to_owned(), "nix".to_owned()]);
        assert_eq!(
            PackagePlanFailure::MissingSystemPackages { names }.to_string(),
            "package source is incomplete; required system packages \"core\" and \"nix\" are missing"
        );
    }

    #[tokio::test]
    async fn probe_reports_index_fetch_failed_on_dead_url() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        write_enabled_server(&path, &dead_base_url().await);
        write_base_manifest(
            &dir.path().join("profile"),
            &[("nix", "1.0.0", "/nix/store/nix")],
        );

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::IndexFetchFailed(_))
            ),
            "got {probe:?}"
        );
    }

    #[tokio::test]
    async fn probe_reports_up_to_date_when_index_matches() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let base_url = spawn_index_server(index_json(&[("nix", "1.0.0", "/nix/store/nix")])).await;
        write_enabled_server(&path, &base_url);
        write_base_manifest(
            &dir.path().join("profile"),
            &[("nix", "1.0.0", "/nix/store/nix")],
        );

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        assert!(matches!(
            upgrader.probe(None, EstimateMode::Skip, &[]).await,
            PackageProbe::UpToDate
        ));
    }

    /// Which typed estimate outcome the stubbed store returns, so a probe test
    /// can drive each routing branch without nix or its `internal-json` format.
    #[derive(Debug)]
    enum StubEstimate {
        Downloads(u64),
        Unrealizable(Vec<String>),
        Transient,
    }

    /// A [`StoreOperations`](bmc_nix::store::StoreOperations) that returns a
    /// fixed dry-run estimate outcome.
    #[derive(Debug)]
    struct StubStore(StubEstimate);

    impl bmc_nix::store::StoreOperations for StubStore {
        fn estimate_realization(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> impl std::future::Future<
            Output = Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError>,
        > + Send {
            use std::os::unix::process::ExitStatusExt as _;
            let result = match &self.0 {
                StubEstimate::Downloads(bytes) => Ok(bmc_nix::store::RealizeEstimate {
                    fetch_paths: 1,
                    download_bytes: *bytes,
                    unpacked_bytes: *bytes,
                }),
                StubEstimate::Unrealizable(paths) => {
                    Err(bmc_nix::store::StorePathError::UnsubstitutablePaths {
                        paths: paths.clone(),
                    })
                }
                // A non-zero dry-run exit — dead substituter, inconclusive not
                // doomed — is the transient class that keeps offering the upgrade.
                StubEstimate::Transient => Err(bmc_nix::store::StorePathError::EstimateExited {
                    status: std::process::ExitStatus::from_raw(1 << 8),
                    messages: vec!["error: cannot reach substituter".to_owned()],
                }),
            };
            async move { result }
        }

        fn store_free_bytes(&self, _profile_dir: &std::path::Path) -> std::io::Result<u64> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }

        async fn restore_nar(
            &self,
            _nar: bmc_nix::store::InputStream,
            _target: &Path,
        ) -> Result<(), bmc_nix::store::StoreCommandError> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }

        async fn delete_store_path(
            &self,
            _store_path: &str,
        ) -> Result<(), bmc_nix::store::StoreCommandError> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }

        async fn realize_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
            _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }

        async fn verify_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }

        async fn collect_garbage(
            &self,
            _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
        ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }

        async fn add_to_store(
            &self,
            _path: &Path,
        ) -> Result<String, bmc_nix::store::AddToStoreError> {
            unreachable!("BUG: StubStore serves probe estimates only")
        }
    }

    #[derive(Debug)]
    struct ApplyStore {
        outcome: ApplyStoreOutcome,
    }

    #[derive(Debug)]
    enum ApplyStoreOutcome {
        SpaceRefusal,
        RealizationFailure,
    }

    impl bmc_nix::store::StoreOperations for ApplyStore {
        async fn estimate_realization(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError> {
            Ok(bmc_nix::store::RealizeEstimate {
                fetch_paths: 1,
                download_bytes: 13,
                unpacked_bytes: 17,
            })
        }

        fn store_free_bytes(&self, _profile_dir: &Path) -> std::io::Result<u64> {
            Ok(match self.outcome {
                ApplyStoreOutcome::SpaceRefusal => 11,
                ApplyStoreOutcome::RealizationFailure => u64::MAX,
            })
        }

        async fn realize_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
            _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
        ) -> Result<(), bmc_nix::store::StorePathError> {
            match self.outcome {
                ApplyStoreOutcome::RealizationFailure => {
                    Err(bmc_nix::store::StorePathError::RealiseFailed(
                        std::io::Error::other("scripted realization failure"),
                    ))
                }
                ApplyStoreOutcome::SpaceRefusal => {
                    unreachable!("BUG: a space refusal must precede realization")
                }
            }
        }

        async fn verify_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: apply failure must precede verification")
        }

        async fn collect_garbage(
            &self,
            _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
        ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
            unreachable!("BUG: package apply disables garbage collection")
        }

        async fn add_to_store(
            &self,
            _path: &Path,
        ) -> Result<String, bmc_nix::store::AddToStoreError> {
            unreachable!("BUG: ApplyStore serves indexed installs only")
        }

        async fn restore_nar(
            &self,
            _nar: bmc_nix::store::InputStream,
            _target: &Path,
        ) -> Result<(), bmc_nix::store::StoreCommandError> {
            unreachable!("BUG: ApplyStore serves indexed installs only")
        }

        async fn delete_store_path(
            &self,
            _store_path: &str,
        ) -> Result<(), bmc_nix::store::StoreCommandError> {
            unreachable!("BUG: ApplyStore serves indexed installs only")
        }
    }

    #[derive(Debug)]
    struct SilentProgress;

    impl bmc_nix::upgrade::UpgradeProgress for SilentProgress {
        fn on_phase(&self, _phase: bmc_nix::upgrade::UpgradePhase) {}
        fn on_realization_started(&self, _total_paths: usize) {}
        fn on_realization_finished(&self) {}
        fn on_download_status(&self, _snapshot: &bmc_nix::store::progress::DownloadSnapshot) {}
        fn on_gc_deleted(&self, _deleted_paths: usize) {}
        fn on_gc_finished(&self, _deleted_paths: usize, _freed_bytes: Option<u64>) {}
    }

    async fn apply_one_widget(store: ApplyStore) -> ApplyError {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        write_base_manifest(&dir.path().join("profile"), &[]);
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

        upgrader
            .apply(
                merged_with(&[("widget-weather", "widget", None)]),
                vec!["widget-weather".to_owned()],
                Arc::new(SilentProgress),
            )
            .await
            .expect_err("scripted store must fail package apply")
    }

    #[tokio::test]
    async fn apply_preserves_not_enough_space_fields_and_source() {
        let error = apply_one_widget(ApplyStore {
            outcome: ApplyStoreOutcome::SpaceRefusal,
        })
        .await;

        let ApplyError::Install(
            source @ bmc_nix::upgrade::InstallError::NotEnoughSpace {
                free_bytes,
                required_bytes,
                unpacked_bytes,
            },
        ) = &error
        else {
            panic!("expected a typed not-enough-space install failure, got {error:?}");
        };
        assert_eq!(*free_bytes, 11);
        assert_eq!(*required_bytes, bmc_nix::store::required_with_headroom(17));
        assert_eq!(*unpacked_bytes, 17);
        let preserved = error
            .source()
            .and_then(|source| source.downcast_ref::<bmc_nix::upgrade::InstallError>())
            .expect("BUG: the apply error must expose the original install error as its source");
        assert!(std::ptr::eq(preserved, source));
    }

    #[tokio::test]
    async fn apply_preserves_non_space_install_source() {
        let error = apply_one_widget(ApplyStore {
            outcome: ApplyStoreOutcome::RealizationFailure,
        })
        .await;

        let ApplyError::Install(source @ bmc_nix::upgrade::InstallError::StorePaths(_)) = &error
        else {
            panic!("expected a typed store-path install failure, got {error:?}");
        };
        let preserved = error
            .source()
            .and_then(|source| source.downcast_ref::<bmc_nix::upgrade::InstallError>())
            .expect("BUG: the apply error must expose the original install error as its source");
        assert!(std::ptr::eq(preserved, source));
    }

    #[tokio::test]
    async fn apply_preserves_resolve_failure_source() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let upgrader = PackageUpgrader::with_store(
            test_nix_config(dir.path(), &servers),
            ApplyStore {
                outcome: ApplyStoreOutcome::SpaceRefusal,
            },
        );

        let error = upgrader
            .apply(
                merged_with(&[]),
                vec!["widget-nope".to_owned()],
                Arc::new(SilentProgress),
            )
            .await
            .expect_err("an unavailable install target must fail resolution");
        let ApplyError::Resolve(source) = &error else {
            panic!("expected a typed resolve failure, got {error:?}");
        };
        assert!(matches!(
            source,
            PackageProbeError::InstallTargetUnavailable(_)
        ));
        let preserved = error
            .source()
            .and_then(|source| source.downcast_ref::<PackageProbeError>())
            .expect("BUG: the apply error must expose the original probe error as its source");
        assert!(std::ptr::eq(preserved, source));
    }

    /// Base `nix@1.0.0` with an index offering `nix@1.1.0` yields a non-empty
    /// changed plan, so `EstimateMode::Estimate` actually runs the estimate.
    async fn write_pending_upgrade(dir: &Path, path: &Path) {
        write_base_manifest(&dir.join("profile"), &[("nix", "1.0.0", "/nix/store/nix")]);
        let base_url =
            spawn_index_server(index_json(&[("nix", "1.1.0", "/nix/store/nix-new")])).await;
        write_enabled_server(path, &base_url);
    }

    #[tokio::test]
    async fn probe_fails_unrealizable_when_estimate_needs_unsubstitutable_paths() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        write_pending_upgrade(dir.path(), &path).await;

        let store = StubStore(StubEstimate::Unrealizable(vec![
            "/nix/store/nix-new".to_owned(),
        ]));
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &path), store);

        let probe = upgrader.probe(None, EstimateMode::Estimate, &[]).await;
        let PackageProbe::Failed(PackageProbeError::Unrealizable(paths)) = probe else {
            panic!("an unsubstitutable estimate must fail the probe, got {probe:?}");
        };
        assert_eq!(paths, "/nix/store/nix-new");
    }

    #[tokio::test]
    async fn probe_offers_upgrade_without_size_on_transient_estimate_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        write_pending_upgrade(dir.path(), &path).await;

        let store = StubStore(StubEstimate::Transient);
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &path), store);

        let probe = upgrader.probe(None, EstimateMode::Estimate, &[]).await;
        let PackageProbe::Available(offer) = probe else {
            panic!("a transient estimate error must still offer the upgrade, got {probe:?}");
        };
        assert!(
            offer.preview.download_size_bytes.is_none(),
            "a transient estimate error must omit the download size"
        );
        assert!(
            !offer.preview.changes.is_empty(),
            "the offered upgrade must still carry the changed package"
        );
    }

    #[tokio::test]
    async fn probe_reports_download_size_from_a_successful_estimate() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        write_pending_upgrade(dir.path(), &path).await;

        let store = StubStore(StubEstimate::Downloads(4096));
        let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &path), store);

        let probe = upgrader.probe(None, EstimateMode::Estimate, &[]).await;
        let PackageProbe::Available(offer) = probe else {
            panic!("a successful estimate must offer the upgrade, got {probe:?}");
        };
        assert_eq!(offer.preview.download_size_bytes, Some(4096));
    }
    #[tokio::test]
    async fn probe_reports_install_target_unavailable() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let base_url = spawn_index_server(index_json(&[("nix", "1.0.0", "/nix/store/nix")])).await;
        write_enabled_server(&path, &base_url);
        write_base_manifest(
            &dir.path().join("profile"),
            &[("nix", "1.0.0", "/nix/store/nix")],
        );

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        // The index carries only "nix"; a requested install the index does not
        // list fails the whole probe at the resolve stage.
        let probe = upgrader
            .probe(None, EstimateMode::Skip, &["widget-nope".to_owned()])
            .await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::InstallTargetUnavailable(_))
            ),
            "got {probe:?}"
        );
    }

    #[tokio::test]
    async fn probe_reports_index_unusable_on_garbage_json() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        // The fetch succeeds (HTTP 200) but the body is not a valid index: a
        // parse failure is an unusable index, not a transient fetch failure.
        let base_url = spawn_index_server("{ not an index".to_owned()).await;
        write_enabled_server(&path, &base_url);
        write_base_manifest(
            &dir.path().join("profile"),
            &[("nix", "1.0.0", "/nix/store/nix")],
        );

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::IndexUnusable(_))
            ),
            "got {probe:?}"
        );
    }

    #[tokio::test]
    async fn probe_reports_manifest_read_failed_when_manifest_malformed() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let base_url = spawn_index_server(index_json(&[("nix", "1.0.0", "/nix/store/nix")])).await;
        write_enabled_server(&path, &base_url);
        // A present but unparseable current manifest is a read failure. An
        // absent profile is not: read_latest_manifest falls back to an empty
        // manifest, so the probe would report UpToDate instead.
        let current = dir.path().join("profile/current");
        std::fs::create_dir_all(&current).expect("BUG: create current dir");
        std::fs::write(current.join("manifest"), "{ not a manifest").expect("BUG: write manifest");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::ManifestReadFailed(_))
            ),
            "got {probe:?}"
        );
    }

    #[tokio::test]
    async fn list_installable_packages_reports_no_enabled_servers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, FACTORY_ONLY).expect("BUG: write servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        assert!(matches!(
            upgrader.list_installable_packages(None).await,
            Err(PackageProbeError::NoEnabledServers)
        ));
    }
}

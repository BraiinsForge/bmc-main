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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bmc_nix::store::ESTIMATE_TIMEOUT;
use serde::Deserialize;
use tracing::{info, warn};

/// Package-index fetches run under the upgrade run gate: a hung index
/// server must time out instead of wedging every check/start in
/// `UpgradeInProgress`.
const INDEX_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct NixUpgradeConfig {
    pub servers_config_path: PathBuf,
    pub gc_config_path: PathBuf,
    pub profile_dir: PathBuf,
    pub hooks_dir: String,
    pub hooks_override_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SystemPackageChange {
    pub name: String,
    pub version_from: Option<String>,
    pub version_to: Option<String>,
    pub category: Option<String>,
    pub changelog: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PackagesPreview {
    pub changes: Vec<SystemPackageChange>,
    pub download_size_bytes: Option<u64>,
    /// Unpacked (NAR) size the realization would add to the store.
    pub unpacked_size_bytes: Option<u64>,
    pub bmc_version: Option<String>,
    pub bmc_changelog: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallablePreview {
    pub image: String,
    /// Scene size the preview depicts (the `assets.previews` map key). Kept as
    /// a free-form string so a size a newer index introduces still round-trips.
    pub size: String,
}

/// Re-exported so consumers can name the known categories without depending
/// on `bmc-widget-manifest` directly.
pub use bmc_widget_manifest::WidgetCategory;

/// Catalog category of an installable widget, read from a package index.
///
/// Locally-authored manifests only ever carry the known [`WidgetCategory`]
/// values, but an index produced by a newer release may list a category this
/// build does not recognize. Unrecognized (or absent) values become
/// [`Self::Unknown`] so one new category cannot break listing the rest of the
/// catalog — mirroring how the index's strategy hints tolerate unknown values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallableCategory {
    Known(WidgetCategory),
    Unknown,
}

impl<'de> Deserialize<'de> for InstallableCategory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(WidgetCategory::deserialize(
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new(&raw),
        )
        .map_or(Self::Unknown, Self::Known))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallableWidget {
    pub package_name: String,
    pub uid: String,
    pub version: String,
    pub display_name: String,
    pub subname: Option<String>,
    pub category: InstallableCategory,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub previews: Vec<InstallablePreview>,
    pub supported_viewports: Vec<bmc_widget_manifest::WidgetViewportConstraint>,
}

#[derive(Clone, Copy, Debug)]
pub enum EstimateMode {
    Estimate,
    Skip,
}

#[derive(Debug)]
pub enum PackageProbe {
    Available(bmc_nix::types::MergedIndex, PackagesPreview),
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

/// A package upgrade application failure,
/// carrying the display message of the underlying error.
///
/// A full store is kept distinct so callers can blame the user's disk
/// rather than the daemon.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("{0}")]
    NotEnoughSpace(String),
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
    async fn probe(&self, estimate: EstimateMode, install: &[String]) -> PackageProbe;
    async fn apply(
        &self,
        merged: bmc_nix::types::MergedIndex,
        install: Vec<String>,
        progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError>;
    async fn list_installable_widgets(&self) -> Result<Vec<InstallableWidget>, PackageProbeError>;
    /// Free bytes on the filesystem holding the package store.
    fn store_free_bytes(&self) -> std::io::Result<u64>;
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
        }
    }

    /// Shared prelude for `probe` and `list_installable_widgets`: load the
    /// servers config, require an enabled server, fetch and merge the package
    /// indexes, and read the profile manifest (current, falling back to
    /// latest). Errors are mapped to `PackageProbeError`; each caller adapts
    /// them to its own return shape.
    async fn fetch_index_and_manifest(
        &self,
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

        // Feed servers resolve their exact index server-side, keyed by the
        // running firmware version; plain index servers need no scope.
        let firmware_scope = if servers.iter().any(|server| {
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

    async fn probe(&self, estimate: EstimateMode, install: &[String]) -> PackageProbe {
        let (merged, base) = match self.fetch_index_and_manifest().await {
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

        PackageProbe::Available(merged, build_packages_preview(&plan, realize_estimate))
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
        let installs = resolve_installs(&merged, &install)
            .map_err(|err| ApplyError::Failed(err.to_string()))?;
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
        .map_err(|err| {
            let message = err.to_string();
            if matches!(err, bmc_nix::upgrade::InstallError::NotEnoughSpace { .. }) {
                ApplyError::NotEnoughSpace(message)
            } else {
                ApplyError::Failed(message)
            }
        })
    }

    async fn list_installable_widgets(&self) -> Result<Vec<InstallableWidget>, PackageProbeError> {
        let (merged, base) = self.fetch_index_and_manifest().await?;
        let installed = base.packages.keys().cloned().collect();
        Ok(installable_widgets_from(&merged, &installed))
    }

    fn store_free_bytes(&self) -> std::io::Result<u64> {
        // The profile directory lives on the same filesystem as the store.
        bmc_nix::store::statvfs_free_bytes(&self.config.profile_dir)
    }
}

/// Discover installable widgets from a merged index: every name not
/// already in the profile that resolves to a `category == "widget"`
/// package, mapped from the resolved entry's `metadata` picker fields.
/// Resolving (rather than reading a raw entry) makes the listed version
/// and metadata match exactly what installing the name would land.
#[must_use]
pub fn installable_widgets_from(
    merged: &bmc_nix::types::MergedIndex,
    installed: &std::collections::BTreeSet<String>,
) -> Vec<InstallableWidget> {
    let widget_str = |resolved: &bmc_nix::types::ResolvedPackage, key: &str| {
        resolved
            .metadata
            .get("widget")
            .and_then(|w| w.get(key))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    };
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
            if resolved.category.as_deref() != Some("widget") {
                return None;
            }
            // `uid` is load-bearing (the frontend places the widget into a
            // scene by it); a widget missing it is useless, so drop it rather
            // than publish an empty uid.
            let uid = widget_str(&resolved, "uid")?;
            Some(InstallableWidget {
                uid,
                display_name: widget_str(&resolved, "display_name")
                    .unwrap_or_else(|| resolved.name.clone()),
                subname: widget_str(&resolved, "subname"),
                category: resolved
                    .metadata
                    .get("widget")
                    .and_then(|w| w.get("category"))
                    .and_then(|c| InstallableCategory::deserialize(c).ok())
                    .unwrap_or(InstallableCategory::Unknown),
                icon: resolved
                    .metadata
                    .get("assets")
                    .and_then(|a| a.get("icon"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                previews: resolved
                    .metadata
                    .get("assets")
                    .and_then(|a| a.get("previews"))
                    .and_then(serde_json::Value::as_object)
                    .map(|by_size| {
                        by_size
                            .iter()
                            .filter_map(|(size, image)| {
                                image.as_str().map(|image| InstallablePreview {
                                    image: image.to_owned(),
                                    size: size.clone(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                supported_viewports: resolved
                    .metadata
                    .get("widget")
                    .and_then(|widget| widget.get("supported_viewports"))
                    .and_then(|value| {
                        Vec::<bmc_widget_manifest::WidgetViewportConstraint>::deserialize(value)
                            .ok()
                    })
                    .unwrap_or_default(),
                package_name: resolved.name,
                version: resolved.version,
                description: resolved.description,
            })
        })
        .collect()
}

/// Resolve requested install names against the merged index into
/// user-installed [`ResolvedPackage`]s for the plan/apply add-set.
///
/// Any package name resolves here, not only `category == "widget"`:
/// installing arbitrary packages is a supported capability of the backend.
/// The widget-only restriction is a presentation concern — the picker
/// surfaces just widgets via [`installable_widgets_from`] — so today users
/// can only reach widget installs, but the plan/apply path deliberately
/// imposes no such limit.
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
    fn resolve_installs_reports_unknown_target() {
        let merged = merged_with(&[("widget-weather", "widget", None)]);
        let err = resolve_installs(&merged, &["widget-nope".to_owned()])
            .expect_err("BUG: unknown install target must fail");
        assert!(
            matches!(err, PackageProbeError::InstallTargetUnavailable(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn installable_widgets_keeps_uninstalled_widget_category_only() {
        let merged = merged_with(&[
            (
                "widget-weather",
                "widget",
                Some(serde_json::json!({
                    "widget": {"uid": "uid-weather", "display_name": "Weather", "subname": "Forecast", "category": "info"},
                    "assets": {"icon": "/nix/store/widget-weather/lib/bmc-widgets/weather/icon.svg"}
                })),
            ),
            (
                "widget-clock",
                "widget",
                Some(serde_json::json!({
                    "widget": {"uid": "uid-clock", "display_name": "Clock", "category": "clock"}
                })),
            ),
            ("core", "system", None),
        ]);
        let installed: std::collections::BTreeSet<String> =
            ["widget-clock".to_owned(), "core".to_owned()]
                .into_iter()
                .collect();

        let widgets = installable_widgets_from(&merged, &installed);

        assert_eq!(widgets.len(), 1, "only the uninstalled widget survives");
        let w = &widgets[0];
        assert_eq!(w.package_name, "widget-weather");
        assert_eq!(w.uid, "uid-weather");
        assert_eq!(w.display_name, "Weather");
        assert_eq!(w.subname.as_deref(), Some("Forecast"));
        // "info" is not a category this build knows, so it folds to Unknown
        // rather than dropping the widget from the catalog.
        assert_eq!(w.category, InstallableCategory::Unknown);
        assert_eq!(
            w.icon.as_deref(),
            Some("/nix/store/widget-weather/lib/bmc-widgets/weather/icon.svg")
        );
        // No `assets.previews` in the index, so the preview list defaults empty.
        assert!(w.previews.is_empty());
    }

    #[test]
    fn installable_widgets_read_supported_viewports_from_index() {
        let merged = merged_with(&[(
            "widget-fullscreen",
            "widget",
            Some(serde_json::json!({
                "widget": {
                    "uid": "uid-fullscreen",
                    "supported_viewports": [{
                        "type": "rectangular",
                        "min_width": 1280,
                        "max_width": 1280,
                        "min_height": 480,
                        "max_height": 480
                    }]
                }
            })),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert_eq!(
            widgets[0].supported_viewports,
            vec![bmc_widget_manifest::WidgetViewportConstraint {
                viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
                min_width: Some(1280),
                max_width: Some(1280),
                min_height: Some(480),
                max_height: Some(480),
                min_dpi: None,
                max_dpi: None,
            }]
        );
    }

    #[test]
    fn installable_widgets_default_missing_supported_viewports_to_empty() {
        let merged = merged_with(&[(
            "widget-legacy",
            "widget",
            Some(serde_json::json!({"widget": {"uid": "uid-legacy"}})),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert!(widgets[0].supported_viewports.is_empty());
    }

    #[test]
    fn installable_widgets_default_invalid_supported_viewports_to_empty() {
        let merged = merged_with(&[(
            "widget-invalid",
            "widget",
            Some(serde_json::json!({
                "widget": {"uid": "uid-invalid", "supported_viewports": "full"}
            })),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert!(widgets[0].supported_viewports.is_empty());
    }

    #[test]
    fn installable_widgets_reads_previews_from_index() {
        // Preview art lives under `assets.previews` in the index (a not-yet
        // installed widget has no parsed manifest to read it from), keyed by the
        // scene size it depicts; each entry becomes one `InstallablePreview`.
        let merged = merged_with(&[(
            "widget-weather",
            "widget",
            Some(serde_json::json!({
                "widget": {"uid": "uid-weather", "display_name": "Weather", "category": "weather"},
                "assets": {
                    "icon": "/nix/store/w/icon.svg",
                    "previews": {
                        "full": "https://example.test/weather-full.png",
                        "medium": "https://example.test/weather-medium.png"
                    }
                }
            })),
        )]);

        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());

        assert_eq!(widgets.len(), 1);
        let by_size: std::collections::BTreeMap<&str, &str> = widgets[0]
            .previews
            .iter()
            .map(|p| (p.size.as_str(), p.image.as_str()))
            .collect();
        assert_eq!(
            by_size,
            std::collections::BTreeMap::from([
                ("full", "https://example.test/weather-full.png"),
                ("medium", "https://example.test/weather-medium.png"),
            ])
        );
    }

    #[test]
    fn installable_category_deserializes_known_and_unknown() {
        let known: InstallableCategory =
            serde_json::from_value(serde_json::json!("weather")).expect("BUG: known category");
        assert_eq!(known, InstallableCategory::Known(WidgetCategory::Weather));

        // A category value a newer index might carry that this build predates.
        let unknown: InstallableCategory =
            serde_json::from_value(serde_json::json!("teleportation")).expect("BUG: unknown ok");
        assert_eq!(unknown, InstallableCategory::Unknown);
    }

    #[test]
    fn installable_widgets_drops_widget_without_uid() {
        // `uid` is load-bearing; a widget package whose metadata lacks it must
        // not be offered, rather than surfacing with an empty uid.
        let merged = merged_with(&[(
            "widget-broken",
            "widget",
            Some(serde_json::json!({
                "widget": {"display_name": "Broken", "category": "info"}
            })),
        )]);
        let widgets = installable_widgets_from(&merged, &std::collections::BTreeSet::new());
        assert!(widgets.is_empty(), "a widget without a uid must be dropped");
    }

    #[tokio::test]
    async fn probe_reports_no_enabled_servers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, FACTORY_ONLY).expect("BUG: write servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        assert!(matches!(
            upgrader.probe(EstimateMode::Skip, &[]).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
    }

    #[tokio::test]
    async fn probe_reports_config_unavailable_when_absent() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(EstimateMode::Skip, &[]).await;
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
            upgrader.probe(EstimateMode::Skip, &[]).await,
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
            upgrader.probe(EstimateMode::Skip, &[]).await,
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

        let probe = upgrader.probe(EstimateMode::Skip, &[]).await;
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
        )) = upgrader.probe(EstimateMode::Skip, &[]).await
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
        )) = upgrader.probe(EstimateMode::Skip, &[]).await
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

        let probe = upgrader.probe(EstimateMode::Skip, &[]).await;
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
            upgrader.probe(EstimateMode::Skip, &[]).await,
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

        let probe = upgrader.probe(EstimateMode::Estimate, &[]).await;
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

        let probe = upgrader.probe(EstimateMode::Estimate, &[]).await;
        let PackageProbe::Available(_, preview) = probe else {
            panic!("a transient estimate error must still offer the upgrade, got {probe:?}");
        };
        assert!(
            preview.download_size_bytes.is_none(),
            "a transient estimate error must omit the download size"
        );
        assert!(
            !preview.changes.is_empty(),
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

        let probe = upgrader.probe(EstimateMode::Estimate, &[]).await;
        let PackageProbe::Available(_, preview) = probe else {
            panic!("a successful estimate must offer the upgrade, got {probe:?}");
        };
        assert_eq!(preview.download_size_bytes, Some(4096));
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
            .probe(EstimateMode::Skip, &["widget-nope".to_owned()])
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

        let probe = upgrader.probe(EstimateMode::Skip, &[]).await;
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

        let probe = upgrader.probe(EstimateMode::Skip, &[]).await;
        assert!(
            matches!(
                probe,
                PackageProbe::Failed(PackageProbeError::ManifestReadFailed(_))
            ),
            "got {probe:?}"
        );
    }

    #[tokio::test]
    async fn list_installable_widgets_reports_no_enabled_servers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, FACTORY_ONLY).expect("BUG: write servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        // list_installable_widgets duplicates probe's server-config prologue,
        // so it must reject a config with no enabled servers the same way.
        assert!(matches!(
            upgrader.list_installable_widgets().await,
            Err(PackageProbeError::NoEnabledServers)
        ));
    }
}

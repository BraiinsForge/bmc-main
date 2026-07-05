// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

/// Package-index fetches run under the upgrade run gate: a hung index
/// server must time out instead of wedging every check/start in
/// `UpgradeInProgress`.
const INDEX_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// The dry-run size estimate spawns `nix-store`, which may consult
/// substituters over the network — under the same run gate as the index
/// fetch, so a hung probe must time out too. On timeout the preview just
/// omits the download size.
const ESTIMATE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct NixUpgradeConfig {
    pub servers_config_path: PathBuf,
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
    pub bmc_version: Option<String>,
    pub bmc_changelog: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum EstimateMode {
    Estimate,
    Skip,
}

#[derive(Debug)]
pub enum PackageProbe {
    Available(bmc_nix::types::MergedIndex, PackagesPreview),
    /// No servers, no plan, planning error — packages simply not offered.
    Unavailable,
    /// The index fetch itself failed transiently
    /// (`FetchIndexesError::Fetch { .. }`) — still "unavailable" on the
    /// wire, but autoupgrade may retry.
    FetchFailed(String),
}

/// A package upgrade application failure, carrying the display message of
/// the underlying error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ApplyError(pub String);

/// Mockability seam for the package half of system upgrades: probing for
/// available package changes and applying them.
#[async_trait::async_trait]
pub trait PackageBackend: Send + Sync + std::fmt::Debug + 'static {
    async fn probe(&self, estimate: EstimateMode) -> PackageProbe;
    async fn apply(
        &self,
        merged: bmc_nix::types::MergedIndex,
        progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError>;
}

/// The nix-backed [`PackageBackend`]: fetches package indexes over HTTP,
/// reads the profile manifest, and applies profile changes through
/// `bmc-nix`.
#[derive(Debug)]
pub struct PackageUpgrader {
    config: NixUpgradeConfig,
    client: reqwest::Client,
}

impl PackageUpgrader {
    #[must_use]
    pub fn new(config: NixUpgradeConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(INDEX_FETCH_TIMEOUT)
            .build()
            .expect("BUG: client builder failed");
        Self { config, client }
    }
}

#[async_trait::async_trait]
impl PackageBackend for PackageUpgrader {
    async fn probe(&self, estimate: EstimateMode) -> PackageProbe {
        if let Some(parent) = self.config.servers_config_path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            warn!(error = %err, "Failed to create the servers config directory");
        }

        let servers = match std::fs::read_to_string(&self.config.servers_config_path) {
            Ok(contents) => {
                match serde_json::from_str::<bmc_nix::types::ServersConfig>(&contents) {
                    Ok(config) => config.servers,
                    Err(err) => {
                        warn!(error = %err, "Servers config is unparseable, packages unavailable");
                        return PackageProbe::Unavailable;
                    }
                }
            }
            Err(err) => {
                warn!(error = %err, "Servers config is unreadable, packages unavailable");
                return PackageProbe::Unavailable;
            }
        };

        if !servers.iter().any(|server| server.enabled) {
            warn!("No enabled package servers, packages unavailable");
            return PackageProbe::Unavailable;
        }

        let merged = match bmc_nix::index::fetch_and_merge_indexes(&self.client, &servers).await {
            Ok(merged) => merged,
            Err(err @ bmc_nix::index::FetchIndexesError::Fetch { .. }) => {
                warn!(error = %err, "Package index fetch failed");
                return PackageProbe::FetchFailed(err.to_string());
            }
            Err(err) => {
                warn!(error = %err, "Package index unusable, packages unavailable");
                return PackageProbe::Unavailable;
            }
        };

        let base = match bmc_nix::manifest::read_current_manifest(&self.config.profile_dir) {
            Ok(manifest) => manifest,
            Err(bmc_nix::manifest::ReadManifestError::CurrentNotFound { .. }) => {
                match bmc_nix::manifest::read_latest_manifest(&self.config.profile_dir) {
                    Ok(manifest) => manifest,
                    Err(err) => {
                        warn!(error = %err, "Failed to read the profile manifest, packages unavailable");
                        return PackageProbe::Unavailable;
                    }
                }
            }
            Err(err) => {
                warn!(error = %err, "Failed to read the profile manifest, packages unavailable");
                return PackageProbe::Unavailable;
            }
        };

        let plan = match bmc_nix::manifest::compute_upgrade_plan(&base, Some(&merged), &[], &[]) {
            Ok(plan) => plan,
            Err(err) => {
                warn!(error = %err, "Failed to plan the package upgrade, packages unavailable");
                return PackageProbe::Unavailable;
            }
        };

        if plan.changed.is_empty() && plan.added.is_empty() && plan.removed.is_empty() {
            info!("Packages are up to date");
            return PackageProbe::Unavailable;
        }

        let download_size_bytes = match estimate {
            EstimateMode::Estimate => {
                // The timeout drops the estimate future, which kills the
                // spawned `nix-store` (tokio's `output()` kills on drop).
                match tokio::time::timeout(
                    ESTIMATE_TIMEOUT,
                    bmc_nix::store::estimate_realization(
                        &bmc_nix::store::TokioCommandRunner,
                        &plan.packages,
                    ),
                )
                .await
                {
                    Ok(Ok(realize_estimate)) => Some(realize_estimate.download_bytes),
                    Ok(Err(err)) => {
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

        PackageProbe::Available(merged, build_packages_preview(&plan, download_size_bytes))
    }

    async fn apply(
        &self,
        merged: bmc_nix::types::MergedIndex,
        progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        bmc_nix::upgrade::apply_profile_change(
            &self.config.profile_dir,
            None, // base manifest is re-read under the profile lock
            Some(&merged),
            &[],
            &[],
            bmc_nix::upgrade::ActivationMode::Activate,
            None, // GC is disabled on the packages path
            Some(progress.as_ref()),
            &self.config.hooks_dir,
            self.config.hooks_override_path.as_deref(),
        )
        .await
        .map(|_| ())
        .map_err(|err| ApplyError(err.to_string()))
    }
}

#[must_use]
pub fn build_packages_preview(
    plan: &bmc_nix::types::UpgradePlan,
    download_size_bytes: Option<u64>,
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
            changelog: resolved.and_then(|package| package.metadata.get("changelog").cloned()),
        });
    }
    for added in &plan.added {
        let resolved = resolved_by_name.get(added.name.as_str());
        changes.push(SystemPackageChange {
            name: added.name.clone(),
            version_from: None,
            version_to: Some(added.version.clone()),
            category: resolved.and_then(|package| package.category.clone()),
            changelog: resolved.and_then(|package| package.metadata.get("changelog").cloned()),
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
    let bmc_version = core.and_then(|package| package.metadata.get("bmc_version").cloned());
    let bmc_changelog = core.and_then(|package| package.metadata.get("changelog").cloned());

    PackagesPreview {
        changes,
        download_size_bytes,
        bmc_version,
        bmc_changelog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_nix_config(root: &Path, servers: &Path) -> NixUpgradeConfig {
        NixUpgradeConfig {
            servers_config_path: servers.to_path_buf(),
            profile_dir: root.join("profile"),
            hooks_dir: "hooks".to_owned(),
            hooks_override_path: None,
        }
    }

    #[tokio::test]
    async fn probe_reports_unavailable_when_no_enabled_servers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(
            &path,
            r#"{"factory":{"id":"factory","base_url":"http://x","known_public_key":"k","priority":0,"enabled":false},"servers":[]}"#,
        )
        .expect("BUG: write servers.json");
        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));
        assert!(matches!(
            upgrader.probe(EstimateMode::Skip).await,
            PackageProbe::Unavailable
        ));
    }

    #[tokio::test]
    async fn probe_reports_unavailable_when_servers_json_missing() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let upgrader =
            PackageUpgrader::new(test_nix_config(dir.path(), &dir.path().join("absent.json")));
        assert!(matches!(
            upgrader.probe(EstimateMode::Skip).await,
            PackageProbe::Unavailable
        ));
    }
}

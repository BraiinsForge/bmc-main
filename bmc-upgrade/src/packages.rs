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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallablePreview {
    pub image: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallableWidget {
    pub package_name: String,
    pub uid: String,
    pub version: String,
    pub display_name: String,
    pub subname: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub previews: Vec<InstallablePreview>,
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
        }
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
    async fn list_installable_widgets(&self) -> Result<Vec<InstallableWidget>, PackageProbeError>;
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
        let config =
            match bmc_nix::servers_config::load_servers_config(&self.config.servers_config_path) {
                Ok(config) => config,
                Err(err) => {
                    warn!(error = %err, "Servers config unavailable");
                    return PackageProbe::Failed(PackageProbeError::ServersConfigUnavailable(
                        err.to_string(),
                    ));
                }
            };
        let servers = config.servers;

        if !servers.iter().any(|server| server.enabled) {
            warn!("No enabled package servers");
            return PackageProbe::Failed(PackageProbeError::NoEnabledServers);
        }

        let merged = match bmc_nix::index::fetch_and_merge_indexes(&self.client, &servers).await {
            Ok(merged) => merged,
            Err(err @ bmc_nix::index::FetchIndexesError::Fetch { .. }) => {
                warn!(error = %err, "Package index fetch failed");
                return PackageProbe::Failed(PackageProbeError::IndexFetchFailed(err.to_string()));
            }
            Err(err) => {
                warn!(error = %err, "Package index unusable");
                return PackageProbe::Failed(PackageProbeError::IndexUnusable(err.to_string()));
            }
        };

        let base = match bmc_nix::manifest::read_current_manifest(&self.config.profile_dir) {
            Ok(manifest) => manifest,
            Err(bmc_nix::manifest::ReadManifestError::CurrentNotFound { .. }) => {
                match bmc_nix::manifest::read_latest_manifest(&self.config.profile_dir) {
                    Ok(manifest) => manifest,
                    Err(err) => {
                        warn!(error = %err, "Failed to read the profile manifest");
                        return PackageProbe::Failed(PackageProbeError::ManifestReadFailed(
                            err.to_string(),
                        ));
                    }
                }
            }
            Err(err) => {
                warn!(error = %err, "Failed to read the profile manifest");
                return PackageProbe::Failed(PackageProbeError::ManifestReadFailed(
                    err.to_string(),
                ));
            }
        };

        let plan = match bmc_nix::manifest::compute_upgrade_plan(&base, Some(&merged), &[], &[]) {
            Ok(plan) => plan,
            Err(bmc_nix::manifest::ComputeUpgradePlanError::MissingSystemPackages { names }) => {
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

    async fn list_installable_widgets(&self) -> Result<Vec<InstallableWidget>, PackageProbeError> {
        let config =
            match bmc_nix::servers_config::load_servers_config(&self.config.servers_config_path) {
                Ok(config) => config,
                Err(err) => {
                    warn!(error = %err, "Servers config unavailable");
                    return Err(PackageProbeError::ServersConfigUnavailable(err.to_string()));
                }
            };
        let servers = config.servers;

        if !servers.iter().any(|server| server.enabled) {
            warn!("No enabled package servers");
            return Err(PackageProbeError::NoEnabledServers);
        }

        let merged = match bmc_nix::index::fetch_and_merge_indexes(&self.client, &servers).await {
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
                match bmc_nix::manifest::read_latest_manifest(&self.config.profile_dir) {
                    Ok(manifest) => manifest,
                    Err(err) => {
                        warn!(error = %err, "Failed to read the profile manifest");
                        return Err(PackageProbeError::ManifestReadFailed(err.to_string()));
                    }
                }
            }
            Err(err) => {
                warn!(error = %err, "Failed to read the profile manifest");
                return Err(PackageProbeError::ManifestReadFailed(err.to_string()));
            }
        };

        let installed = base.packages.keys().cloned().collect();
        Ok(installable_widgets_from(&merged, &installed))
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
                category: widget_str(&resolved, "category"),
                icon: resolved
                    .metadata
                    .get("assets")
                    .and_then(|a| a.get("icon"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                package_name: resolved.name,
                version: resolved.version,
                description: resolved.description,
                previews: Vec::new(),
            })
        })
        .collect()
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
            r#"{{"factory":{{"id":"forge","base_url":"{base_url}","known_public_key":"k","priority":0,"enabled":false}},"servers":[{{"id":"srv","type":"mirror","base_url":"{base_url}","known_public_key":"k","priority":10,"enabled":true,"required":true}}]}}"#
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
        assert_eq!(w.category.as_deref(), Some("info"));
        assert_eq!(
            w.icon.as_deref(),
            Some("/nix/store/widget-weather/lib/bmc-widgets/weather/icon.svg")
        );
        assert!(w.previews.is_empty());
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
            upgrader.probe(EstimateMode::Skip).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
    }

    #[tokio::test]
    async fn probe_reports_config_unavailable_when_absent() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");

        let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

        let probe = upgrader.probe(EstimateMode::Skip).await;
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
            upgrader.probe(EstimateMode::Skip).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
        assert!(
            !dir.path().join("servers.json.bcp").exists(),
            "recovering a missing (not corrupt) config must not back anything up"
        );
        assert!(path.exists(), "the recovered config must be persisted");
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
            upgrader.probe(EstimateMode::Skip).await,
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

        let probe = upgrader.probe(EstimateMode::Skip).await;
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
        )) = upgrader.probe(EstimateMode::Skip).await
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
        )) = upgrader.probe(EstimateMode::Skip).await
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

        let probe = upgrader.probe(EstimateMode::Skip).await;
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
            upgrader.probe(EstimateMode::Skip).await,
            PackageProbe::UpToDate
        ));
    }
}

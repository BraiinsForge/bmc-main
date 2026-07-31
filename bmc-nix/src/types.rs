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

use std::collections::BTreeMap;
use std::path::PathBuf;

use semver::Version;
use serde::{Deserialize, Serialize};

/// Which existing generation the caller wants to diff the new
/// generation against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseSelector {
    /// Use the manifest of the `current` symlink; fall back to
    /// `latest` when `current` is missing (applied by
    /// `apply_profile_change`, not by parsing).
    Current,
    /// Use the manifest of the highest-numbered generation.
    Latest,
    /// Use the manifest of a specific generation number.
    Generation(usize),
}

/// Error returned when a `BaseSelector` fails to parse from a string.
#[derive(Debug, thiserror::Error)]
#[error("invalid base selector `{0}` (expected `current`, `latest`, or a positive integer)")]
pub struct BaseSelectorParseError(String);

impl std::str::FromStr for BaseSelector {
    type Err = BaseSelectorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "current" => Ok(Self::Current),
            "latest" => Ok(Self::Latest),
            _ => {
                let n: usize = s
                    .parse()
                    .map_err(|_| BaseSelectorParseError(s.to_owned()))?;
                if n == 0 {
                    return Err(BaseSelectorParseError(s.to_owned()));
                }
                Ok(Self::Generation(n))
            }
        }
    }
}

/// Remote package index (`nix-package-index.v1.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageIndex {
    pub version: u32,
    pub provenance: Option<Provenance>,
    pub indexes: Vec<String>,
    pub caches: Vec<CacheEntry>,
    pub packages: Vec<PackageEntry>,
}

/// Provenance information for an index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub commit: String,
}

/// A single cache entry from the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub name: String,
    pub cache_url: String,
    pub cache_key: String,
}

/// Upgrade strategy hints for UI and orchestration.
///
/// A newer server may publish strategy values this build does not know;
/// they deserialize to [`Self::Unknown`] so one unrecognized hint cannot
/// fail the entire index parse and hide every upgrade.
///
/// Invariant: display-only. Never branch on a strategy value for control flow
/// (resolution or the reboot/apply decision). [`Self::Unknown`] round-trips
/// lossily, and a package dropped from every index carries its old strategy
/// forward across upgrades — both are safe only while nothing reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpgradeStrategy {
    Reboot,
    #[serde(other)]
    Unknown,
}

/// Install strategy hints for UI and orchestration.
///
/// A newer server may publish strategy values this build does not know;
/// they deserialize to [`Self::Unknown`] so one unrecognized hint cannot
/// fail the entire index parse and hide every upgrade.
///
/// Invariant: display-only. Never branch on a strategy value for control flow
/// (resolution or the reboot/apply decision). [`Self::Unknown`] round-trips
/// lossily, and a package dropped from every index carries its old strategy
/// forward across upgrades — both are safe only while nothing reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallStrategy {
    Reboot,
    #[serde(other)]
    Unknown,
}

/// A package entry as it appears in the remote index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub cache: Option<String>,
    pub store_path: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub upgrade_strategy: Option<UpgradeStrategy>,
    #[serde(default)]
    pub install_strategy: Option<InstallStrategy>,
    /// Populated during index merging, not in JSON
    #[serde(skip)]
    pub server_id: String,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

/// A fully resolved package ready for installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub store_path: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub upgrade_strategy: Option<UpgradeStrategy>,
    #[serde(default)]
    pub install_strategy: Option<InstallStrategy>,
    pub installed_by: InstalledBy,
    pub installed_from: String,
    #[serde(default)]
    pub pinned: Option<String>,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

/// What initiated the installation of a package
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstalledBy {
    System,
    User,
}

/// Profile manifest (stored in each generation)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub packages: BTreeMap<String, ManifestPackage>,
}

/// Map the legacy `"pinned": "none"` sentinel to `None`.
///
/// Pre-branch manifests stored `pinned` as an enum whose `None` variant
/// serialized to the string `"none"`. Those persisted manifests must read
/// back as unpinned; real constraints and `null`/absent are left intact.
fn deserialize_legacy_pin<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(d)?;
    Ok(raw.filter(|v| v != "none"))
}

/// Per-package manifest entry.
///
/// Cache metadata is not persisted here — it lives only in `PackageIndex`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPackage {
    pub version: String,
    pub store_path: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub upgrade_strategy: Option<UpgradeStrategy>,
    #[serde(default)]
    pub install_strategy: Option<InstallStrategy>,
    pub installed_by: InstalledBy,
    pub installed_from: String,
    #[serde(default, deserialize_with = "deserialize_legacy_pin")]
    pub pinned: Option<String>,
}

/// Profile generation metadata
#[derive(Debug, Clone)]
pub struct ProfileGeneration {
    pub number: usize,
    pub path: PathBuf,
    pub manifest: Manifest,
}

/// Server registry (`/etc/nix-upgrade/servers.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServersConfig {
    pub factory: FactoryServerEntry,
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    /// Set only when `register-server` bootstrapped this config on a
    /// device that had neither a runtime nor a default config; gates
    /// the synchronized factory + server-entry update on repeat
    /// registration. Shipped defaults must never set it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bootstrapped_factory: bool,
}

/// Factory server entry in the server registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryServerEntry {
    pub id: String,
    pub base_url: String,
    pub known_public_key: String,
    pub priority: u32,
    pub enabled: bool,
}

/// Serde default for [`ServerEntry::required`]: an entry that omits the
/// field is treated as required, preserving the pre-flag fail-hard
/// behavior for configs written before the flag existed.
fn default_true() -> bool {
    true
}

/// Where a configured server's content lives: exactly one of a package
/// feed (the per-firmware release catalog) or a direct package index.
/// Both are exact document URLs — nothing is appended to them.
#[derive(Debug, Clone)]
pub enum ServerSource {
    Feed { feed_url: String },
    Index { index_url: String },
}

/// A configured package server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawServerEntry", into = "RawServerEntry")]
pub struct ServerEntry {
    pub id: String,
    pub source: ServerSource,
    pub known_public_key: String,
    pub priority: u32,
    pub enabled: bool,
    /// When true, a failed index fetch from this server aborts the whole
    /// merge. When false, the failure degrades to a warning and the merge
    /// proceeds with the remaining servers.
    pub required: bool,
}

/// Wire shape of [`ServerEntry`]: the source enum flattens to optional
/// `feed_url`/`index_url` fields, with exactly-one enforcement and URL
/// validation on deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawServerEntry {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    feed_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index_url: Option<String>,
    known_public_key: String,
    priority: u32,
    enabled: bool,
    #[serde(default = "default_true")]
    required: bool,
}

impl TryFrom<RawServerEntry> for ServerEntry {
    type Error = String;

    fn try_from(raw: RawServerEntry) -> Result<Self, Self::Error> {
        let source = match (raw.feed_url, raw.index_url) {
            (Some(feed_url), None) => {
                validate_content_url("feed_url", &feed_url)?;
                ServerSource::Feed { feed_url }
            }
            (None, Some(index_url)) => {
                validate_content_url("index_url", &index_url)?;
                ServerSource::Index { index_url }
            }
            (Some(_), Some(_)) | (None, None) => {
                return Err(format!(
                    "server '{}' must contain exactly one of feed_url or index_url",
                    raw.id
                ));
            }
        };
        Ok(ServerEntry {
            id: raw.id,
            source,
            known_public_key: raw.known_public_key,
            priority: raw.priority,
            enabled: raw.enabled,
            required: raw.required,
        })
    }
}

impl From<ServerEntry> for RawServerEntry {
    fn from(entry: ServerEntry) -> Self {
        let (feed_url, index_url) = match entry.source {
            ServerSource::Feed { feed_url } => (Some(feed_url), None),
            ServerSource::Index { index_url } => (None, Some(index_url)),
        };
        RawServerEntry {
            id: entry.id,
            feed_url,
            index_url,
            known_public_key: entry.known_public_key,
            priority: entry.priority,
            enabled: entry.enabled,
            required: entry.required,
        }
    }
}

/// Absolute-URL check for server content links, parsed with
/// [`reqwest::Url`]: the scheme must be `http`, `https`, or `file`;
/// http(s) URLs must carry a host; `file` URLs must carry an absolute
/// path and no host. `what` names the offending field in the error.
///
/// # Errors
///
/// Returns a human-readable description of why `url` was rejected.
pub fn validate_content_url(what: &str, url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|source| {
        format!("{what} '{url}' is not an absolute http(s):// or file:// URL: {source}")
    })?;
    match parsed.scheme() {
        "http" | "https" => {
            if parsed.host_str().is_none() {
                return Err(format!("{what} '{url}' has no host"));
            }
        }
        "file" => {
            if parsed.host_str().is_some_and(|host| !host.is_empty())
                || !parsed.path().starts_with('/')
            {
                return Err(format!("{what} '{url}' must be an absolute file:///… path"));
            }
        }
        other => {
            return Err(format!(
                "{what} '{url}' has unsupported scheme '{other}' (expected http, https, or file)"
            ));
        }
    }
    Ok(())
}

/// A single fetched index bundled with its source-server metadata.
///
/// Built by [`crate::index::fetch_and_merge_indexes`] from `ServerEntry`
/// data and consumed by [`crate::index::merge_indexes`].
#[derive(Debug, Clone)]
pub struct FetchedIndex {
    pub server_id: String,
    pub server_priority: u32,
    pub index: PackageIndex,
}

/// Result of merging indexes from all servers.
///
/// Stores all package entries from all servers in a flat vec.
/// `by_name` provides fast lookup by package name to indices into `packages`.
#[derive(Debug, Clone)]
pub struct MergedIndex {
    /// All entries in insertion order.
    pub packages: Vec<MergedPackageEntry>,
    /// Lookup by package name → indices into `packages`.
    pub by_name: BTreeMap<String, Vec<usize>>,
}

/// A package entry within a [`MergedIndex`], tagged with server metadata.
///
/// Cache metadata is intentionally absent — store paths are realised
/// through configured Nix substituters.
#[derive(Debug, Clone)]
pub struct MergedPackageEntry {
    pub name: String,
    pub version: Version,
    pub store_path: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub upgrade_strategy: Option<UpgradeStrategy>,
    pub install_strategy: Option<InstallStrategy>,
    pub server_id: String,
    pub server_priority: u32,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

/// Whether the application's periodic collection path runs.
///
/// An escape hatch for developers debugging on a device, where a collection
/// landing mid-session perturbs what they are looking at. It does not affect
/// collection before an automatic upgrade or `bmc-nix-cli gc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PeriodicGcMode {
    #[default]
    Enabled,
    Disabled,
}

/// GC configuration (`/etc/nix-upgrade/gc.json`).
///
/// `#[serde(default)]` lets a partial file fill any missing field from
/// [`GcConfig::default`], and lets an absent file fall back entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GcConfig {
    pub keep_generations: usize,
    /// Keep generations newer than this many days. `None` disables
    /// age-based retention.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<usize>,
    pub protected_generations: Vec<usize>,
    pub periodic: PeriodicGcMode,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            keep_generations: 2,
            keep_days: None,
            protected_generations: Vec::new(),
            periodic: PeriodicGcMode::Enabled,
        }
    }
}

/// Output of computing an upgrade plan.
#[derive(Debug)]
pub struct UpgradePlan {
    /// Resolved packages to apply (includes unchanged packages).
    pub packages: Vec<ResolvedPackage>,
    /// Packages newly added in the target profile.
    pub added: Vec<PackageVersion>,
    /// Packages removed from the target profile.
    pub removed: Vec<PackageVersion>,
    /// Packages that change version.
    pub changed: Vec<PackageChange>,
    /// Packages present in the current manifest but missing (or with no
    /// satisfying version) in the merged index. Carried over at the
    /// current version so they remain installed.
    pub stale: Vec<PackageVersion>,
}

/// A package that changes between the current and target profile.
///
/// "Change" covers both a version bump and a store-path change at the same
/// version (rebuild or re-derivation), so callers can distinguish a rebuild
/// from a version upgrade.
#[derive(Debug, Clone)]
pub struct PackageChange {
    pub name: String,
    pub from_version: String,
    pub to_version: String,
    pub from_store_path: String,
    pub to_store_path: String,
}

/// A package name+version pair.
#[derive(Debug, Clone)]
pub struct PackageVersion {
    pub name: String,
    pub version: String,
}

/// Summary of strategies present in a given install/upgrade run.
#[derive(Debug)]
pub struct StrategySummary {
    pub upgrade: Vec<UpgradeStrategy>,
    pub install: Vec<InstallStrategy>,
}

impl StrategySummary {
    /// Collect unique strategy hints from a set of resolved packages.
    #[must_use]
    pub fn from_packages(packages: &[ResolvedPackage]) -> Self {
        use std::collections::HashSet;

        let mut upgrade_set = HashSet::new();
        let mut install_set = HashSet::new();

        for pkg in packages {
            if let Some(ref s) = pkg.upgrade_strategy {
                upgrade_set.insert(s.clone());
            }
            if let Some(ref s) = pkg.install_strategy {
                install_set.insert(s.clone());
            }
        }

        Self {
            upgrade: upgrade_set.into_iter().collect(),
            install: install_set.into_iter().collect(),
        }
    }
}

/// Result of an install/upgrade run.
///
/// `generation` is `None` only in the edge case where the request was a
/// no-op (empty added/removed/changed) AND no prior profile existed, so
/// there is nothing to point at.
#[derive(Debug)]
pub struct InstallResult {
    pub generation: Option<ProfileGeneration>,
    pub strategies: StrategySummary,
    pub added: Vec<PackageVersion>,
    pub removed: Vec<PackageVersion>,
    pub changed: Vec<PackageChange>,
    /// Packages carried over because they are no longer represented in the
    /// live merged indexes. Surfaced so callers can warn the operator.
    pub stale: Vec<PackageVersion>,
    /// Outcome of the post-activation GC sweep. The generation is already
    /// built and activated by the time GC runs, so a failure here is
    /// reported for the operator to see rather than failing the upgrade.
    /// `Ok(())` also covers the case where GC was not requested.
    pub gc: Result<(), crate::gc::GcError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_result_carries_stale_packages() {
        let result = InstallResult {
            generation: None,
            strategies: StrategySummary {
                upgrade: vec![],
                install: vec![],
            },
            added: vec![],
            removed: vec![],
            changed: vec![],
            stale: vec![PackageVersion {
                name: "clock".into(),
                version: "1.0.0".into(),
            }],
            gc: Ok(()),
        };

        assert_eq!(result.stale[0].name, "clock");
    }

    #[test]
    fn deserialize_package_index() {
        let json = r#"{
            "version": 1,
            "provenance": { "commit": "abc123" },
            "indexes": [],
            "caches": [{
                "name": "default",
                "cache_url": "https://cache.example.com",
                "cache_key": "cache.example.com:AAAA"
            }],
            "packages": [{
                "name": "test-pkg",
                "version": "1.0.0",
                "store_path": "/nix/store/abc-test-pkg-1.0.0",
                "category": "core",
                "description": "Test package"
            }]
        }"#;
        let index: PackageIndex =
            serde_json::from_str(json).expect("BUG: test JSON should be valid");
        assert_eq!(index.version, 1);
        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].name, "test-pkg");
    }

    #[test]
    fn unknown_strategy_values_deserialize_to_fallback() {
        // A newer server may serve strategy hints this build does not
        // know. They must map to the Unknown fallback instead of failing
        // the whole index parse, or fielded devices could never see the
        // upgrade that understands the new value.
        let json = r#"{
            "name": "test-pkg",
            "version": "2.0.0",
            "store_path": "/nix/store/abc-test-pkg-2.0.0",
            "upgrade_strategy": "hot-swap",
            "install_strategy": "hot-swap"
        }"#;
        let entry: PackageEntry =
            serde_json::from_str(json).expect("BUG: unknown strategies must not fail the parse");
        assert_eq!(entry.upgrade_strategy, Some(UpgradeStrategy::Unknown));
        assert_eq!(entry.install_strategy, Some(InstallStrategy::Unknown));
    }

    #[test]
    fn deserialize_manifest() {
        let manifest = Manifest {
            packages: BTreeMap::from([(
                "test-pkg".into(),
                ManifestPackage {
                    version: "1.0.0".into(),
                    store_path: "/nix/store/abc-test-pkg-1.0.0".into(),
                    category: Some("core".into()),
                    description: Some("Test".into()),
                    upgrade_strategy: None,
                    install_strategy: None,
                    installed_by: InstalledBy::System,
                    installed_from: "local".into(),
                    pinned: None,
                },
            )]),
        };
        let json = serde_json::to_string_pretty(&manifest).expect("BUG: serialize should succeed");
        let parsed: Manifest = serde_json::from_str(&json).expect("BUG: round-trip should succeed");
        assert_eq!(parsed.packages.len(), 1);
        assert!(parsed.packages.contains_key("test-pkg"));
    }

    #[test]
    fn package_entry_metadata_defaults_empty_and_deserializes() {
        let without: PackageEntry = serde_json::from_str(
            r#"{"name":"core","version":"1.0.0","store_path":"/nix/store/x"}"#,
        )
        .expect("BUG: entry without metadata parses");
        assert!(without.metadata.is_empty());

        let with: PackageEntry = serde_json::from_str(
            r#"{"name":"core","version":"1.0.0","store_path":"/nix/store/x",
                "metadata":{"bmc_version":"2.4.0","changelog":"fixes"}}"#,
        )
        .expect("BUG: entry with metadata parses");
        assert_eq!(
            with.metadata
                .get("bmc_version")
                .and_then(serde_json::Value::as_str),
            Some("2.4.0")
        );
    }

    #[test]
    fn package_entry_optional_fields_default() {
        let json = r#"{
            "name": "minimal",
            "version": "0.1.0",
            "store_path": "/nix/store/xyz"
        }"#;
        let entry: PackageEntry =
            serde_json::from_str(json).expect("BUG: minimal JSON should parse");
        assert_eq!(entry.name, "minimal");
        assert!(entry.cache.is_none());
        assert!(entry.category.is_none());
        assert!(entry.upgrade_strategy.is_none());
    }

    /// Verify that `"pinned": null` in a manifest round-trips correctly.
    #[test]
    fn pinned_null_deserializes_as_default() {
        let json = r#"{
            "packages": {
                "test-pkg": {
                    "version": "1.0.0",
                    "cache": "local",
                    "store_path": "/nix/store/abc-test-pkg-1.0.0",
                    "installed_by": "system",
                    "installed_from": "local",
                    "pinned": null
                }
            }
        }"#;
        let result: Result<Manifest, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "\"pinned\": null should deserialize: {result:?}"
        );
        let manifest = result.expect("BUG: just checked is_ok");
        let pkg = manifest
            .packages
            .get("test-pkg")
            .expect("BUG: test-pkg should exist");
        assert_eq!(pkg.pinned, None);
    }

    /// A pre-branch manifest persisting the legacy `"pinned": "none"`
    /// sentinel must deserialize as unpinned, not as the constraint
    /// `"none"` (which fails to parse and would abort `upgrade`).
    #[test]
    fn pinned_legacy_none_sentinel_deserializes_as_unpinned() {
        let json = r#"{
            "packages": {
                "test-pkg": {
                    "version": "1.0.0",
                    "store_path": "/nix/store/abc-test-pkg-1.0.0",
                    "installed_by": "system",
                    "installed_from": "local",
                    "pinned": "none"
                }
            }
        }"#;
        let manifest: Manifest =
            serde_json::from_str(json).expect("BUG: legacy sentinel manifest should deserialize");
        let pkg = manifest
            .packages
            .get("test-pkg")
            .expect("BUG: test-pkg should exist");
        assert_eq!(pkg.pinned, None);
    }

    /// A real constraint must survive deserialization unchanged.
    #[test]
    fn pinned_real_constraint_deserializes_verbatim() {
        let json = r#"{
            "packages": {
                "test-pkg": {
                    "version": "1.0.0",
                    "store_path": "/nix/store/abc-test-pkg-1.0.0",
                    "installed_by": "system",
                    "installed_from": "local",
                    "pinned": "^1.0.0"
                }
            }
        }"#;
        let manifest: Manifest =
            serde_json::from_str(json).expect("BUG: constraint manifest should deserialize");
        let pkg = manifest
            .packages
            .get("test-pkg")
            .expect("BUG: test-pkg should exist");
        assert_eq!(pkg.pinned.as_deref(), Some("^1.0.0"));
    }

    /// Deserialize a real index.json as produced by `nix build .#init-index-armv7`.
    /// This catches mismatches between the Nix-generated JSON and Rust types.
    #[test]
    fn deserialize_generated_index() {
        let json = r#"{
            "caches": [{
                "cache_key": "cache.braiins.com:placeholder",
                "cache_url": "https://cache.braiins.com",
                "name": "default"
            }],
            "indexes": [],
            "packages": [
                {
                    "cache": "default",
                    "category": "core",
                    "description": "Core system package (bmc-openwrt + activation/hooks)",
                    "install_strategy": null,
                    "name": "core",
                    "store_path": "/nix/store/abc-bmc-core",
                    "upgrade_strategy": "reboot",
                    "version": "0.1.0"
                },
                {
                    "cache": "default",
                    "category": "widget",
                    "description": "Digital clock widget",
                    "install_strategy": null,
                    "name": "digital-clock",
                    "store_path": "/nix/store/xyz-bmc-widget-digital-clock",
                    "upgrade_strategy": null,
                    "version": "1.0.0"
                }
            ],
            "provenance": { "commit": "a569acdb" },
            "version": 1
        }"#;

        let index: PackageIndex =
            serde_json::from_str(json).expect("BUG: generated index should deserialize");

        assert_eq!(index.version, 1);
        assert_eq!(index.caches.len(), 1);
        assert_eq!(index.packages.len(), 2);

        let core = &index.packages[0];
        assert_eq!(core.name, "core");
        assert!(
            matches!(core.upgrade_strategy, Some(UpgradeStrategy::Reboot)),
            "core upgrade_strategy should be Some(Reboot)"
        );
        assert!(
            core.install_strategy.is_none(),
            "core install_strategy should be None"
        );

        let widget = &index.packages[1];
        assert_eq!(widget.name, "digital-clock");
        assert!(
            widget.upgrade_strategy.is_none(),
            "widget upgrade_strategy should be None"
        );
        assert!(
            widget.install_strategy.is_none(),
            "widget install_strategy should be None"
        );
    }

    #[test]
    fn server_entry_omitting_required_defaults_to_true() {
        let json = r#"{
            "id": "dev",
            "index_url": "https://dev.example.com/v1/nix-package-index.v1.json",
            "known_public_key": "k",
            "priority": 50,
            "enabled": true
        }"#;
        let entry: ServerEntry =
            serde_json::from_str(json).expect("BUG: entry without `required` should deserialize");
        assert!(
            entry.required,
            "an omitted `required` must default to true so existing configs stay fail-hard"
        );
    }

    #[test]
    fn server_entry_honours_explicit_required_false() {
        let json = r#"{
            "id": "dev",
            "index_url": "https://dev.example.com/v1/nix-package-index.v1.json",
            "known_public_key": "k",
            "priority": 50,
            "enabled": true,
            "required": false
        }"#;
        let entry: ServerEntry =
            serde_json::from_str(json).expect("BUG: entry with `required` should deserialize");
        assert!(
            !entry.required,
            "an explicit `required: false` must be honoured"
        );
    }

    #[test]
    fn base_selector_parses_current_latest_and_integer() {
        use std::str::FromStr as _;
        assert!(matches!(
            BaseSelector::from_str("current").expect("BUG: parse current"),
            BaseSelector::Current
        ));
        assert!(matches!(
            BaseSelector::from_str("latest").expect("BUG: parse latest"),
            BaseSelector::Latest
        ));
        assert!(matches!(
            BaseSelector::from_str("3").expect("BUG: parse 3"),
            BaseSelector::Generation(3)
        ));
    }

    #[test]
    fn base_selector_rejects_invalid_input() {
        use std::str::FromStr as _;
        assert!(BaseSelector::from_str("").is_err());
        assert!(BaseSelector::from_str("0").is_err());
        assert!(BaseSelector::from_str("-3").is_err());
        assert!(BaseSelector::from_str("abc").is_err());
        assert!(BaseSelector::from_str("latests").is_err());
    }

    #[test]
    fn strategy_summary_collects_unique() {
        let packages = vec![
            ResolvedPackage {
                name: "a".into(),
                version: "1.0.0".into(),
                store_path: "/nix/store/a".into(),
                category: None,
                description: None,
                upgrade_strategy: Some(UpgradeStrategy::Reboot),
                install_strategy: None,
                installed_by: InstalledBy::System,
                installed_from: "local".into(),
                pinned: None,
                metadata: BTreeMap::new(),
            },
            ResolvedPackage {
                name: "b".into(),
                version: "1.0.0".into(),
                store_path: "/nix/store/b".into(),
                category: None,
                description: None,
                upgrade_strategy: Some(UpgradeStrategy::Reboot),
                install_strategy: Some(InstallStrategy::Reboot),
                installed_by: InstalledBy::System,
                installed_from: "local".into(),
                pinned: None,
                metadata: BTreeMap::new(),
            },
        ];
        let summary = StrategySummary::from_packages(&packages);
        assert_eq!(summary.upgrade.len(), 1);
        assert_eq!(summary.install.len(), 1);
    }
}

#[cfg(test)]
mod servers_config_serde_tests {
    use super::*;

    const LEGACY: &str = r#"{"factory":{"id":"forge","base_url":"https://cache.braiins.com/v1","known_public_key":"k","priority":0,"enabled":true},"servers":[]}"#;

    #[test]
    fn config_without_marker_deserializes_as_not_bootstrapped() {
        let config: ServersConfig =
            serde_json::from_str(LEGACY).expect("BUG: legacy config must parse");
        assert!(!config.bootstrapped_factory);
    }

    #[test]
    fn marker_is_omitted_when_false_and_kept_when_true() {
        let mut config: ServersConfig =
            serde_json::from_str(LEGACY).expect("BUG: legacy config must parse");
        let json = serde_json::to_string(&config).expect("BUG: serialize");
        assert!(!json.contains("bootstrapped_factory"));

        config.bootstrapped_factory = true;
        let json = serde_json::to_string(&config).expect("BUG: serialize");
        assert!(json.contains(r#""bootstrapped_factory":true"#));
        let round: ServersConfig = serde_json::from_str(&json).expect("BUG: round-trip");
        assert!(round.bootstrapped_factory);
    }
}

#[cfg(test)]
mod server_entry_serde_tests {
    use super::*;

    #[test]
    fn server_entry_feed_source_round_trips() {
        let json = r#"{"id":"s","feed_url":"https://h/nix-package-feed.v1.json","known_public_key":"k","priority":1,"enabled":true}"#;
        let entry: ServerEntry = serde_json::from_str(json).expect("BUG: valid feed entry");
        assert!(
            matches!(&entry.source, ServerSource::Feed { feed_url } if feed_url == "https://h/nix-package-feed.v1.json")
        );
        assert!(entry.required, "required defaults to true");
        let back = serde_json::to_string(&entry).expect("BUG: serializable");
        assert!(back.contains("feed_url") && !back.contains("index_url"));
    }

    #[test]
    fn server_entry_index_source_round_trips() {
        let json = r#"{"id":"s","index_url":"file:///tmp/i.json","known_public_key":"k","priority":1,"enabled":true,"required":false}"#;
        let entry: ServerEntry = serde_json::from_str(json).expect("BUG: valid index entry");
        assert!(matches!(&entry.source, ServerSource::Index { .. }));
        assert!(!entry.required);
    }

    #[test]
    fn server_entry_rejects_both_and_neither_source() {
        let both = r#"{"id":"s","feed_url":"https://a/f.json","index_url":"https://a/i.json","known_public_key":"k","priority":1,"enabled":true}"#;
        let err = serde_json::from_str::<ServerEntry>(both)
            .expect_err("both sources")
            .to_string();
        assert!(
            err.contains("exactly one of feed_url or index_url"),
            "{err}"
        );
        let neither = r#"{"id":"s","known_public_key":"k","priority":1,"enabled":true}"#;
        assert!(serde_json::from_str::<ServerEntry>(neither).is_err());
    }

    #[test]
    fn server_entry_rejects_old_base_url_shape() {
        let old = r#"{"id":"s","type":"http","base_url":"https://h","known_public_key":"k","priority":1,"enabled":true}"#;
        assert!(serde_json::from_str::<ServerEntry>(old).is_err());
    }

    #[test]
    fn server_entry_rejects_relative_url() {
        let rel = r#"{"id":"s","index_url":"h/i.json","known_public_key":"k","priority":1,"enabled":true}"#;
        let err = serde_json::from_str::<ServerEntry>(rel)
            .expect_err("relative URL")
            .to_string();
        assert!(err.contains("http://") || err.contains("absolute"), "{err}");
    }

    #[test]
    fn server_entry_rejects_malformed_absolute_urls() {
        let bare = r#"{"id":"s","feed_url":"https://","known_public_key":"k","priority":1,"enabled":true}"#;
        assert!(
            serde_json::from_str::<ServerEntry>(bare).is_err(),
            "a scheme with no host must be rejected"
        );
        let relative_file = r#"{"id":"s","index_url":"file://relative/path","known_public_key":"k","priority":1,"enabled":true}"#;
        assert!(
            serde_json::from_str::<ServerEntry>(relative_file).is_err(),
            "a file URL without an absolute path must be rejected"
        );
    }

    #[test]
    fn validate_content_url_accepts_absolute_forms() {
        validate_content_url("feed_url", "https://h/f.json").expect("BUG: https accepted");
        validate_content_url("feed_url", "http://h:8080/f.json").expect("BUG: http accepted");
        validate_content_url("index_url", "file:///tmp/i.json").expect("BUG: file accepted");
    }
}

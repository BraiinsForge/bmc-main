// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::{BTreeMap, HashSet};

use semver::Version;
use tracing::{debug, warn};

use crate::types::{
    CacheEntry, InstalledBy, ManifestPackage, MergedIndex, MergedPackageEntry, PackageEntry,
    PackageIndex, PinStrategy, ResolvedPackage, ServerEntry,
};

/// Errors that can occur when resolving packages from an index.
#[derive(Debug, thiserror::Error)]
pub enum ResolveIndexError {
    #[error("cache '{cache}' not found for package '{package}'")]
    CacheNotFound { package: String, cache: String },
    #[error("no caches defined for package '{package}'")]
    NoCaches { package: String },
}

/// Resolve the cache URL for a package entry.
///
/// If the entry specifies a named cache, look it up in the caches list.
/// If no cache is specified, use the first cache entry as the default.
/// Returns an error if the referenced cache is not found or no caches
/// are defined.
fn resolve_cache_url(
    entry: &PackageEntry,
    caches: &[CacheEntry],
) -> Result<String, ResolveIndexError> {
    match &entry.cache {
        Some(cache_name) => caches
            .iter()
            .find(|c| c.name == *cache_name)
            .map(|c| c.cache_url.clone())
            .ok_or_else(|| ResolveIndexError::CacheNotFound {
                package: entry.name.clone(),
                cache: cache_name.clone(),
            }),
        None => {
            caches
                .first()
                .map(|c| c.cache_url.clone())
                .ok_or_else(|| ResolveIndexError::NoCaches {
                    package: entry.name.clone(),
                })
        }
    }
}

/// Resolve all packages in an index to [`ResolvedPackage`] values.
///
/// Each package entry is paired with its cache URL and given Stage 1
/// defaults: `installed_by = System`, `installed_from = "local"`,
/// `pinned = false`.
///
/// This is used by `bmc-nix-cli build-profile` when packages are
/// already present in the local Nix store.
pub fn resolve_all_from_index(
    index: &PackageIndex,
) -> Result<Vec<ResolvedPackage>, ResolveIndexError> {
    index
        .packages
        .iter()
        .map(|entry| {
            let cache_url = resolve_cache_url(entry, &index.caches)?;
            Ok(ResolvedPackage {
                name: entry.name.clone(),
                version: entry.version.clone(),
                store_path: entry.store_path.clone(),
                cache_url: Some(cache_url),
                cache_name: "local".into(),
                category: entry.category.clone(),
                description: entry.description.clone(),
                upgrade_strategy: entry.upgrade_strategy.clone(),
                install_strategy: entry.install_strategy.clone(),
                installed_by: InstalledBy::System,
                installed_from: "local".into(),
                pinned: PinStrategy::None,
            })
        })
        .collect()
}

/// Errors that can occur when fetching indexes from servers.
#[derive(Debug, thiserror::Error)]
pub enum FetchIndexesError {
    #[error("failed to fetch index from {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("invalid index JSON from {url}: {source}")]
    InvalidJson {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("unsupported index version {version} from {url}")]
    UnsupportedVersion { url: String, version: u32 },
}

/// Errors that can occur when resolving a single package.
#[derive(Debug, thiserror::Error)]
pub enum ResolvePackageError {
    #[error("package '{0}' not found in any index")]
    PackageNotFound(String),
    #[error("no version matching '{constraint}' for package '{package}'")]
    VersionNotFound { package: String, constraint: String },
    #[error(
        "ambiguous resolution for package '{package}': multiple servers at same version and priority"
    )]
    Ambiguous { package: String },
    #[error("cache '{cache}' referenced by package '{package}' not found in server '{server}'")]
    CacheNotFound {
        package: String,
        cache: String,
        server: String,
    },
}

/// Fetch a single index from a URL.
pub async fn fetch_index(
    client: &reqwest::Client,
    url: &str,
) -> Result<PackageIndex, FetchIndexesError> {
    let index: PackageIndex = client
        .get(url)
        .send()
        .await
        .map_err(|source| FetchIndexesError::Fetch {
            url: url.to_owned(),
            source,
        })?
        .json()
        .await
        .map_err(|source| FetchIndexesError::InvalidJson {
            url: url.to_owned(),
            source,
        })?;

    if index.version != 1 {
        return Err(FetchIndexesError::UnsupportedVersion {
            url: url.to_owned(),
            version: index.version,
        });
    }

    Ok(index)
}

/// Fetch indexes from all enabled servers.
pub async fn fetch_indexes(
    client: &reqwest::Client,
    servers: &[ServerEntry],
) -> Result<Vec<(String, PackageIndex)>, FetchIndexesError> {
    let mut results = Vec::new();
    for server in servers {
        if !server.enabled {
            debug!(server_id = %server.id, "skipping disabled server");
            continue;
        }
        let index = fetch_index(client, &server.index_url).await?;
        results.push((server.id.clone(), index));
    }
    Ok(results)
}

/// Fetch and merge indexes from all servers, following federated
/// `indexes` URLs with visited-set cycle detection.
pub async fn fetch_and_merge_indexes(
    client: &reqwest::Client,
    servers: &[ServerEntry],
) -> Result<MergedIndex, FetchIndexesError> {
    let mut all_fetched: Vec<(String, PackageIndex)> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    // Fetch primary indexes from servers
    for server in servers {
        if !server.enabled {
            continue;
        }
        visited.insert(server.index_url.clone());
        let index = fetch_index(client, &server.index_url).await?;

        // Queue federated indexes for follow-up
        let mut federated_urls: Vec<String> = index.indexes.clone();
        all_fetched.push((server.id.clone(), index));

        // Follow federated indexes
        while let Some(url) = federated_urls.pop() {
            if !visited.insert(url.clone()) {
                debug!(%url, "skipping already-visited federated index");
                continue;
            }
            let federated_index = fetch_index(client, &url).await?;
            federated_urls.extend(federated_index.indexes.clone());
            all_fetched.push((server.id.clone(), federated_index));
        }
    }

    let priorities: BTreeMap<String, u32> = servers
        .iter()
        .filter(|s| s.enabled)
        .map(|s| (s.id.clone(), s.priority))
        .collect();

    Ok(merge_indexes(all_fetched, &priorities))
}

/// Merge pre-fetched indexes into a single [`MergedIndex`].
///
/// Tags each package entry with its `server_id` and resolves cache URLs.
/// Builds the `by_name` lookup map for fast package resolution.
pub fn merge_indexes(
    fetched: Vec<(String, PackageIndex)>,
    server_priorities: &BTreeMap<String, u32>,
) -> MergedIndex {
    let mut caches = Vec::new();
    let mut packages = Vec::new();
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for (server_id, index) in fetched {
        let priority = server_priorities
            .get(&server_id)
            .copied()
            .unwrap_or(u32::MAX);

        // Merge caches (deduplicate by name)
        let existing_cache_names: HashSet<String> =
            caches.iter().map(|c: &CacheEntry| c.name.clone()).collect();
        for cache in &index.caches {
            if !existing_cache_names.contains(&cache.name) {
                caches.push(cache.clone());
            }
        }

        for entry in &index.packages {
            // Resolve cache URL for this entry
            let (cache_url, cache_name) = match resolve_cache_url(entry, &index.caches) {
                Ok(url) => {
                    let name = entry.cache.clone().unwrap_or_else(|| {
                        index
                            .caches
                            .first()
                            .map(|c| c.name.clone())
                            .unwrap_or_default()
                    });
                    (url, name)
                }
                Err(e) => {
                    warn!(
                        package = %entry.name,
                        server = %server_id,
                        "skipping package with unresolvable cache: {e}"
                    );
                    continue;
                }
            };

            let version = match Version::parse(&entry.version) {
                Ok(v) => v,
                Err(e) => {
                    warn!(
                        package = %entry.name,
                        version = %entry.version,
                        "skipping package with invalid semver: {e}"
                    );
                    continue;
                }
            };

            let idx = packages.len();
            packages.push(MergedPackageEntry {
                name: entry.name.clone(),
                version,
                store_path: entry.store_path.clone(),
                cache_url,
                cache_name,
                category: entry.category.clone(),
                description: entry.description.clone(),
                upgrade_strategy: entry.upgrade_strategy.clone(),
                install_strategy: entry.install_strategy.clone(),
                server_id: server_id.clone(),
                server_priority: priority as usize,
            });

            by_name.entry(entry.name.clone()).or_default().push(idx);
        }
    }

    MergedIndex {
        caches,
        packages,
        by_name,
    }
}

/// Resolve a new package by name and optional version constraint.
///
/// If `version` is `None`, picks the latest version available.
/// On version ties, resolves by server priority (lower number wins).
/// Fails explicitly if ambiguous after all tie-breaking.
pub fn resolve_new_package(
    merged: &MergedIndex,
    name: &str,
    version: Option<&str>,
    installed_by: InstalledBy,
) -> Result<ResolvedPackage, ResolvePackageError> {
    let indices = merged
        .by_name
        .get(name)
        .ok_or_else(|| ResolvePackageError::PackageNotFound(name.to_owned()))?;

    let mut candidates: Vec<&MergedPackageEntry> =
        indices.iter().map(|&i| &merged.packages[i]).collect();

    // Apply version constraint if specified
    if let Some(constraint) = version {
        candidates.retain(|e| version_matches(&e.version, constraint));
        if candidates.is_empty() {
            return Err(ResolvePackageError::VersionNotFound {
                package: name.to_owned(),
                constraint: constraint.to_owned(),
            });
        }
    }

    pick_best_candidate(name, &candidates, installed_by)
}

/// Resolve an already-installed package to its upgraded version.
///
/// Uses the manifest entry to determine source server and pin strategy.
/// First looks on the same server (`installed_from`), then falls back
/// to all servers.
pub fn resolve_installed_package(
    merged: &MergedIndex,
    name: &str,
    current: &ManifestPackage,
) -> Result<ResolvedPackage, ResolvePackageError> {
    let indices = merged
        .by_name
        .get(name)
        .ok_or_else(|| ResolvePackageError::PackageNotFound(name.to_owned()))?;

    let all_entries: Vec<&MergedPackageEntry> =
        indices.iter().map(|&i| &merged.packages[i]).collect();

    let current_version =
        Version::parse(&current.version).map_err(|_| ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: format!("current version '{}' is invalid semver", current.version),
        })?;

    // Step 1: Filter by pin strategy
    let pin_filtered: Vec<&MergedPackageEntry> = all_entries
        .iter()
        .filter(|e| version_matches_pin(&e.version, &current_version, &current.pinned))
        .copied()
        .collect();

    if pin_filtered.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: format!("pin strategy {:?} from {}", current.pinned, current.version),
        });
    }

    // Step 2: Try same server first (server affinity)
    let same_server: Vec<&MergedPackageEntry> = pin_filtered
        .iter()
        .filter(|e| e.server_id == current.installed_from)
        .copied()
        .collect();

    if !same_server.is_empty() {
        return pick_best_candidate(name, &same_server, current.installed_by.clone());
    }

    // Step 3: Fall back to all servers
    pick_best_candidate(name, &pin_filtered, current.installed_by.clone())
}

/// From a set of candidates, pick the best one: latest version, then
/// lowest priority on tie, then fail if still ambiguous.
fn pick_best_candidate(
    name: &str,
    candidates: &[&MergedPackageEntry],
    installed_by: InstalledBy,
) -> Result<ResolvedPackage, ResolvePackageError> {
    if candidates.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: "no candidates available".to_owned(),
        });
    }

    // Sort: latest version first, then lowest priority
    let mut sorted: Vec<&MergedPackageEntry> = candidates.to_vec();
    sorted.sort_by(|a, b| {
        b.version
            .cmp(&a.version)
            .then(a.server_priority.cmp(&b.server_priority))
    });

    let best = sorted[0];

    // Check for ambiguity: if the second candidate has same version and priority
    if sorted.len() > 1 {
        let second = sorted[1];
        if best.version == second.version && best.server_priority == second.server_priority {
            return Err(ResolvePackageError::Ambiguous {
                package: name.to_owned(),
            });
        }
    }

    Ok(merged_entry_to_resolved(best, installed_by))
}

/// Convert a [`MergedPackageEntry`] to a [`ResolvedPackage`].
fn merged_entry_to_resolved(
    entry: &MergedPackageEntry,
    installed_by: InstalledBy,
) -> ResolvedPackage {
    ResolvedPackage {
        name: entry.name.clone(),
        version: entry.version.to_string(),
        store_path: entry.store_path.clone(),
        cache_url: Some(entry.cache_url.clone()),
        cache_name: entry.cache_name.clone(),
        category: entry.category.clone(),
        description: entry.description.clone(),
        upgrade_strategy: entry.upgrade_strategy.clone(),
        install_strategy: entry.install_strategy.clone(),
        installed_by,
        installed_from: entry.server_id.clone(),
        pinned: PinStrategy::None,
    }
}

/// Check if a version matches a prefix constraint (e.g., "1.2" matches "1.2.3").
fn version_matches(version: &Version, constraint: &str) -> bool {
    let version_str = version.to_string();
    version_str == constraint || version_str.starts_with(&format!("{constraint}."))
}

/// Check if a candidate version is allowed by a pin strategy relative to
/// the currently installed version.
fn version_matches_pin(candidate: &Version, current: &Version, pin: &PinStrategy) -> bool {
    match pin {
        PinStrategy::None => true,
        PinStrategy::Major => candidate.major == current.major,
        PinStrategy::Minor => candidate.major == current.major && candidate.minor == current.minor,
        PinStrategy::Patch => candidate == current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CacheEntry, PackageEntry, PackageIndex, Provenance};

    /// Helper: build a minimal `PackageIndex` from given caches and
    /// packages.
    fn make_index(caches: Vec<CacheEntry>, packages: Vec<PackageEntry>) -> PackageIndex {
        PackageIndex {
            version: 1,
            provenance: Some(Provenance {
                commit: "test".into(),
            }),
            indexes: vec![],
            caches,
            packages,
        }
    }

    fn default_cache() -> CacheEntry {
        CacheEntry {
            name: "default".into(),
            cache_url: "https://cache.example.com".into(),
            cache_key: "cache.example.com:AAAA".into(),
        }
    }

    fn make_package(name: &str, cache: Option<&str>) -> PackageEntry {
        PackageEntry {
            name: name.into(),
            version: "1.0.0".into(),
            cache: cache.map(Into::into),
            store_path: format!("/nix/store/abc-{name}-1.0.0"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            server_id: String::new(),
        }
    }

    #[test]
    fn resolve_all_from_index_basic() {
        let index = make_index(vec![default_cache()], vec![make_package("hello", None)]);

        let resolved = resolve_all_from_index(&index)
            .expect("BUG: resolution should succeed with a default cache");

        assert_eq!(resolved.len(), 1);
        let pkg = &resolved[0];
        assert_eq!(pkg.name, "hello");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.store_path, "/nix/store/abc-hello-1.0.0");
        assert_eq!(pkg.cache_url.as_deref(), Some("https://cache.example.com"));
        assert_eq!(pkg.installed_from, "local");
        assert_eq!(pkg.pinned, PinStrategy::None);
        assert!(matches!(pkg.installed_by, InstalledBy::System));
    }

    #[test]
    fn resolve_all_with_named_cache() {
        let extra_cache = CacheEntry {
            name: "extra".into(),
            cache_url: "https://extra-cache.example.com".into(),
            cache_key: "extra.example.com:BBBB".into(),
        };
        let index = make_index(
            vec![default_cache(), extra_cache],
            vec![make_package("world", Some("extra"))],
        );

        let resolved = resolve_all_from_index(&index)
            .expect("BUG: resolution should succeed with named cache");

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].cache_url.as_deref(),
            Some("https://extra-cache.example.com")
        );
    }

    #[test]
    fn resolve_all_missing_cache_returns_error() {
        let index = make_index(
            vec![default_cache()],
            vec![make_package("broken", Some("nonexistent"))],
        );

        let err = resolve_all_from_index(&index)
            .expect_err("should fail when named cache does not exist");

        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent"),
            "error should mention the missing cache name: {msg}"
        );
        assert!(
            msg.contains("broken"),
            "error should mention the package name: {msg}"
        );
    }

    #[test]
    fn resolve_all_empty_caches_returns_error() {
        let index = make_index(vec![], vec![make_package("orphan", None)]);

        let err =
            resolve_all_from_index(&index).expect_err("should fail when no caches are defined");

        let msg = err.to_string();
        assert!(
            msg.contains("no caches defined"),
            "error should describe missing caches: {msg}"
        );
        assert!(
            msg.contains("orphan"),
            "error should mention the package name: {msg}"
        );
    }

    #[test]
    fn resolve_all_multiple_packages() {
        let index = make_index(
            vec![default_cache()],
            vec![
                make_package("alpha", None),
                make_package("beta", None),
                make_package("gamma", None),
            ],
        );

        let resolved = resolve_all_from_index(&index)
            .expect("BUG: resolution should succeed for multiple packages");

        assert_eq!(resolved.len(), 3);
        let names: Vec<&str> = resolved.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);

        // All should share the same cache URL
        for pkg in &resolved {
            assert_eq!(pkg.cache_url.as_deref(), Some("https://cache.example.com"));
            assert!(matches!(pkg.installed_by, InstalledBy::System));
            assert_eq!(pkg.installed_from, "local");
        }
    }

    // ---- Test helpers for merge/resolve ----

    fn make_versioned_package(name: &str, version: &str, cache: Option<&str>) -> PackageEntry {
        PackageEntry {
            name: name.into(),
            version: version.into(),
            cache: cache.map(Into::into),
            store_path: format!("/nix/store/hash-{name}-{version}"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            server_id: String::new(),
        }
    }

    fn test_package_index(entries: Vec<(&str, &str, &str)>) -> PackageIndex {
        let mut cache_names: HashSet<String> = HashSet::new();
        let mut caches = Vec::new();
        let mut packages = Vec::new();

        for (name, version, cache_name) in entries {
            if cache_names.insert(cache_name.to_owned()) {
                caches.push(CacheEntry {
                    name: cache_name.into(),
                    cache_url: format!("https://{cache_name}.example.com"),
                    cache_key: format!("{cache_name}:KEY"),
                });
            }
            packages.push(make_versioned_package(name, version, Some(cache_name)));
        }

        PackageIndex {
            version: 1,
            provenance: None,
            indexes: vec![],
            caches,
            packages,
        }
    }

    /// Build a MergedIndex using merge_indexes with default priority 1 for
    /// all servers.
    fn build_test_merged_index(entries: &[(&str, &str, &str)]) -> MergedIndex {
        let with_priorities: Vec<_> = entries
            .iter()
            .map(|(n, v, s)| (*n, *v, *s, 1_u32))
            .collect();
        build_test_merged_index_with_priorities(&with_priorities)
    }

    /// Build a MergedIndex with explicit server priorities.
    fn build_test_merged_index_with_priorities(entries: &[(&str, &str, &str, u32)]) -> MergedIndex {
        // Group entries by server_id, preserving insertion order via BTreeMap
        let mut by_server: BTreeMap<String, Vec<(&str, &str)>> = BTreeMap::new();
        let mut priorities: BTreeMap<String, u32> = BTreeMap::new();

        for &(name, version, server_id, priority) in entries {
            by_server
                .entry(server_id.to_owned())
                .or_default()
                .push((name, version));
            priorities.insert(server_id.to_owned(), priority);
        }

        let mut all_fetched = Vec::new();
        for (server_id, pkgs) in &by_server {
            let cache_name = format!("cache-{server_id}");
            let packages: Vec<PackageEntry> = pkgs
                .iter()
                .map(|(name, version)| make_versioned_package(name, version, Some(&cache_name)))
                .collect();
            let index = PackageIndex {
                version: 1,
                provenance: None,
                indexes: vec![],
                caches: vec![CacheEntry {
                    name: cache_name.clone(),
                    cache_url: format!("https://{cache_name}.example.com"),
                    cache_key: format!("{cache_name}:KEY"),
                }],
                packages,
            };
            all_fetched.push((server_id.clone(), index));
        }

        merge_indexes(all_fetched, &priorities)
    }

    fn test_manifest_package(
        version: &str,
        installed_from: &str,
        pinned: PinStrategy,
    ) -> ManifestPackage {
        ManifestPackage {
            version: version.into(),
            cache: "default".into(),
            store_path: format!("/nix/store/hash-pkg-{version}"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: installed_from.into(),
            pinned,
        }
    }

    // ---- merge_indexes tests ----

    #[test]
    fn merge_indexes_deduplicates_by_name() {
        let index_a = test_package_index(vec![("widget", "1.0.0", "cache-a")]);
        let index_b = test_package_index(vec![("widget", "2.0.0", "cache-b")]);

        let priorities = BTreeMap::from([("server_a".into(), 1), ("server_b".into(), 2)]);
        let merged = merge_indexes(
            vec![("server_a".into(), index_a), ("server_b".into(), index_b)],
            &priorities,
        );
        assert_eq!(merged.by_name["widget"].len(), 2);
    }

    #[test]
    fn merge_indexes_tags_entries_with_server_id() {
        let index = test_package_index(vec![("widget", "1.0.0", "cache-x")]);
        let priorities = BTreeMap::from([("my_server".into(), 1)]);
        let merged = merge_indexes(vec![("my_server".into(), index)], &priorities);
        let idx = merged.by_name["widget"][0];
        assert_eq!(merged.packages[idx].server_id, "my_server");
    }

    #[test]
    fn merge_indexes_preserves_all_versions_per_package() {
        let index_a = test_package_index(vec![("widget", "1.0.0", "cache-a")]);
        let index_b = test_package_index(vec![
            ("widget", "1.0.0", "cache-b"),
            ("widget", "2.0.0", "cache-b"),
        ]);
        let priorities = BTreeMap::from([("server_a".into(), 1), ("server_b".into(), 2)]);
        let merged = merge_indexes(
            vec![("server_a".into(), index_a), ("server_b".into(), index_b)],
            &priorities,
        );
        assert_eq!(merged.by_name["widget"].len(), 3);
    }

    // ---- resolve_new_package tests ----

    #[test]
    fn resolve_new_package_picks_latest_version() {
        let merged = build_test_merged_index(&[
            ("widget", "1.0.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let resolved = resolve_new_package(&merged, "widget", None, InstalledBy::User)
            .expect("BUG: resolve failed");
        assert_eq!(resolved.version, "2.0.0");
    }

    #[test]
    fn resolve_new_with_version_constraint() {
        let merged = build_test_merged_index(&[
            ("widget", "1.2.3", "server_a"),
            ("widget", "1.3.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let resolved = resolve_new_package(&merged, "widget", Some("1.2"), InstalledBy::User)
            .expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.2.3");
    }

    #[test]
    fn resolve_new_prefers_higher_priority_server_on_version_tie() {
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "2.0.0", "server_a", 3),
            ("widget", "2.0.0", "server_b", 1),
        ]);
        let resolved = resolve_new_package(&merged, "widget", None, InstalledBy::User)
            .expect("BUG: resolve failed");
        assert_eq!(resolved.installed_from, "server_b");
    }

    #[test]
    fn resolve_new_picks_latest_across_servers() {
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "1.0.0", "server_a", 1),
            ("widget", "2.0.0", "server_b", 3),
        ]);
        let resolved = resolve_new_package(&merged, "widget", None, InstalledBy::User)
            .expect("BUG: resolve failed");
        assert_eq!(resolved.version, "2.0.0");
        assert_eq!(resolved.installed_from, "server_b");
    }

    #[test]
    fn resolve_new_fails_on_ambiguous_after_priority() {
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "1.0.0", "server_a", 1),
            ("widget", "1.0.0", "server_b", 1),
        ]);
        let result = resolve_new_package(&merged, "widget", None, InstalledBy::User);
        assert!(result.is_err());
    }

    #[test]
    fn package_not_found_returns_error() {
        let merged = build_test_merged_index(&[("widget", "1.0.0", "server_a")]);
        let result = resolve_new_package(&merged, "nonexistent", None, InstalledBy::User);
        assert!(result.is_err());
    }

    // ---- resolve_installed_package tests ----

    #[test]
    fn resolve_installed_respects_pin_major() {
        let merged = build_test_merged_index(&[
            ("widget", "1.0.0", "server_a"),
            ("widget", "1.1.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let current = test_manifest_package("1.0.0", "server_a", PinStrategy::Major);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.1.0");
    }

    #[test]
    fn resolve_installed_pin_minor_keeps_minor() {
        let merged = build_test_merged_index(&[
            ("widget", "1.2.0", "server_a"),
            ("widget", "1.2.5", "server_a"),
            ("widget", "1.3.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let current = test_manifest_package("1.2.0", "server_a", PinStrategy::Minor);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.2.5");
    }

    #[test]
    fn resolve_installed_pin_patch_keeps_version() {
        let merged = build_test_merged_index(&[
            ("widget", "1.0.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let current = test_manifest_package("1.0.0", "server_a", PinStrategy::Patch);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.0.0");
    }

    #[test]
    fn resolve_installed_pin_patch_ignores_newer_on_other_server() {
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "1.0.0", "server_a", 1),
            ("widget", "2.0.0", "server_b", 2),
        ]);
        let current = test_manifest_package("1.0.0", "server_a", PinStrategy::Patch);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.0.0");
    }

    #[test]
    fn resolve_installed_falls_back_to_other_server() {
        let merged = build_test_merged_index_with_priorities(&[("widget", "2.0.0", "server_b", 2)]);
        let current = test_manifest_package("1.0.0", "server_a", PinStrategy::None);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "2.0.0");
        assert_eq!(resolved.installed_from, "server_b");
    }

    #[test]
    fn resolve_installed_prefers_same_server() {
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "2.0.0", "server_a", 1),
            ("widget", "2.0.0", "server_b", 2),
        ]);
        let current = test_manifest_package("1.0.0", "server_b", PinStrategy::None);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.installed_from, "server_b");
        assert_eq!(resolved.version, "2.0.0");
    }

    #[test]
    fn resolve_installed_affinity_wins_over_other_server() {
        // Server affinity means: if the original server has a valid version,
        // pick from there even if another server has a newer one. Affinity
        // server_a has 1.1.0, server_b has 2.0.0. With PinStrategy::None,
        // affinity picks 1.1.0 from server_a because the concept doc says
        // "first look for matches on the same server".
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "1.1.0", "server_a", 2),
            ("widget", "2.0.0", "server_b", 1),
        ]);
        let current = test_manifest_package("1.0.0", "server_a", PinStrategy::None);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.1.0");
        assert_eq!(resolved.installed_from, "server_a");
    }

    #[test]
    fn resolve_installed_pin_major_with_server_fallback() {
        let merged = build_test_merged_index_with_priorities(&[
            ("widget", "1.5.0", "server_b", 2),
            ("widget", "2.0.0", "server_b", 2),
        ]);
        let current = test_manifest_package("1.0.0", "server_a", PinStrategy::Major);
        let resolved =
            resolve_installed_package(&merged, "widget", &current).expect("BUG: resolve failed");
        assert_eq!(resolved.version, "1.5.0");
        assert_eq!(resolved.installed_from, "server_b");
    }

    #[test]
    fn resolve_installed_no_matching_version_returns_error() {
        let merged = build_test_merged_index(&[
            ("widget", "1.3.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let current = test_manifest_package("1.2.0", "server_a", PinStrategy::Minor);
        let result = resolve_installed_package(&merged, "widget", &current);
        assert!(result.is_err());
    }
}

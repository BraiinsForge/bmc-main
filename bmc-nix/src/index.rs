// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::{BTreeMap, HashSet, VecDeque};

use futures::future::try_join_all;
use semver::{Version, VersionReq};
use tracing::{debug, warn};

use crate::types::{
    FetchedIndex, InstalledBy, ManifestPackage, MergedIndex, MergedPackageEntry, PackageIndex,
    ResolvedPackage, ServerEntry,
};

pub const PACKAGE_INDEX_VERSION: u32 = 1;
pub const FACTORY_INDEX_VERSION: u32 = 1;

#[must_use]
pub fn make_index_url(base_url: &str) -> String {
    format!(
        "{}/nix-package-index.v{}.json",
        base_url.trim_end_matches('/'),
        PACKAGE_INDEX_VERSION,
    )
}

#[must_use]
pub fn make_factory_url(base_url: &str) -> String {
    format!(
        "{}/nix-factory.v{}.json",
        base_url.trim_end_matches('/'),
        FACTORY_INDEX_VERSION,
    )
}

/// Resolve all packages in an index to [`ResolvedPackage`] values.
///
/// Each package entry is given Stage 1 defaults: `installed_by = System`,
/// `installed_from = "local"`, `pinned = None`. Cache metadata from
/// `PackageEntry.cache` is intentionally ignored — store paths are
/// realised through configured Nix substituters.
///
/// This is used by `bmc-nix-cli build-profile` when packages are
/// already present in the local Nix store.
#[must_use]
pub fn resolve_all_from_index(index: &PackageIndex) -> Vec<ResolvedPackage> {
    index
        .packages
        .iter()
        .map(|entry| ResolvedPackage {
            name: entry.name.clone(),
            version: entry.version.clone(),
            store_path: entry.store_path.clone(),
            category: entry.category.clone(),
            description: entry.description.clone(),
            upgrade_strategy: entry.upgrade_strategy.clone(),
            install_strategy: entry.install_strategy.clone(),
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: None,
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
        source: serde_json::Error,
    },
    #[error("unsupported index version {version} from {url}")]
    UnsupportedVersion { url: String, version: u32 },
    #[error("federated index walk exceeded the {limit}-index cap")]
    TooManyIndexes { limit: usize },
}

/// Errors that can occur when resolving a single package.
#[derive(Debug, thiserror::Error)]
pub enum ResolvePackageError {
    #[error("package '{0}' not found in any index")]
    PackageNotFound(String),
    #[error("no version matching '{constraint}' for package '{package}'")]
    VersionNotFound { package: String, constraint: String },
    #[error("invalid version constraint '{constraint}'")]
    InvalidVersionConstraint { constraint: String },
    #[error(
        "ambiguous resolution for package '{package}': multiple servers at same version and priority"
    )]
    Ambiguous { package: String },
}

/// Parse and validate an index response body.
///
/// Pure function — no I/O. Returns `InvalidJson` on bad JSON,
/// `UnsupportedVersion` when `index.version != PACKAGE_INDEX_VERSION`,
/// otherwise the parsed [`PackageIndex`].
fn parse_and_validate_index(url: &str, body: &[u8]) -> Result<PackageIndex, FetchIndexesError> {
    let index: PackageIndex =
        serde_json::from_slice(body).map_err(|source| FetchIndexesError::InvalidJson {
            url: url.to_owned(),
            source,
        })?;
    if index.version != PACKAGE_INDEX_VERSION {
        return Err(FetchIndexesError::UnsupportedVersion {
            url: url.to_owned(),
            version: index.version,
        });
    }
    Ok(index)
}

/// Fetch a single index from a base URL.
///
/// The full URL is constructed via [`make_index_url`] and includes the
/// versioned filename suffix.
pub async fn fetch_index(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<PackageIndex, FetchIndexesError> {
    let url = make_index_url(base_url);
    let body = client
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|source| FetchIndexesError::Fetch {
            url: url.clone(),
            source,
        })?
        .bytes()
        .await
        .map_err(|source| FetchIndexesError::Fetch {
            url: url.clone(),
            source,
        })?;
    parse_and_validate_index(&url, &body)
}

/// Upper bound on the total number of indexes fetched in a single
/// [`fetch_and_merge_indexes`] call (primaries plus federated children).
///
/// A federated index lists further `indexes` URLs, each of which may list
/// more, so a hostile or compromised index can drive an unbounded
/// sequential walk that fetches and retains every reachable index. This
/// cap bounds that work; 256 is generous for any real mirror topology.
const MAX_TOTAL_INDEXES: usize = 256;

/// Canonicalize a federation base URL for cycle detection so that values
/// differing only by trailing slashes (`http://x` vs `http://x/`) are
/// treated as one and cannot inflate the visited set or bypass the walk
/// cap.
fn canonical_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_owned()
}

/// Fetch and merge indexes from all servers, following federated
/// `indexes` URLs with visited-set cycle detection.
///
/// Top-level fetch failures abort the whole call with `Err`. Federated
/// child failures are logged and skipped; the merge continues. Child
/// references are also rejected unless their `base_url` is `http://` or
/// `https://` (top-level configured servers keep `file://` support). The
/// total number of index fetch attempts — the enabled top-level servers plus
/// every federated child, counting successes, failures, and scheme rejections
/// alike — is capped at [`MAX_TOTAL_INDEXES`]; exceeding it is a fatal
/// [`FetchIndexesError::TooManyIndexes`] rather than a silent truncation, so
/// resolution stays deterministic.
pub async fn fetch_and_merge_indexes(
    client: &reqwest::Client,
    servers: &[ServerEntry],
) -> Result<MergedIndex, FetchIndexesError> {
    fetch_and_merge_indexes_with_cap(client, servers, MAX_TOTAL_INDEXES).await
}

/// [`fetch_and_merge_indexes`] with an explicit walk cap, so the federation
/// bound can be exercised without building hundreds of fixtures.
async fn fetch_and_merge_indexes_with_cap(
    client: &reqwest::Client,
    servers: &[ServerEntry],
    max_total_indexes: usize,
) -> Result<MergedIndex, FetchIndexesError> {
    let mut enabled_servers: Vec<&ServerEntry> = servers.iter().filter(|s| s.enabled).collect();
    enabled_servers.sort_by_key(|s| s.priority);

    if enabled_servers.len() > max_total_indexes {
        return Err(FetchIndexesError::TooManyIndexes {
            limit: max_total_indexes,
        });
    }

    let primary_results: Vec<FetchedIndex> =
        try_join_all(enabled_servers.iter().map(|server| async move {
            let url = make_index_url(&server.base_url);
            let index = fetch_index(client, &server.base_url).await?;
            let commit = index
                .provenance
                .as_ref()
                .map_or("none", |p| p.commit.as_str());
            debug!(
                url = %url,
                server_id = %server.id,
                commit = %commit,
                "fetched index",
            );
            Ok::<_, FetchIndexesError>(FetchedIndex {
                server_id: server.id.clone(),
                server_priority: server.priority,
                index,
            })
        }))
        .await?;

    let mut all_fetched: Vec<FetchedIndex> = Vec::new();
    let mut visited: HashSet<String> = enabled_servers
        .iter()
        .map(|s| canonical_base_url(&s.base_url))
        .collect();

    let mut queue: VecDeque<(String, u32, String)> = VecDeque::new();
    for fetched in primary_results {
        for child_base_url in &fetched.index.indexes {
            queue.push_back((
                fetched.server_id.clone(),
                fetched.server_priority,
                child_base_url.clone(),
            ));
        }
        all_fetched.push(fetched);
    }

    // Counts fetch attempts, not successes, so a hostile or flaky index
    // that lists many unreachable children still exhausts the cap instead
    // of letting the walk retry them forever.
    let mut attempted: usize = enabled_servers.len();

    while let Some((server_id, server_priority, base_url)) = queue.pop_front() {
        if !visited.insert(canonical_base_url(&base_url)) {
            continue;
        }
        attempted += 1;
        if attempted > max_total_indexes {
            return Err(FetchIndexesError::TooManyIndexes {
                limit: max_total_indexes,
            });
        }
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            warn!(
                base_url = %base_url,
                server_id = %server_id,
                "federated child index has unsupported scheme, skipping"
            );
            continue;
        }
        match fetch_index(client, &base_url).await {
            Ok(child) => {
                let url = make_index_url(&base_url);
                let commit = child
                    .provenance
                    .as_ref()
                    .map_or("none", |p| p.commit.as_str());
                debug!(
                    url = %url,
                    server_id = %server_id,
                    commit = %commit,
                    "fetched federated index",
                );
                for child_base_url in &child.indexes {
                    queue.push_back((server_id.clone(), server_priority, child_base_url.clone()));
                }
                all_fetched.push(FetchedIndex {
                    server_id: server_id.clone(),
                    server_priority,
                    index: child,
                });
            }
            Err(e) => {
                warn!(
                    error = %e,
                    server_id = %server_id,
                    "federated index fetch failed, skipping"
                );
            }
        }
    }

    Ok(merge_indexes(all_fetched))
}

/// Parse a package version string into a [`semver::Version`], tolerating
/// the 1- or 2-component versions that real Nix packages use (e.g. `0.8`
/// -> `0.8.0`). A pre-release or build suffix is preserved. Returns
/// `None` for versions that are not numeric-dotted at the core; those are
/// not semver-comparable and remain unsupported.
fn parse_package_version(raw: &str) -> Option<Version> {
    if let Ok(version) = Version::parse(raw) {
        return Some(version);
    }
    let core_end = raw.find(['-', '+']).unwrap_or(raw.len());
    let (core, suffix) = raw.split_at(core_end);
    if core.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = core.split('.').collect();
    if parts.len() >= 3 {
        return None;
    }
    while parts.len() < 3 {
        parts.push("0");
    }
    Version::parse(&format!("{}{}", parts.join("."), suffix)).ok()
}

/// Merge pre-fetched indexes into a single [`MergedIndex`].
///
/// Package entries with an unsupported version field are skipped with a
/// warning and do not abort the merge.
pub fn merge_indexes(fetched: Vec<FetchedIndex>) -> MergedIndex {
    let mut packages = Vec::new();
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for fi in fetched {
        let FetchedIndex {
            server_id,
            server_priority,
            index,
        } = fi;

        for entry in &index.packages {
            let Some(version) = parse_package_version(&entry.version) else {
                warn!(
                    package = %entry.name,
                    version = %entry.version,
                    "skipping package with unsupported version"
                );
                continue;
            };

            let idx = packages.len();
            packages.push(MergedPackageEntry {
                name: entry.name.clone(),
                version,
                store_path: entry.store_path.clone(),
                category: entry.category.clone(),
                description: entry.description.clone(),
                upgrade_strategy: entry.upgrade_strategy.clone(),
                install_strategy: entry.install_strategy.clone(),
                server_id: server_id.clone(),
                server_priority,
            });

            by_name.entry(entry.name.clone()).or_default().push(idx);
        }
    }

    MergedIndex { packages, by_name }
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

    if let Some(constraint_str) = version {
        let constraint = VersionConstraint::parse(constraint_str)?;
        candidates.retain(|e| constraint.matches(&e.version));
        if candidates.is_empty() {
            return Err(ResolvePackageError::VersionNotFound {
                package: name.to_owned(),
                constraint: constraint_str.to_owned(),
            });
        }
    }

    pick_best_candidate(name, &candidates, installed_by, None)
}

/// Resolve an already-installed package to its upgraded version.
///
/// Uses the manifest entry to determine source server and pin constraint.
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

    let pin_filtered: Vec<&MergedPackageEntry> = match &current.pinned {
        Some(constraint_str) => {
            let constraint = VersionConstraint::parse(constraint_str)?;
            all_entries
                .iter()
                .filter(|e| constraint.matches(&e.version))
                .copied()
                .collect()
        }
        None => all_entries.clone(),
    };

    if pin_filtered.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: current.pinned.clone().unwrap_or_else(|| "*".to_owned()),
        });
    }

    // An `upgrade` must never activate a store path older than the
    // installed one. Drop candidates below the installed version while
    // keeping equal ones, so same-version store-path rebuilds still
    // resolve. A malformed installed version disables the guard rather
    // than masking every candidate as stale.
    let no_downgrade: Vec<&MergedPackageEntry> = match parse_package_version(&current.version) {
        Some(current_version) => pin_filtered
            .iter()
            .filter(|e| e.version >= current_version)
            .copied()
            .collect(),
        None => pin_filtered.clone(),
    };

    if no_downgrade.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: current.pinned.clone().unwrap_or_else(|| "*".to_owned()),
        });
    }

    let same_server: Vec<&MergedPackageEntry> = no_downgrade
        .iter()
        .filter(|e| e.server_id == current.installed_from)
        .copied()
        .collect();

    if !same_server.is_empty() {
        return pick_best_candidate(
            name,
            &same_server,
            current.installed_by.clone(),
            current.pinned.clone(),
        );
    }

    pick_best_candidate(
        name,
        &no_downgrade,
        current.installed_by.clone(),
        current.pinned.clone(),
    )
}

/// From a set of candidates, pick the best one: latest version, then
/// lowest priority on tie, then fail if still ambiguous.
fn pick_best_candidate(
    name: &str,
    candidates: &[&MergedPackageEntry],
    installed_by: InstalledBy,
    pinned: Option<String>,
) -> Result<ResolvedPackage, ResolvePackageError> {
    if candidates.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: "no candidates available".to_owned(),
        });
    }

    let mut sorted: Vec<&MergedPackageEntry> = candidates.to_vec();
    sorted.sort_by(|a, b| {
        b.version
            .cmp(&a.version)
            .then(a.server_priority.cmp(&b.server_priority))
    });

    let best = sorted[0];

    // A version/priority tie is only a conflict when the tied entries
    // disagree on the store path; servers mirroring the byte-identical
    // package are not ambiguous.
    let conflicting = sorted[1..]
        .iter()
        .take_while(|entry| {
            entry.version == best.version && entry.server_priority == best.server_priority
        })
        .any(|entry| entry.store_path != best.store_path);
    if conflicting {
        return Err(ResolvePackageError::Ambiguous {
            package: name.to_owned(),
        });
    }

    Ok(merged_entry_to_resolved(best, installed_by, pinned))
}

/// Convert a [`MergedPackageEntry`] to a [`ResolvedPackage`].
fn merged_entry_to_resolved(
    entry: &MergedPackageEntry,
    installed_by: InstalledBy,
    pinned: Option<String>,
) -> ResolvedPackage {
    ResolvedPackage {
        name: entry.name.clone(),
        version: entry.version.to_string(),
        store_path: entry.store_path.clone(),
        category: entry.category.clone(),
        description: entry.description.clone(),
        upgrade_strategy: entry.upgrade_strategy.clone(),
        install_strategy: entry.install_strategy.clone(),
        installed_by,
        installed_from: entry.server_id.clone(),
        pinned,
    }
}

/// A parsed package version constraint.
///
/// A bare, fully specified version (`1.2.3`) means exactly that version.
/// Every other form is a `semver` range (`^1.2`, `>=1.2, <2`, `*`, and a
/// bare partial such as `1.2`, which `VersionReq` reads as `^1.2`).
enum VersionConstraint {
    Exact(Version),
    Range(VersionReq),
}

impl VersionConstraint {
    fn parse(constraint: &str) -> Result<Self, ResolvePackageError> {
        if let Ok(version) = Version::parse(constraint) {
            return Ok(Self::Exact(version));
        }
        VersionReq::parse(constraint).map(Self::Range).map_err(|_| {
            ResolvePackageError::InvalidVersionConstraint {
                constraint: constraint.to_owned(),
            }
        })
    }

    fn matches(&self, version: &Version) -> bool {
        match self {
            Self::Exact(exact) => version == exact,
            Self::Range(req) => req.matches(version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CacheEntry, PackageEntry, PackageIndex, Provenance};

    #[test]
    fn make_index_url_normalizes_configured_base_url() {
        assert_eq!(
            make_index_url("https://cache.braiins.com/v1"),
            "https://cache.braiins.com/v1/nix-package-index.v1.json"
        );
    }

    #[test]
    fn make_factory_url_normalizes_configured_base_url() {
        assert_eq!(
            make_factory_url("https://cache.braiins.com/v1"),
            "https://cache.braiins.com/v1/nix-factory.v1.json"
        );
    }

    #[test]
    fn make_index_url_trims_trailing_slashes() {
        assert_eq!(
            make_index_url("https://cache.braiins.com/v1///"),
            "https://cache.braiins.com/v1/nix-package-index.v1.json"
        );
    }

    #[test]
    fn make_factory_url_trims_trailing_slashes() {
        assert_eq!(
            make_factory_url("https://cache.braiins.com/v1/"),
            "https://cache.braiins.com/v1/nix-factory.v1.json"
        );
    }

    /// Helper: build a minimal `PackageIndex` from given caches and packages.
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

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 1);
        let pkg = &resolved[0];
        assert_eq!(pkg.name, "hello");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.store_path, "/nix/store/abc-hello-1.0.0");
        assert_eq!(pkg.installed_from, "local");
        assert_eq!(pkg.pinned, None);
        assert!(matches!(pkg.installed_by, InstalledBy::System));
    }

    #[test]
    fn resolve_all_ignores_missing_named_cache() {
        let index = make_index(
            vec![default_cache()],
            vec![make_package("broken", Some("nonexistent"))],
        );

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "broken");
    }

    #[test]
    fn resolve_all_accepts_empty_cache_list() {
        let index = make_index(vec![], vec![make_package("orphan", None)]);

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "orphan");
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

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 3);
        let names: Vec<&str> = resolved.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);

        for pkg in &resolved {
            assert!(matches!(pkg.installed_by, InstalledBy::System));
            assert_eq!(pkg.installed_from, "local");
        }
    }

    // ---- helpers for merge/resolve tests ----

    fn fetched(server_id: &str, priority: u32, packages: Vec<PackageEntry>) -> FetchedIndex {
        FetchedIndex {
            server_id: server_id.to_owned(),
            server_priority: priority,
            index: make_index(vec![default_cache()], packages),
        }
    }

    fn versioned_package(name: &str, version: &str, store_path: &str) -> PackageEntry {
        PackageEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            cache: Some("default".to_owned()),
            store_path: store_path.to_owned(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            server_id: String::new(),
        }
    }

    // ---- merge_indexes tests ----

    #[test]
    fn merge_indexes_ignores_cache_metadata() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("clock", "1.0.0", "/nix/store/clock")],
        )]);

        let pkg = &merged.packages[0];
        assert_eq!(pkg.name, "clock");
        assert_eq!(pkg.server_id, "braiins");
        assert_eq!(pkg.store_path, "/nix/store/clock");
    }

    #[test]
    fn merge_indexes_skips_invalid_semver() {
        let packages = vec![
            versioned_package("good", "1.0.0", "/nix/store/good"),
            versioned_package("bad", "not-semver", "/nix/store/bad"),
        ];
        let merged = merge_indexes(vec![fetched("braiins", 10, packages)]);

        assert_eq!(
            merged.packages.len(),
            1,
            "invalid semver package should be skipped"
        );
        assert_eq!(merged.packages[0].name, "good");
        assert!(
            !merged.by_name.contains_key("bad"),
            "bad package must not appear in by_name"
        );
    }

    #[test]
    fn parse_package_version_pads_short_cores() {
        assert_eq!(
            parse_package_version("0.8"),
            Some(Version::new(0, 8, 0)),
            "two-component versions must pad to a patch of zero"
        );
        assert_eq!(
            parse_package_version("1"),
            Some(Version::new(1, 0, 0)),
            "single-component versions must pad to minor and patch zero"
        );
        assert_eq!(
            parse_package_version("1.2.3"),
            Some(Version::new(1, 2, 3)),
            "full versions must pass through unchanged"
        );
    }

    #[test]
    fn parse_package_version_preserves_prerelease() {
        let parsed = parse_package_version("0.8-rc1").expect("BUG: padded pre-release must parse");
        assert_eq!(
            parsed,
            Version::parse("0.8.0-rc1").expect("BUG: reference version"),
            "padding must keep the pre-release suffix"
        );
    }

    #[test]
    fn parse_package_version_rejects_non_numeric_cores() {
        assert_eq!(parse_package_version("1.2.3.4"), None);
        assert_eq!(parse_package_version("abc"), None);
        assert_eq!(parse_package_version(""), None);
    }

    #[test]
    fn merge_indexes_includes_two_component_version() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("bos-avahi", "0.8", "/nix/store/avahi")],
        )]);

        let indices = merged
            .by_name
            .get("bos-avahi")
            .expect("BUG: a two-component nix version must survive the merge");
        assert_eq!(indices.len(), 1);
        assert_eq!(merged.packages[indices[0]].version, Version::new(0, 8, 0));
    }

    // ---- resolve_new_package tests ----

    #[test]
    fn resolve_new_package_picks_latest_version_then_lowest_priority() {
        let merged = merge_indexes(vec![
            fetched(
                "slow",
                20,
                vec![versioned_package("clock", "2.0.0", "/nix/store/slow")],
            ),
            fetched(
                "fast",
                10,
                vec![versioned_package("clock", "2.0.0", "/nix/store/fast")],
            ),
            fetched(
                "old",
                1,
                vec![versioned_package("clock", "1.9.0", "/nix/store/old")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "clock", None, InstalledBy::User)
            .expect("BUG: package should resolve");

        assert_eq!(resolved.version, "2.0.0");
        assert_eq!(resolved.installed_from, "fast");
        assert_eq!(resolved.store_path, "/nix/store/fast");
    }

    #[test]
    fn resolve_new_package_rejects_same_version_same_priority_ambiguity() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "2.0.0", "/nix/store/a")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "2.0.0", "/nix/store/b")],
            ),
        ]);

        let err = resolve_new_package(&merged, "clock", None, InstalledBy::User)
            .expect_err("same version and priority should be ambiguous");

        assert!(matches!(err, ResolvePackageError::Ambiguous { .. }));
    }

    #[test]
    fn resolve_new_package_accepts_identical_entries_from_tied_servers() {
        // Two servers mirroring the byte-identical package (same version,
        // priority, and store path) offer the same target; that is not a
        // conflict.
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "2.0.0", "/nix/store/same")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "2.0.0", "/nix/store/same")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "clock", None, InstalledBy::User)
            .expect("BUG: identical tied entries must resolve, not read as ambiguous");

        assert_eq!(resolved.store_path, "/nix/store/same");
    }

    #[test]
    fn resolve_new_package_exact_version_rejects_other_patches() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "1.2.3", "/nix/store/v123")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "1.2.4", "/nix/store/v124")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "clock", Some("1.2.3"), InstalledBy::User)
            .expect("BUG: exact version should resolve");

        assert_eq!(resolved.version, "1.2.3");
        assert_eq!(resolved.store_path, "/nix/store/v123");
    }

    #[test]
    fn resolve_new_package_two_component_pin_resolves_padded_entry() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("avahi", "0.8", "/nix/store/v08")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("avahi", "0.9", "/nix/store/v09")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "avahi", Some("0.8"), InstalledBy::User)
            .expect("BUG: a two-component exact pin should resolve");

        assert_eq!(resolved.version, "0.8.0");
        assert_eq!(resolved.store_path, "/nix/store/v08");
    }

    #[test]
    fn resolve_new_package_caret_constraint_picks_latest_in_range() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "1.2.0", "/nix/store/v120")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "1.5.0", "/nix/store/v150")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "clock", Some("^1.2.0"), InstalledBy::User)
            .expect("BUG: caret constraint should resolve");

        assert_eq!(resolved.version, "1.5.0");
    }

    #[test]
    fn resolve_new_package_range_constraint_excludes_out_of_range() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "1.2.0", "/nix/store/v120")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "1.5.0", "/nix/store/v150")],
            ),
            fetched(
                "c",
                10,
                vec![versioned_package("clock", "2.0.0", "/nix/store/v200")],
            ),
        ]);

        let resolved =
            resolve_new_package(&merged, "clock", Some(">=1.2, <2.0.0"), InstalledBy::User)
                .expect("BUG: range constraint should resolve");

        assert_eq!(resolved.version, "1.5.0");
    }

    #[test]
    fn resolve_new_package_bare_partial_behaves_as_caret() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "1.2.0", "/nix/store/v120")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "1.9.0", "/nix/store/v190")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "clock", Some("1.2"), InstalledBy::User)
            .expect("BUG: bare partial constraint should resolve");

        assert_eq!(resolved.version, "1.9.0");
    }

    #[test]
    fn resolve_new_package_prerelease_matches_exactly() {
        let merged = merge_indexes(vec![fetched(
            "a",
            10,
            vec![versioned_package("clock", "1.2.3-rc1", "/nix/store/rc1")],
        )]);

        let resolved = resolve_new_package(&merged, "clock", Some("1.2.3-rc1"), InstalledBy::User)
            .expect("BUG: pre-release exact should resolve");

        assert_eq!(resolved.version, "1.2.3-rc1");
    }

    #[test]
    fn resolve_new_package_invalid_constraint_returns_invalid_error() {
        let merged = merge_indexes(vec![fetched(
            "a",
            10,
            vec![versioned_package("clock", "1.2.0", "/nix/store/v120")],
        )]);

        let err = resolve_new_package(&merged, "clock", Some("not-a-version"), InstalledBy::User)
            .expect_err("malformed constraint should error");

        assert!(matches!(
            err,
            ResolvePackageError::InvalidVersionConstraint { constraint }
                if constraint == "not-a-version"
        ));
    }

    #[test]
    fn resolve_new_package_wildcard_constraint_matches_within_minor() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                10,
                vec![versioned_package("clock", "1.2.7", "/nix/store/v127")],
            ),
            fetched(
                "b",
                10,
                vec![versioned_package("clock", "1.3.0", "/nix/store/v130")],
            ),
        ]);

        let resolved = resolve_new_package(&merged, "clock", Some("1.2.x"), InstalledBy::User)
            .expect("BUG: wildcard constraint should resolve");

        assert_eq!(resolved.version, "1.2.7");
    }

    #[test]
    fn resolve_new_package_valid_constraint_no_match_returns_version_not_found() {
        let merged = merge_indexes(vec![fetched(
            "a",
            10,
            vec![versioned_package("clock", "1.2.0", "/nix/store/v120")],
        )]);

        let err = resolve_new_package(&merged, "clock", Some("^3.0.0"), InstalledBy::User)
            .expect_err("no matching version should error");

        assert!(matches!(err, ResolvePackageError::VersionNotFound { .. }));
    }

    // ---- resolve_installed_package tests ----

    #[test]
    fn resolve_installed_package_prefers_same_server_when_allowed_by_pin() {
        let merged = merge_indexes(vec![
            fetched(
                "braiins",
                20,
                vec![versioned_package("clock", "1.2.0", "/nix/store/same")],
            ),
            fetched(
                "other",
                1,
                vec![versioned_package("clock", "1.3.0", "/nix/store/other")],
            ),
        ]);
        let current = ManifestPackage {
            version: "1.0.0".into(),
            store_path: "/nix/store/current".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: Some("^1.0.0".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: package should resolve");

        assert_eq!(resolved.installed_from, "braiins");
        assert_eq!(resolved.version, "1.2.0");
    }

    #[test]
    fn resolve_installed_package_respects_patch_pin() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![
                versioned_package("clock", "1.0.0", "/nix/store/exact"),
                versioned_package("clock", "1.0.1", "/nix/store/newer"),
            ],
        )]);
        let current = ManifestPackage {
            version: "1.0.0".into(),
            store_path: "/nix/store/exact".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: Some("1.0.0".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: package should resolve at exact version");

        assert_eq!(resolved.version, "1.0.0");
    }

    #[test]
    fn resolve_installed_package_unpinned_resolves_latest() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![
                versioned_package("clock", "1.2.0", "/nix/store/v120"),
                versioned_package("clock", "1.5.0", "/nix/store/v150"),
            ],
        )]);
        let current = ManifestPackage {
            version: "1.0.0".into(),
            store_path: "/nix/store/current".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: unpinned package should resolve");

        assert_eq!(resolved.version, "1.5.0");
    }

    #[test]
    fn resolve_installed_package_range_pin_limits_upgrade() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![
                versioned_package("clock", "1.2.5", "/nix/store/v125"),
                versioned_package("clock", "1.3.0", "/nix/store/v130"),
            ],
        )]);
        let current = ManifestPackage {
            version: "1.2.0".into(),
            store_path: "/nix/store/current".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: Some("~1.2".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: tilde-pinned package should resolve within minor");

        assert_eq!(resolved.version, "1.2.5");
    }

    #[test]
    fn resolve_installed_package_refuses_downgrade_when_index_only_older() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("clock", "1.4.0", "/nix/store/v140")],
        )]);
        let current = ManifestPackage {
            version: "1.5.0".into(),
            store_path: "/nix/store/v150".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: Some("^1.0.0".to_owned()),
        };

        let err = resolve_installed_package(&merged, "clock", &current)
            .expect_err("an older-only index must not downgrade an installed package");

        assert!(
            matches!(err, ResolvePackageError::VersionNotFound { .. }),
            "expected VersionNotFound (→ stale), got {err:?}"
        );
    }

    #[test]
    fn resolve_installed_package_upgrades_to_newer_in_pin() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("clock", "1.1.0", "/nix/store/v110")],
        )]);
        let current = ManifestPackage {
            version: "1.0.0".into(),
            store_path: "/nix/store/v100".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: Some("^1.0.0".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: a newer in-pin version should still upgrade");

        assert_eq!(resolved.version, "1.1.0");
        assert_eq!(resolved.store_path, "/nix/store/v110");
    }

    #[test]
    fn resolve_installed_package_allows_same_version_store_path_rebuild() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("clock", "1.0.0", "/nix/store/rebuilt")],
        )]);
        let current = ManifestPackage {
            version: "1.0.0".into(),
            store_path: "/nix/store/original".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: same-version store-path rebuild should resolve");

        assert_eq!(resolved.version, "1.0.0");
        assert_eq!(resolved.store_path, "/nix/store/rebuilt");
    }

    #[test]
    fn resolve_installed_two_component_current_upgrades_to_newer() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![
                versioned_package("avahi", "0.8", "/nix/store/v08"),
                versioned_package("avahi", "0.9", "/nix/store/v09"),
            ],
        )]);
        let current = ManifestPackage {
            version: "0.8".into(),
            store_path: "/nix/store/v08".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "avahi", &current)
            .expect("BUG: a two-component current version should upgrade");

        assert_eq!(resolved.version, "0.9.0");
    }

    #[test]
    fn resolve_installed_two_component_current_resolves_same_version() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("avahi", "0.8", "/nix/store/v08")],
        )]);
        let current = ManifestPackage {
            version: "0.8".into(),
            store_path: "/nix/store/old".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "avahi", &current)
            .expect("BUG: a same-version two-component current must not be stale");

        assert_eq!(resolved.version, "0.8.0");
    }

    #[test]
    fn resolve_installed_two_component_current_refuses_downgrade() {
        let merged = merge_indexes(vec![fetched(
            "braiins",
            10,
            vec![versioned_package("avahi", "0.7", "/nix/store/v07")],
        )]);
        let current = ManifestPackage {
            version: "0.8".into(),
            store_path: "/nix/store/v08".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: None,
        };

        let err = resolve_installed_package(&merged, "avahi", &current)
            .expect_err("an older-only index must not downgrade a two-component current");

        assert!(matches!(err, ResolvePackageError::VersionNotFound { .. }));
    }

    // ---- fetch_index tests (pure parse_and_validate_index path) ----

    #[tokio::test]
    async fn fetch_index_uses_versioned_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock server");
        let addr = listener.local_addr().expect("BUG: no local addr");

        let body = format!(
            r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[],"caches":[],"packages":[]}}"#
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("BUG: mock server failed to accept");
            let mut buf = [0_u8; 4096];
            let n = stream
                .read(&mut buf)
                .await
                .expect("BUG: failed to read request");
            let request = String::from_utf8_lossy(&buf[..n]);
            let request_line = request
                .lines()
                .next()
                .expect("BUG: empty request")
                .to_owned();
            stream
                .write_all(response.as_bytes())
                .await
                .expect("BUG: failed to write response");
            request_line
        });

        let client = reqwest::Client::new();
        let base_url = format!("http://{addr}");
        fetch_index(&client, &base_url)
            .await
            .expect("BUG: fetch_index should succeed against mock server");

        let request_line = server_task.await.expect("BUG: mock server task panicked");

        // e.g. "GET /nix-package-index.v1.json HTTP/1.1"
        let path = request_line
            .split_whitespace()
            .nth(1)
            .expect("BUG: malformed request line");
        let expected = format!("/nix-package-index.v{PACKAGE_INDEX_VERSION}.json");
        assert_eq!(
            path, expected,
            "fetch_index must request the versioned path"
        );
    }

    #[test]
    fn fetch_index_rejects_unsupported_version() {
        let body = br#"{
            "version": 99,
            "provenance": null,
            "indexes": [],
            "caches": [],
            "packages": []
        }"#;
        let err = parse_and_validate_index("http://test/url", body)
            .expect_err("version 99 should be rejected");
        assert!(
            matches!(
                err,
                FetchIndexesError::UnsupportedVersion { version: 99, .. }
            ),
            "expected UnsupportedVersion {{ version: 99 }}, got {err:?}"
        );
    }

    // ---- federation walk / canonicalization / streaming-cap tests ----

    fn write_index(
        dir: &std::path::Path,
        name: &str,
        children: &[String],
        packages: &str,
    ) -> String {
        let children_json = children
            .iter()
            .map(|c| serde_json::to_string(c).expect("BUG: serialize child url"))
            .collect::<Vec<_>>()
            .join(",");
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[{children_json}],"caches":[],"packages":[{packages}]}}"#
            ),
        )
        .expect("BUG: write index file");
        format!("file://{}", path.display())
    }

    fn server_entry(base_url: &str) -> ServerEntry {
        ServerEntry {
            id: "primary".to_owned(),
            server_type: "package".to_owned(),
            base_url: base_url.to_owned(),
            known_public_key: String::new(),
            priority: 10,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn too_many_top_level_servers_exceed_cap() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let root_a = write_index(dir.path(), "a.json", &[], "");
        let root_b = write_index(dir.path(), "b.json", &[], "");
        let root_c = write_index(dir.path(), "c.json", &[], "");

        let client = reqwest::Client::new();
        let servers = vec![
            server_entry(&root_a),
            server_entry(&root_b),
            server_entry(&root_c),
        ];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, 2)
            .await
            .expect_err("more enabled top-level servers than the cap must abort");

        assert!(
            matches!(err, FetchIndexesError::TooManyIndexes { limit: 2 }),
            "expected TooManyIndexes {{ limit: 2 }}, got {err:?}"
        );
    }
}

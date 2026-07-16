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

use std::collections::{BTreeMap, HashSet, VecDeque};

use futures::future::join_all;
use semver::{Version, VersionReq};
use tracing::{debug, warn};

use crate::types::{
    FetchedIndex, InstalledBy, ManifestPackage, MergedIndex, MergedPackageEntry, PackageIndex,
    ResolvedPackage, ServerEntry, ServerSource,
};

pub const PACKAGE_INDEX_VERSION: u32 = 1;

#[must_use]
pub fn make_index_url(base_url: &str) -> String {
    format!(
        "{}/nix-package-index.v{}.json",
        base_url.trim_end_matches('/'),
        PACKAGE_INDEX_VERSION,
    )
}

#[must_use]
pub fn make_package_feed_url(base_url: &str) -> String {
    format!(
        "{}/nix-package-feed.v{}.json",
        base_url.trim_end_matches('/'),
        crate::feed::PACKAGE_FEED_VERSION,
    )
}

/// Errors returned when resolving every package from an index.
#[derive(Debug, thiserror::Error)]
pub enum ResolveAllFromIndexError {
    #[error("system packages missing from index: {}", names.join(", "))]
    MissingSystemPackages { names: Vec<String> },
}

/// Resolve all packages in an index to [`ResolvedPackage`] values.
///
/// Each package entry is given from-scratch defaults: `installed_by =
/// System` when its name is in `system_packages`, `User` otherwise;
/// `installed_from = "local"`; `pinned = None`. The caller names the
/// packages the image is broken without — a `System` package missing
/// from a later merged index aborts the whole upgrade
/// (`MissingSystemPackages`), while every other package degrades to a
/// stale carry instead. Cache metadata from `PackageEntry.cache` is
/// intentionally ignored — store paths are realised through
/// configured Nix substituters.
///
/// This is used by `bmc-nix-cli build-profile` and `reset-profile`
/// when packages are already present in the local Nix store.
///
/// # Errors
///
/// Returns [`ResolveAllFromIndexError::MissingSystemPackages`] when a
/// requested system package is absent from the index.
pub fn resolve_all_from_index(
    index: &PackageIndex,
    system_packages: &[String],
) -> Result<Vec<ResolvedPackage>, ResolveAllFromIndexError> {
    let index_package_names: HashSet<&str> = index
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let mut missing_system_packages: Vec<String> = system_packages
        .iter()
        .filter(|name| !index_package_names.contains(name.as_str()))
        .cloned()
        .collect();
    missing_system_packages.sort();
    missing_system_packages.dedup();

    if !missing_system_packages.is_empty() {
        return Err(ResolveAllFromIndexError::MissingSystemPackages {
            names: missing_system_packages,
        });
    }

    Ok(index
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
            installed_by: if system_packages.contains(&entry.name) {
                InstalledBy::System
            } else {
                InstalledBy::User
            },
            installed_from: "local".into(),
            pinned: None,
            metadata: entry.metadata.clone(),
        })
        .collect())
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
    #[error("index file not found: {path}")]
    FileNotFound { path: String },
    #[error("failed to read index file {path}: {source}")]
    FileRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "index from {location} is too large: {size} bytes exceeds the {MAX_INDEX_BYTES}-byte cap"
    )]
    IndexTooLarge { location: String, size: u64 },
    #[error("federated index walk exceeded the {limit}-index cap")]
    TooManyIndexes { limit: usize },
    #[error("package feed at {url} is invalid JSON: {source}")]
    InvalidFeedJson {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("server '{server_id}': package feed fetch for firmware '{firmware}' failed: {source}")]
    FeedFetch {
        server_id: String,
        firmware: String,
        #[source]
        source: Box<FetchIndexesError>,
    },
    #[error("server '{server_id}': package feed does not serve firmware '{firmware}': {source}")]
    FeedResolution {
        server_id: String,
        firmware: String,
        #[source]
        source: crate::feed::FeedError,
    },
    #[error("server '{server_id}' links a package feed but no firmware scope is available")]
    MissingFirmwareScope { server_id: String },
}

/// Maximum accepted size of a single fetched index, in bytes.
///
/// A JSON package index is small. HTTP fetches reject early on a
/// `Content-Length` over this cap and otherwise accumulate the body chunk
/// by chunk against a running ceiling; `file://` reads reject on the file
/// `metadata` length before reading. This bounds the memory an oversized
/// or hostile response can allocate.
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

/// Reject an index whose byte length exceeds `cap`.
fn check_index_size_with_cap(location: &str, len: u64, cap: u64) -> Result<(), FetchIndexesError> {
    if len > cap {
        return Err(FetchIndexesError::IndexTooLarge {
            location: location.to_owned(),
            size: len,
        });
    }
    Ok(())
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

/// Turn an index reference into `(location, bytes)`, branching on scheme.
///
/// `file://<path>` reads `<path>` verbatim — the reference is the direct
/// path to the index JSON, so no filename is appended. Every other
/// reference is treated as an `http(s)` base URL whose versioned index
/// path is built by [`make_index_url`]. `location` is the resolved URL or
/// path, used for diagnostics and by `parse_and_validate_index`.
async fn fetch_index_bytes(
    client: &reqwest::Client,
    reference: &str,
) -> Result<(String, Vec<u8>), FetchIndexesError> {
    fetch_index_bytes_with_cap(client, reference, MAX_INDEX_BYTES).await
}

/// [`fetch_index_bytes`] with an explicit byte cap, so the streaming and
/// early-reject ceiling can be exercised without a multi-megabyte fixture.
async fn fetch_index_bytes_with_cap(
    client: &reqwest::Client,
    reference: &str,
    cap: u64,
) -> Result<(String, Vec<u8>), FetchIndexesError> {
    if reference.starts_with("file://") {
        return fetch_document_bytes(client, reference, cap).await;
    }
    fetch_http_bytes(client, &make_index_url(reference), cap).await
}

/// Fetch the document at `url` verbatim — no filename is appended for
/// either scheme. `file://<path>` reads `<path>` directly; anything else
/// is requested as-is over http(s), so http(s) URLs must have been
/// validated absolute upstream.
async fn fetch_document_bytes(
    client: &reqwest::Client,
    url: &str,
    cap: u64,
) -> Result<(String, Vec<u8>), FetchIndexesError> {
    if let Some(path) = url.strip_prefix("file://") {
        return fetch_file_bytes(path, cap).await;
    }
    fetch_http_bytes(client, url, cap).await
}

/// Read a `file://` document from `path`, rejecting non-regular files and
/// anything whose metadata length exceeds `cap` before reading.
async fn fetch_file_bytes(path: &str, cap: u64) -> Result<(String, Vec<u8>), FetchIndexesError> {
    let map_io_error = |source: std::io::Error| {
        if source.kind() == std::io::ErrorKind::NotFound {
            FetchIndexesError::FileNotFound {
                path: path.to_owned(),
            }
        } else {
            FetchIndexesError::FileRead {
                path: path.to_owned(),
                source,
            }
        }
    };
    let metadata = tokio::fs::metadata(path).await.map_err(map_io_error)?;
    if !metadata.file_type().is_file() {
        return Err(FetchIndexesError::FileRead {
            path: path.to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "index reference is not a regular file",
            ),
        });
    }
    check_index_size_with_cap(path, metadata.len(), cap)?;
    let bytes = tokio::fs::read(path).await.map_err(map_io_error)?;
    Ok((path.to_owned(), bytes))
}

/// GET `url` over http(s), rejecting early on a `Content-Length` over
/// `cap` and accumulating the body chunk by chunk against the same cap.
async fn fetch_http_bytes(
    client: &reqwest::Client,
    url: &str,
    cap: u64,
) -> Result<(String, Vec<u8>), FetchIndexesError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|source| FetchIndexesError::Fetch {
            url: url.to_owned(),
            source,
        })?;

    if let Some(len) = response.content_length() {
        check_index_size_with_cap(url, len, cap)?;
    }

    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| FetchIndexesError::Fetch {
            url: url.to_owned(),
            source,
        })?
    {
        check_index_size_with_cap(url, (bytes.len() + chunk.len()) as u64, cap)?;
        bytes.extend_from_slice(&chunk);
    }
    Ok((url.to_owned(), bytes))
}

/// Fetch and validate a single index from a reference.
///
/// The reference is an `http(s)` base URL (the versioned filename is
/// appended) or a `file://` path to the index JSON (read verbatim).
pub async fn fetch_index(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<PackageIndex, FetchIndexesError> {
    let (location, body) = fetch_index_bytes(client, base_url).await?;
    parse_and_validate_index(&location, &body)
}

/// Fetch and validate a package index at an exact document URL — no
/// filename appending for either scheme.
async fn fetch_index_document(
    client: &reqwest::Client,
    url: &str,
) -> Result<PackageIndex, FetchIndexesError> {
    let (location, body) = fetch_document_bytes(client, url, MAX_INDEX_BYTES).await?;
    parse_and_validate_index(&location, &body)
}

/// Fetch and deserialize a package feed document at an exact URL.
///
/// Fetch + deserialize ONLY — no validation; the per-server resolver
/// runs [`crate::feed::validate_feed`]/[`crate::feed::select_entry`]/
/// [`crate::feed::require_index_url`] and owns the `FeedError` mapping.
async fn fetch_package_feed(
    client: &reqwest::Client,
    url: &str,
) -> Result<crate::feed::PackageFeed, FetchIndexesError> {
    let (location, body) = fetch_document_bytes(client, url, MAX_INDEX_BYTES).await?;
    serde_json::from_slice(&body).map_err(|source| FetchIndexesError::InvalidFeedJson {
        url: location,
        source,
    })
}

/// Resolve one configured server to its package index.
///
/// An index-linked server fetches its exact document. A feed-linked
/// server fetches the feed, selects the `firmware` entry, and follows
/// that entry's `index_url`; feed selection failures surface as
/// [`FetchIndexesError::FeedResolution`] and transport failures as
/// [`FetchIndexesError::FeedFetch`], both naming the server id and the
/// target firmware. A feed-linked server without a firmware scope is
/// [`FetchIndexesError::MissingFirmwareScope`].
async fn fetch_server_index(
    client: &reqwest::Client,
    server: &ServerEntry,
    firmware: Option<&str>,
) -> Result<PackageIndex, FetchIndexesError> {
    match &server.source {
        ServerSource::Index { index_url } => fetch_index_document(client, index_url).await,
        ServerSource::Feed { feed_url } => {
            let Some(firmware) = firmware else {
                return Err(FetchIndexesError::MissingFirmwareScope {
                    server_id: server.id.clone(),
                });
            };
            let wrap_fetch = |source: FetchIndexesError| FetchIndexesError::FeedFetch {
                server_id: server.id.clone(),
                firmware: firmware.to_owned(),
                source: Box::new(source),
            };
            let wrap_feed = |source: crate::feed::FeedError| FetchIndexesError::FeedResolution {
                server_id: server.id.clone(),
                firmware: firmware.to_owned(),
                source,
            };
            let feed = fetch_package_feed(client, feed_url)
                .await
                .map_err(wrap_fetch)?;
            crate::feed::validate_feed(feed_url, &feed).map_err(wrap_feed)?;
            let entry = crate::feed::select_entry(feed_url, &feed, firmware).map_err(wrap_feed)?;
            let index_url = crate::feed::require_index_url(feed_url, entry).map_err(wrap_feed)?;
            fetch_index_document(client, index_url)
                .await
                .map_err(wrap_fetch)
        }
    }
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

/// An ad-hoc `--index` reference resolved alongside the configured
/// servers.
///
/// `reference` keeps the `--index` flag's semantics: an `http(s)` base
/// URL the well-known index filename is appended to, or a `file://`
/// document path read verbatim. Ad-hoc references are always fatal on
/// fetch failure.
#[derive(Debug, Clone)]
pub struct AdHocIndexRef {
    pub id: String,
    pub reference: String,
}

/// Priority assigned to ad-hoc `--index` references: 0 is the highest
/// precedence, so an ad-hoc index wins a version tie against any
/// configured server.
pub const AD_HOC_INDEX_PRIORITY: u32 = 0;

/// Fetch and merge indexes from all servers and ad-hoc references,
/// following federated `indexes` URLs with visited-set cycle detection.
///
/// Configured servers resolve through their [`ServerSource`]: an exact
/// index document, or a package feed whose `firmware` entry links the
/// index. Required-server and ad-hoc fetch failures abort the whole call
/// with `Err`; optional-server failures degrade to a warning. Federated
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
    ad_hoc: &[AdHocIndexRef],
    firmware: Option<&str>,
) -> Result<MergedIndex, FetchIndexesError> {
    fetch_and_merge_indexes_with_cap(client, servers, ad_hoc, firmware, MAX_TOTAL_INDEXES).await
}

/// Reduce the per-server primary fetch results to the successful indexes.
///
/// A required server's fetch failure aborts the whole merge with its error.
/// An optional server's failure degrades to a warning so the merge proceeds
/// with the servers that did respond. The first optional failure is
/// remembered so that an all-optional set where every fetch failed still
/// surfaces an error rather than silently returning an empty index, unless
/// an ad-hoc source will satisfy source resolution. The `fetch_results` are
/// positionally aligned with `enabled_servers`.
fn select_primary_indexes(
    enabled_servers: &[&ServerEntry],
    fetch_results: Vec<Result<FetchedIndex, FetchIndexesError>>,
    has_ad_hoc_source: bool,
) -> Result<Vec<FetchedIndex>, FetchIndexesError> {
    let mut primary_results: Vec<FetchedIndex> = Vec::new();
    let mut first_optional_error: Option<FetchIndexesError> = None;
    for (server, result) in enabled_servers.iter().zip(fetch_results) {
        match result {
            Ok(fetched) => primary_results.push(fetched),
            Err(error) if server.required => return Err(error),
            Err(error) => {
                warn!(
                    error = %error,
                    server_id = %server.id,
                    "optional server index fetch failed, degrading"
                );
                first_optional_error.get_or_insert(error);
            }
        }
    }

    if primary_results.is_empty() && !enabled_servers.is_empty() && !has_ad_hoc_source {
        return Err(first_optional_error
            .expect("BUG: enabled servers with no successful fetch imply a recorded failure"));
    }

    Ok(primary_results)
}

/// Fetch the top-level fetch set: the enabled configured servers reduced
/// via [`select_primary_indexes`], then the ad-hoc references. Ad-hoc
/// references are reduced separately from the configured servers: any
/// ad-hoc fetch failure is fatal, matching the required synthetic entries
/// they used to be.
async fn fetch_primary_indexes(
    client: &reqwest::Client,
    enabled_servers: &[&ServerEntry],
    ad_hoc: &[AdHocIndexRef],
    firmware: Option<&str>,
) -> Result<Vec<FetchedIndex>, FetchIndexesError> {
    let fetch_results: Vec<Result<FetchedIndex, FetchIndexesError>> =
        join_all(enabled_servers.iter().map(|server| async move {
            let index = fetch_server_index(client, server, firmware).await?;
            let commit = index
                .provenance
                .as_ref()
                .map_or("none", |p| p.commit.as_str());
            debug!(
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
        .await;

    // Every ad-hoc failure is propagated below, so a non-empty ad-hoc set
    // either supplies a successful source or makes the whole call fail.
    let mut primary_results =
        select_primary_indexes(enabled_servers, fetch_results, !ad_hoc.is_empty())?;

    let ad_hoc_results: Vec<Result<FetchedIndex, FetchIndexesError>> =
        join_all(ad_hoc.iter().map(|reference| async move {
            let index = fetch_index(client, &reference.reference).await?;
            Ok::<_, FetchIndexesError>(FetchedIndex {
                server_id: reference.id.clone(),
                server_priority: AD_HOC_INDEX_PRIORITY,
                index,
            })
        }))
        .await;
    for result in ad_hoc_results {
        primary_results.push(result?);
    }

    Ok(primary_results)
}

/// [`fetch_and_merge_indexes`] with an explicit walk cap, so the federation
/// bound can be exercised without building hundreds of fixtures.
async fn fetch_and_merge_indexes_with_cap(
    client: &reqwest::Client,
    servers: &[ServerEntry],
    ad_hoc: &[AdHocIndexRef],
    firmware: Option<&str>,
    max_total_indexes: usize,
) -> Result<MergedIndex, FetchIndexesError> {
    let mut enabled_servers: Vec<&ServerEntry> = servers.iter().filter(|s| s.enabled).collect();
    enabled_servers.sort_by_key(|s| s.priority);

    if enabled_servers.len() + ad_hoc.len() > max_total_indexes {
        return Err(FetchIndexesError::TooManyIndexes {
            limit: max_total_indexes,
        });
    }

    let primary_results = fetch_primary_indexes(client, &enabled_servers, ad_hoc, firmware).await?;

    let mut all_fetched: Vec<FetchedIndex> = Vec::new();
    let mut visited: HashSet<String> = enabled_servers
        .iter()
        .map(|s| {
            canonical_base_url(match &s.source {
                ServerSource::Feed { feed_url } => feed_url,
                ServerSource::Index { index_url } => index_url,
            })
        })
        .chain(ad_hoc.iter().map(|r| canonical_base_url(&r.reference)))
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
    let mut attempted: usize = enabled_servers.len() + ad_hoc.len();

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
pub(crate) fn parse_package_version(raw: &str) -> Option<Version> {
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
                metadata: entry.metadata.clone(),
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
/// Entries are first scoped to the server the package was installed from.
/// If that server still lists the package, resolution stays confined to
/// it: no-downgrade, pin constraint, latest version, and server priority
/// are applied strictly within that server's entries, so an origin stuck
/// on old versions reports stale rather than migrating to another server.
/// Only when the origin server has no entry at all for the package does
/// resolution fall back to the same steps across every server.
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

    let constraint = current
        .pinned
        .as_deref()
        .map(VersionConstraint::parse)
        .transpose()?;

    let origin: Vec<&MergedPackageEntry> = all_entries
        .iter()
        .filter(|e| e.server_id == current.installed_from)
        .copied()
        .collect();

    let scoped = if origin.is_empty() {
        all_entries
    } else {
        origin
    };

    // An `upgrade` must never activate a store path older than the
    // installed one. Drop candidates below the installed version while
    // keeping equal ones, so same-version store-path rebuilds still
    // resolve. A malformed installed version disables the guard rather
    // than masking every candidate as stale.
    let no_downgrade: Vec<&MergedPackageEntry> = match parse_package_version(&current.version) {
        Some(current_version) => scoped
            .iter()
            .filter(|e| e.version >= current_version)
            .copied()
            .collect(),
        None => scoped,
    };

    if no_downgrade.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: current.pinned.clone().unwrap_or_else(|| "*".to_owned()),
        });
    }

    let candidates: Vec<&MergedPackageEntry> = match &constraint {
        Some(constraint) => no_downgrade
            .into_iter()
            .filter(|e| constraint.matches(&e.version))
            .collect(),
        None => no_downgrade,
    };

    if candidates.is_empty() {
        return Err(ResolvePackageError::VersionNotFound {
            package: name.to_owned(),
            constraint: current.pinned.clone().unwrap_or_else(|| "*".to_owned()),
        });
    }

    pick_best_candidate(
        name,
        &candidates,
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
        metadata: entry.metadata.clone(),
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
    fn make_package_feed_url_normalizes_configured_base_url() {
        assert_eq!(
            make_package_feed_url("https://cache.braiins.com/v1"),
            "https://cache.braiins.com/v1/nix-package-feed.v1.json"
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
    fn make_package_feed_url_trims_trailing_slashes() {
        assert_eq!(
            make_package_feed_url("https://cache.braiins.com/v1/"),
            "https://cache.braiins.com/v1/nix-package-feed.v1.json"
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
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn resolve_all_from_index_basic() {
        let index = make_index(vec![default_cache()], vec![make_package("hello", None)]);

        let resolved = resolve_all_from_index(&index, &[])
            .expect("an index without required system packages must resolve");

        assert_eq!(resolved.len(), 1);
        let pkg = &resolved[0];
        assert_eq!(pkg.name, "hello");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.store_path, "/nix/store/abc-hello-1.0.0");
        assert_eq!(pkg.installed_from, "local");
        assert_eq!(pkg.pinned, None);
        assert!(matches!(pkg.installed_by, InstalledBy::User));
    }

    #[test]
    fn resolve_all_ignores_missing_named_cache() {
        let index = make_index(
            vec![default_cache()],
            vec![make_package("broken", Some("nonexistent"))],
        );

        let resolved = resolve_all_from_index(&index, &[])
            .expect("an index without required system packages must resolve");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "broken");
    }

    #[test]
    fn resolve_all_accepts_empty_cache_list() {
        let index = make_index(vec![], vec![make_package("orphan", None)]);

        let resolved = resolve_all_from_index(&index, &[])
            .expect("an index without required system packages must resolve");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "orphan");
    }

    #[test]
    fn resolve_all_rejects_system_packages_missing_from_index() {
        let index = make_index(vec![], vec![make_package("core", None)]);

        let error =
            resolve_all_from_index(&index, &["nix".into(), "core".into(), "bmc-nix-cli".into()])
                .expect_err("required system packages missing from the index must fail");

        assert!(matches!(
            error,
            ResolveAllFromIndexError::MissingSystemPackages { names }
                if names == ["bmc-nix-cli", "nix"]
        ));
    }

    /// Exactly the caller-listed packages may resolve as `System`: a
    /// `System` package missing from a later merged index aborts the
    /// whole upgrade, so a factory-shipped widget marked `System`
    /// would brick upgrades the moment it leaves the server index,
    /// while a required package marked `User` would let such an index
    /// silently strand a device without its runtime.
    #[test]
    fn resolve_all_only_listed_system_packages_are_system() {
        let index = make_index(
            vec![default_cache()],
            vec![
                make_package("core", None),
                make_package("nix", None),
                make_package("bmc-nix-cli", None),
                make_package("bos-avahi", None),
                make_package("widget-clock", None),
            ],
        );

        let resolved = resolve_all_from_index(&index, &["core".into(), "nix".into()])
            .expect("all required system packages are present");

        assert_eq!(resolved.len(), 5);
        for pkg in &resolved {
            let expected = match pkg.name.as_str() {
                "core" | "nix" => InstalledBy::System,
                _ => InstalledBy::User,
            };
            assert_eq!(pkg.installed_by, expected, "package {}", pkg.name);
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
            metadata: BTreeMap::new(),
        }
    }

    // ---- merge_indexes tests ----

    #[test]
    fn merge_indexes_ignores_cache_metadata() {
        let merged = merge_indexes(vec![fetched(
            "forge",
            10,
            vec![versioned_package("clock", "1.0.0", "/nix/store/clock")],
        )]);

        let pkg = &merged.packages[0];
        assert_eq!(pkg.name, "clock");
        assert_eq!(pkg.server_id, "forge");
        assert_eq!(pkg.store_path, "/nix/store/clock");
    }

    #[test]
    fn merge_indexes_skips_invalid_semver() {
        let packages = vec![
            versioned_package("good", "1.0.0", "/nix/store/good"),
            versioned_package("bad", "not-semver", "/nix/store/bad"),
        ];
        let merged = merge_indexes(vec![fetched("forge", 10, packages)]);

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
            "forge",
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

    #[test]
    fn metadata_survives_merge_and_resolve() {
        let mut entry = versioned_package("core", "1.0.0", "/nix/store/core");
        entry
            .metadata
            .insert("bmc_version".to_owned(), "2.4.0".into());
        let merged = merge_indexes(vec![fetched("forge", 10, vec![entry])]);
        let m = &merged.packages[0];
        assert_eq!(
            m.metadata
                .get("bmc_version")
                .and_then(serde_json::Value::as_str),
            Some("2.4.0")
        );

        let resolved = merged_entry_to_resolved(m, InstalledBy::System, None);
        assert_eq!(
            resolved
                .metadata
                .get("bmc_version")
                .and_then(serde_json::Value::as_str),
            Some("2.4.0")
        );
    }

    #[test]
    fn nested_widget_metadata_survives_merge_and_resolve() {
        let json = r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[
          {"name":"widget-weather","version":"1.3.0","store_path":"/nix/store/w",
           "category":"widget",
           "metadata":{"widget":{"uid":"uid-weather","display_name":"Weather","category":"info"},
                       "assets":{"icon":"/nix/store/w/lib/bmc-widgets/weather/icon.svg"}}}
        ]}"#;
        let raw: PackageIndex = serde_json::from_str(json).expect("BUG: parse");
        let merged = merge_indexes(vec![FetchedIndex {
            server_id: "srv".to_owned(),
            server_priority: 10,
            index: raw,
        }]);
        let resolved = resolve_new_package(&merged, "widget-weather", None, InstalledBy::User)
            .expect("BUG: resolve failed");
        assert_eq!(
            resolved.metadata["widget"]["uid"].as_str(),
            Some("uid-weather")
        );
        assert_eq!(
            resolved.metadata["assets"]["icon"].as_str(),
            Some("/nix/store/w/lib/bmc-widgets/weather/icon.svg")
        );
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

    #[test]
    fn resolve_new_package_unknown_name_returns_package_not_found() {
        let merged = merge_indexes(vec![fetched(
            "a",
            10,
            vec![versioned_package("clock", "1.2.0", "/nix/store/v120")],
        )]);

        let err = resolve_new_package(&merged, "widget-nope", None, InstalledBy::User)
            .expect_err("an unknown package name should error");

        assert!(matches!(err, ResolvePackageError::PackageNotFound(name) if name == "widget-nope"));
    }

    // ---- resolve_installed_package tests ----

    #[test]
    fn resolve_installed_package_prefers_same_server_when_allowed_by_pin() {
        let merged = merge_indexes(vec![
            fetched(
                "forge",
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
            installed_from: "forge".into(),
            pinned: Some("^1.0.0".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: package should resolve");

        assert_eq!(resolved.installed_from, "forge");
        assert_eq!(resolved.version, "1.2.0");
    }

    #[test]
    fn resolve_installed_package_stays_on_source_server_when_pin_excludes_it() {
        let merged = merge_indexes(vec![
            fetched(
                "a",
                1,
                vec![versioned_package("clock", "2.0.0", "/nix/store/source")],
            ),
            fetched(
                "b",
                2,
                vec![versioned_package("clock", "1.3.0", "/nix/store/other")],
            ),
        ]);
        let current = ManifestPackage {
            version: "1.2.0".into(),
            store_path: "/nix/store/current".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "a".into(),
            pinned: Some("^1.2".to_owned()),
        };

        let result = resolve_installed_package(&merged, "clock", &current);

        assert!(matches!(
            result,
            Err(ResolvePackageError::VersionNotFound { .. })
        ));
    }

    #[test]
    fn resolve_installed_package_respects_patch_pin() {
        let merged = merge_indexes(vec![fetched(
            "forge",
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
            installed_from: "forge".into(),
            pinned: Some("1.0.0".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: package should resolve at exact version");

        assert_eq!(resolved.version, "1.0.0");
    }

    #[test]
    fn resolve_installed_package_unpinned_resolves_latest() {
        let merged = merge_indexes(vec![fetched(
            "forge",
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
            installed_from: "forge".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: unpinned package should resolve");

        assert_eq!(resolved.version, "1.5.0");
    }

    #[test]
    fn resolve_installed_package_range_pin_limits_upgrade() {
        let merged = merge_indexes(vec![fetched(
            "forge",
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
            installed_from: "forge".into(),
            pinned: Some("~1.2".to_owned()),
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: tilde-pinned package should resolve within minor");

        assert_eq!(resolved.version, "1.2.5");
    }

    #[test]
    fn resolve_installed_package_refuses_downgrade_when_index_only_older() {
        let merged = merge_indexes(vec![fetched(
            "forge",
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
            installed_from: "forge".into(),
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
            "forge",
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
            installed_from: "forge".into(),
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
            "forge",
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
            installed_from: "forge".into(),
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
            "forge",
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
            installed_from: "forge".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "avahi", &current)
            .expect("BUG: a two-component current version should upgrade");

        assert_eq!(resolved.version, "0.9.0");
    }

    #[test]
    fn resolve_installed_two_component_current_resolves_same_version() {
        let merged = merge_indexes(vec![fetched(
            "forge",
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
            installed_from: "forge".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "avahi", &current)
            .expect("BUG: a same-version two-component current must not be stale");

        assert_eq!(resolved.version, "0.8.0");
    }

    #[test]
    fn origin_with_only_older_versions_stays_stale_not_migrated() {
        let merged = merge_indexes(vec![
            fetched(
                "srv-a",
                10,
                vec![versioned_package("clock", "1.9.0", "/nix/store/old")],
            ),
            fetched(
                "srv-b",
                1,
                vec![versioned_package("clock", "2.1.0", "/nix/store/new")],
            ),
        ]);
        let current = ManifestPackage {
            version: "2.0.0".into(),
            store_path: "/nix/store/current".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "srv-a".into(),
            pinned: None,
        };

        let err = resolve_installed_package(&merged, "clock", &current)
            .expect_err("BUG: must not migrate to another server");

        assert!(matches!(err, ResolvePackageError::VersionNotFound { .. }));
    }

    #[test]
    fn missing_origin_falls_back_to_other_servers_without_downgrade() {
        let merged = merge_indexes(vec![
            fetched(
                "srv-b",
                5,
                vec![
                    versioned_package("clock", "1.5.0", "/nix/store/older"),
                    versioned_package("clock", "2.1.0", "/nix/store/newer"),
                ],
            ),
            fetched(
                "srv-c",
                20,
                vec![versioned_package(
                    "clock",
                    "2.1.0",
                    "/nix/store/worse-priority",
                )],
            ),
        ]);
        let current = ManifestPackage {
            version: "2.0.0".into(),
            store_path: "/nix/store/current".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "srv-a".into(),
            pinned: None,
        };

        let resolved = resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: a package whose origin lists no entries must fall back");

        assert_eq!(resolved.version, "2.1.0");
        assert_eq!(resolved.installed_from, "srv-b");
        assert_eq!(resolved.store_path, "/nix/store/newer");
    }

    #[test]
    fn resolve_installed_two_component_current_refuses_downgrade() {
        let merged = merge_indexes(vec![fetched(
            "forge",
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
            installed_from: "forge".into(),
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

    #[tokio::test]
    async fn fetch_document_bytes_requests_exact_http_path() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock server");
        let addr = listener.local_addr().expect("BUG: no local addr");

        let body = "feed-bytes";
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
        let url = format!("http://{addr}/custom-doc-name.json");
        let (location, bytes) = fetch_document_bytes(&client, &url, MAX_INDEX_BYTES)
            .await
            .expect("BUG: fetch_document_bytes should succeed against mock server");

        let request_line = server_task.await.expect("BUG: mock server task panicked");
        let path = request_line
            .split_whitespace()
            .nth(1)
            .expect("BUG: malformed request line");
        assert_eq!(
            path, "/custom-doc-name.json",
            "fetch_document_bytes must request the given URL verbatim"
        );
        assert_eq!(location, url);
        assert_eq!(bytes, body.as_bytes());
    }

    #[tokio::test]
    async fn fetch_document_bytes_reads_file_url_verbatim() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("custom-doc-name.json");
        std::fs::write(&path, b"feed-bytes").expect("BUG: write document file");

        let client = reqwest::Client::new();
        let url = format!("file://{}", path.display());
        let (location, bytes) = fetch_document_bytes(&client, &url, MAX_INDEX_BYTES)
            .await
            .expect("BUG: fetch_document_bytes should read the file verbatim");

        assert_eq!(location, path.display().to_string());
        assert_eq!(bytes, b"feed-bytes");
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

    #[tokio::test]
    async fn fetch_index_reads_file_url_verbatim() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("my-index.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[],"caches":[],"packages":[{{"name":"clock","version":"1.0.0","store_path":"/nix/store/clock"}}]}}"#
            ),
        )
        .expect("BUG: write index file");

        let client = reqwest::Client::new();
        let reference = format!("file://{}", path.display());
        let index = fetch_index(&client, &reference)
            .await
            .expect("BUG: file:// fetch should parse");

        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].name, "clock");
    }

    #[tokio::test]
    async fn fetch_index_missing_file_returns_file_not_found() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let missing = dir.path().join("absent.json");
        let reference = format!("file://{}", missing.display());

        let client = reqwest::Client::new();
        let err = fetch_index(&client, &reference)
            .await
            .expect_err("missing file must error");

        assert!(
            matches!(err, FetchIndexesError::FileNotFound { ref path } if path == &missing.display().to_string()),
            "expected FileNotFound carrying the path, got {err:?}"
        );
    }

    #[test]
    fn check_index_size_accepts_within_cap() {
        check_index_size_with_cap("file:///idx.json", 4, 4)
            .expect("BUG: a payload at the cap must be accepted");
    }

    #[test]
    fn check_index_size_rejects_over_cap() {
        let err = check_index_size_with_cap("file:///idx.json", 5, 4)
            .expect_err("a payload over the cap must be rejected");
        assert!(
            matches!(
                err,
                FetchIndexesError::IndexTooLarge { ref location, size }
                    if location == "file:///idx.json" && size == 5
            ),
            "expected IndexTooLarge carrying location and size, got {err:?}"
        );
    }

    #[tokio::test]
    async fn fetch_index_file_over_cap_rejected() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("big.json");
        std::fs::write(&path, b"0123456789").expect("BUG: write index file");

        let client = reqwest::Client::new();
        let reference = format!("file://{}", path.display());
        let err = fetch_index_bytes_with_cap(&client, &reference, 4)
            .await
            .expect_err("a file over the cap must be rejected before reading");

        assert!(
            matches!(
                err,
                FetchIndexesError::IndexTooLarge { ref location, size }
                    if location == &path.display().to_string() && size == 10
            ),
            "expected IndexTooLarge carrying the path and size, got {err:?}"
        );
    }

    #[tokio::test]
    async fn fetch_index_file_non_regular_rejected() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let reference = format!("file://{}", dir.path().display());

        let client = reqwest::Client::new();
        let err = fetch_index(&client, &reference)
            .await
            .expect_err("a non-regular file must be rejected");

        assert!(
            matches!(
                err,
                FetchIndexesError::FileRead { ref path, .. }
                    if path == &dir.path().display().to_string()
            ),
            "expected FileRead for a non-regular file, got {err:?}"
        );
    }

    #[tokio::test]
    async fn fetch_index_file_wrong_version_rejected() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("v99.json");
        std::fs::write(
            &path,
            r#"{"version":99,"provenance":null,"indexes":[],"caches":[],"packages":[]}"#,
        )
        .expect("BUG: write index file");

        let client = reqwest::Client::new();
        let reference = format!("file://{}", path.display());
        let err = fetch_index(&client, &reference)
            .await
            .expect_err("version 99 must be rejected");

        assert!(
            matches!(
                err,
                FetchIndexesError::UnsupportedVersion { version: 99, .. }
            ),
            "expected UnsupportedVersion, got {err:?}"
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

    fn server_entry(index_url: &str) -> ServerEntry {
        required_server("primary", index_url, 10)
    }

    fn required_server(id: &str, index_url: &str, priority: u32) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            source: ServerSource::Index {
                index_url: index_url.to_owned(),
            },
            known_public_key: String::new(),
            priority,
            enabled: true,
            required: true,
        }
    }

    fn optional_server(id: &str, index_url: &str, priority: u32) -> ServerEntry {
        ServerEntry {
            required: false,
            ..required_server(id, index_url, priority)
        }
    }

    fn feed_server(id: &str, feed_url: &str, priority: u32, required: bool) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            source: ServerSource::Feed {
                feed_url: feed_url.to_owned(),
            },
            known_public_key: String::new(),
            priority,
            enabled: true,
            required,
        }
    }

    /// Spawn a local HTTP listener that serves `body` as a `200 OK` JSON
    /// response to every connection, returning its `http://` base URL and
    /// the server task handle (the caller must `abort()` it when done).
    async fn spawn_mock_index_server(body: String) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock server");
        let addr = listener.local_addr().expect("BUG: no local addr");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        (format!("http://{addr}"), server)
    }

    /// Bind an ephemeral port and immediately drop the listener, yielding a
    /// `http://` URL that no server is listening on, so connecting to it
    /// fails fast with a real network error.
    async fn dead_http_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind ephemeral port");
        let addr = listener.local_addr().expect("BUG: no local addr");
        drop(listener);
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn federation_walk_aborts_when_total_exceeds_cap() {
        let leaf_url = dead_http_url().await;
        let middle_body = format!(
            r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[{}],"caches":[],"packages":[]}}"#,
            serde_json::to_string(&leaf_url).expect("BUG: serialize child url"),
        );
        let (middle_url, middle_server) = spawn_mock_index_server(middle_body).await;

        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let root = write_index(dir.path(), "root.json", &[middle_url], "");

        let client = reqwest::Client::new();
        let servers = vec![server_entry(&root)];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 2)
            .await
            .expect_err("a federation chain past the cap must abort");

        middle_server.abort();

        assert!(
            matches!(err, FetchIndexesError::TooManyIndexes { limit: 2 }),
            "expected TooManyIndexes {{ limit: 2 }}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn federation_walk_within_cap_merges_whole_chain() {
        let leaf_body = format!(
            r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[],"caches":[],"packages":[{{"name":"leaf","version":"1.0.0","store_path":"/nix/store/leaf"}}]}}"#
        );
        let (leaf_url, leaf_server) = spawn_mock_index_server(leaf_body).await;

        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let root = write_index(dir.path(), "root.json", &[leaf_url], "");

        let client = reqwest::Client::new();
        let servers = vec![server_entry(&root)];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 2)
            .await
            .expect("BUG: a chain at the cap must merge");

        leaf_server.abort();

        assert_eq!(merged.by_name.get("leaf").map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn failing_children_still_consume_the_walk_cap() {
        let dead0 = dead_http_url().await;
        let dead1 = dead_http_url().await;
        let dead2 = dead_http_url().await;

        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let root = write_index(dir.path(), "root.json", &[dead0, dead1, dead2], "");

        let client = reqwest::Client::new();
        let servers = vec![server_entry(&root)];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 2)
            .await
            .expect_err("dead children must still consume the cap and abort the walk");

        assert!(
            matches!(err, FetchIndexesError::TooManyIndexes { limit: 2 }),
            "expected TooManyIndexes {{ limit: 2 }}, got {err:?}"
        );
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
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 2)
            .await
            .expect_err("more enabled top-level servers than the cap must abort");

        assert!(
            matches!(err, FetchIndexesError::TooManyIndexes { limit: 2 }),
            "expected TooManyIndexes {{ limit: 2 }}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn file_scheme_child_references_are_rejected() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        // The child is a perfectly valid index; only the scheme guard can
        // keep its package out of the merge.
        let child = write_index(
            dir.path(),
            "child.json",
            &[],
            r#"{"name":"smuggled","version":"1.0.0","store_path":"/nix/store/smuggled"}"#,
        );
        let root = write_index(dir.path(), "root.json", &[child], "");

        let client = reqwest::Client::new();
        let servers = vec![server_entry(&root)];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect("BUG: a rejected child scheme must not abort the whole walk");

        assert!(
            !merged.by_name.contains_key("smuggled"),
            "a file:// child reference must be rejected, not read"
        );
    }

    #[tokio::test]
    async fn federation_canonicalizes_trailing_slash_to_fetch_child_once() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock server");
        let addr = listener.local_addr().expect("BUG: no local addr");

        let body = format!(
            r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[],"caches":[],"packages":[{{"name":"leaf","version":"1.0.0","store_path":"/nix/store/leaf"}}]}}"#
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let child = format!("http://{addr}");
        let child_slash = format!("http://{addr}/");
        let root = write_index(dir.path(), "root.json", &[child, child_slash], "");

        let client = reqwest::Client::new();
        let servers = vec![server_entry(&root)];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect("BUG: federation should merge");

        server.abort();

        assert_eq!(
            merged.by_name.get("leaf").map(Vec::len),
            Some(1),
            "a child listed with and without a trailing slash must be fetched once"
        );
    }

    #[tokio::test]
    async fn fetch_index_http_over_cap_without_content_length_rejected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock server");
        let addr = listener.local_addr().expect("BUG: no local addr");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("BUG: mock server failed to accept");
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n0123456789")
                .await
                .expect("BUG: failed to write response");
        });

        let client = reqwest::Client::new();
        let base_url = format!("http://{addr}");
        let err = fetch_index_bytes_with_cap(&client, &base_url, 4)
            .await
            .expect_err("a body over the cap with no Content-Length must be rejected");

        server.await.expect("BUG: mock server task panicked");

        assert!(
            matches!(err, FetchIndexesError::IndexTooLarge { size, .. } if size > 4),
            "expected IndexTooLarge from the streaming ceiling, got {err:?}"
        );
    }

    // ---- optional / required primary-server degradation tests ----

    #[tokio::test]
    async fn optional_server_failure_degrades() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let healthy = write_index(
            dir.path(),
            "healthy.json",
            &[],
            r#"{"name":"clock","version":"1.0.0","store_path":"/nix/store/clock"}"#,
        );
        let dead = dead_http_url().await;

        let client = reqwest::Client::new();
        let servers = vec![
            required_server("healthy", &healthy, 10),
            optional_server("dead", &dead, 20),
        ];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect("BUG: an unreachable optional server must not abort the merge");

        assert_eq!(
            merged.by_name.get("clock").map(Vec::len),
            Some(1),
            "the healthy server's package must survive"
        );
        assert_eq!(
            merged.packages.len(),
            1,
            "only the healthy server contributes packages"
        );
    }

    #[tokio::test]
    async fn required_server_failure_aborts() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let healthy = write_index(
            dir.path(),
            "healthy.json",
            &[],
            r#"{"name":"clock","version":"1.0.0","store_path":"/nix/store/clock"}"#,
        );
        let dead = dead_http_url().await;

        let client = reqwest::Client::new();
        let servers = vec![
            required_server("dead", &dead, 10),
            required_server("healthy", &healthy, 20),
        ];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect_err(
                "an unreachable required server must abort the merge \
                 even when another server is healthy",
            );

        assert!(
            matches!(err, FetchIndexesError::Fetch { .. }),
            "expected the per-server fetch error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn all_enabled_failing_is_an_error() {
        let dead0 = dead_http_url().await;
        let dead1 = dead_http_url().await;

        let client = reqwest::Client::new();
        let servers = vec![
            optional_server("dead0", &dead0, 10),
            optional_server("dead1", &dead1, 20),
        ];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect_err("every enabled server failing must error, not yield an empty index");

        assert!(
            matches!(err, FetchIndexesError::Fetch { .. }),
            "expected the first per-server fetch error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn no_enabled_servers_yields_empty_merge() {
        let disabled = ServerEntry {
            enabled: false,
            ..required_server("off", "https://off.example.com/v1", 10)
        };

        let client = reqwest::Client::new();
        let servers = vec![disabled];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect("BUG: no enabled servers must yield an empty merge, not an error");

        assert!(
            merged.packages.is_empty(),
            "a fetch set with no enabled server must merge to nothing"
        );
    }

    // ---- feed-linked server resolution tests ----

    type RouteHits = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>;

    /// A bound-but-not-yet-serving route listener, split so callers can
    /// learn the port (feed bodies embed the server's own URLs) before
    /// the route table is known.
    struct PendingRouteServer {
        listener: tokio::net::TcpListener,
        addr: std::net::SocketAddr,
    }

    async fn bind_route_server() -> PendingRouteServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: failed to bind mock server");
        let addr = listener.local_addr().expect("BUG: no local addr");
        PendingRouteServer { listener, addr }
    }

    impl PendingRouteServer {
        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        /// Serve `routes` (exact path -> body, unknown paths 404) with
        /// per-path hit counting. Abort the returned handle when done.
        fn serve(self, routes: Vec<(String, String)>) -> (RouteHits, tokio::task::JoinHandle<()>) {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let hits: RouteHits = std::sync::Arc::default();
            let task_hits = std::sync::Arc::clone(&hits);
            let listener = self.listener;

            let handle = tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let mut buf = [0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        continue;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                    *task_hits
                        .lock()
                        .expect("BUG: hits lock")
                        .entry(path.clone())
                        .or_insert(0) += 1;
                    // Connection: close — the stream is dropped after one
                    // response; letting reqwest pool it for reuse races
                    // the close and flakes with ConnectionReset.
                    let response = match routes.iter().find(|(p, _)| *p == path) {
                        Some((_, body)) => format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        ),
                        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned(),
                    };
                    let _ = stream.write_all(response.as_bytes()).await;
                }
            });

            (hits, handle)
        }
    }

    fn hit_count(hits: &RouteHits, path: &str) -> usize {
        hits.lock()
            .expect("BUG: hits lock")
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    fn feed_entry(bos_version: &str, index_url: Option<&str>) -> crate::feed::PackageFeedEntry {
        crate::feed::PackageFeedEntry {
            bos_version: bos_version.to_owned(),
            download_url: "https://example.com/init.tar.gz".to_owned(),
            profile_path: "/nix/var/nix/gcroots/profiles/bmc".to_owned(),
            index_url: index_url.map(str::to_owned),
            signature: None,
        }
    }

    fn feed_json(entries: Vec<crate::feed::PackageFeedEntry>) -> String {
        serde_json::to_string(&crate::feed::PackageFeed {
            version: 1,
            entries,
        })
        .expect("BUG: serialize feed")
    }

    fn index_json(package: &str) -> String {
        format!(
            r#"{{"version":{PACKAGE_INDEX_VERSION},"provenance":null,"indexes":[],"caches":[],"packages":[{{"name":"{package}","version":"1.0.0","store_path":"/nix/store/{package}"}}]}}"#
        )
    }

    #[tokio::test]
    async fn feed_server_resolves_firmware_index_at_exact_paths() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (hits, server) = pending.serve(vec![
            (
                "/custom-feed-name.json".to_owned(),
                feed_json(vec![feed_entry(
                    "fw1",
                    Some(&format!("{base}/custom-index-name.json")),
                )]),
            ),
            ("/custom-index-name.json".to_owned(), index_json("clock")),
        ]);

        let client = reqwest::Client::new();
        let servers = vec![feed_server(
            "feedsrv",
            &format!("{base}/custom-feed-name.json"),
            10,
            true,
        )];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], Some("fw1"), 256)
            .await
            .expect("BUG: feed-linked resolution should merge");

        server.abort();

        assert_eq!(
            merged.by_name.get("clock").map(Vec::len),
            Some(1),
            "the feed-resolved index's package must be merged"
        );
        assert_eq!(
            hit_count(&hits, "/custom-feed-name.json"),
            1,
            "the feed must be requested at exactly its configured URL"
        );
        assert_eq!(
            hit_count(&hits, "/custom-index-name.json"),
            1,
            "the index must be requested at exactly the feed entry's URL"
        );
        assert_eq!(
            hits.lock().expect("BUG: hits lock").len(),
            2,
            "no other path may be requested (no filename appending)"
        );
    }

    #[tokio::test]
    async fn required_feed_without_target_firmware_aborts() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (_hits, server) = pending.serve(vec![(
            "/feed.json".to_owned(),
            feed_json(vec![feed_entry("other-fw", None)]),
        )]);

        let client = reqwest::Client::new();
        let servers = vec![feed_server(
            "feedsrv",
            &format!("{base}/feed.json"),
            10,
            true,
        )];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], Some("fw1"), 256)
            .await
            .expect_err("a required feed without the target firmware must abort");

        server.abort();

        assert!(
            matches!(
                &err,
                FetchIndexesError::FeedResolution { server_id, firmware, source: crate::feed::FeedError::MissingEntry { .. } }
                    if server_id == "feedsrv" && firmware == "fw1"
            ),
            "expected FeedResolution/MissingEntry naming server and firmware, got {err:?}"
        );
    }

    #[tokio::test]
    async fn required_feed_entry_without_index_url_aborts() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (_hits, server) = pending.serve(vec![(
            "/feed.json".to_owned(),
            feed_json(vec![feed_entry("fw1", None)]),
        )]);

        let client = reqwest::Client::new();
        let servers = vec![feed_server(
            "feedsrv",
            &format!("{base}/feed.json"),
            10,
            true,
        )];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], Some("fw1"), 256)
            .await
            .expect_err("a feed entry without index_url must abort upgrade resolution");

        server.abort();

        assert!(
            matches!(
                &err,
                FetchIndexesError::FeedResolution {
                    source: crate::feed::FeedError::MissingIndexUrl { .. },
                    ..
                }
            ),
            "expected FeedResolution/MissingIndexUrl, got {err:?}"
        );
    }

    #[tokio::test]
    async fn optional_feed_failure_degrades() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let healthy = write_index(
            dir.path(),
            "healthy.json",
            &[],
            r#"{"name":"clock","version":"1.0.0","store_path":"/nix/store/clock"}"#,
        );
        let dead = dead_http_url().await;

        let client = reqwest::Client::new();
        let servers = vec![
            required_server("healthy", &healthy, 10),
            feed_server("flaky", &format!("{dead}/feed.json"), 20, false),
        ];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], Some("fw1"), 256)
            .await
            .expect("BUG: an unreachable optional feed must not abort the merge");

        assert_eq!(merged.by_name.get("clock").map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn all_optional_feed_failures_still_error() {
        let dead0 = dead_http_url().await;
        let dead1 = dead_http_url().await;

        let client = reqwest::Client::new();
        let servers = vec![
            feed_server("dead0", &format!("{dead0}/feed.json"), 10, false),
            feed_server("dead1", &format!("{dead1}/feed.json"), 20, false),
        ];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], Some("fw1"), 256)
            .await
            .expect_err("every enabled feed failing must error, not yield an empty index");

        assert!(
            matches!(err, FetchIndexesError::FeedFetch { .. }),
            "expected the first per-server feed fetch error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn ad_hoc_success_allows_all_optional_feed_failures_to_degrade() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let reference = write_index(
            dir.path(),
            "adhoc.json",
            &[],
            r#"{"name":"adhoc-pkg","version":"1.0.0","store_path":"/nix/store/adhoc"}"#,
        );
        let dead0 = dead_http_url().await;
        let dead1 = dead_http_url().await;

        let client = reqwest::Client::new();
        let servers = vec![
            feed_server("dead0", &format!("{dead0}/feed.json"), 10, false),
            feed_server("dead1", &format!("{dead1}/feed.json"), 20, false),
        ];
        let ad_hoc = vec![AdHocIndexRef {
            id: "custom-0".to_owned(),
            reference,
        }];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &ad_hoc, Some("fw1"), 256)
            .await
            .expect("BUG: a successful ad-hoc index satisfies source resolution");

        assert_eq!(merged.by_name.get("adhoc-pkg").map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn disabled_feed_server_is_never_fetched() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (hits, server) = pending.serve(vec![
            (
                "/feed.json".to_owned(),
                feed_json(vec![feed_entry("fw1", Some(&format!("{base}/index.json")))]),
            ),
            ("/index.json".to_owned(), index_json("clock")),
        ]);

        let disabled = ServerEntry {
            enabled: false,
            ..feed_server("off", &format!("{base}/feed.json"), 10, true)
        };

        let client = reqwest::Client::new();
        let merged = fetch_and_merge_indexes_with_cap(&client, &[disabled], &[], Some("fw1"), 256)
            .await
            .expect("BUG: a disabled server must merge to nothing, not error");

        server.abort();

        assert!(merged.packages.is_empty());
        assert_eq!(
            hit_count(&hits, "/feed.json"),
            0,
            "a disabled server's feed must never be requested"
        );
        assert_eq!(
            hit_count(&hits, "/index.json"),
            0,
            "a disabled server's index must never be requested"
        );
    }

    #[tokio::test]
    async fn feed_server_without_firmware_scope_errors() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (hits, server) = pending.serve(vec![(
            "/feed.json".to_owned(),
            feed_json(vec![feed_entry("fw1", None)]),
        )]);

        let client = reqwest::Client::new();
        let servers = vec![feed_server(
            "feedsrv",
            &format!("{base}/feed.json"),
            10,
            true,
        )];
        let err = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect_err("an enabled feed server with no firmware scope must error");

        server.abort();

        assert!(
            matches!(
                &err,
                FetchIndexesError::MissingFirmwareScope { server_id } if server_id == "feedsrv"
            ),
            "expected MissingFirmwareScope, got {err:?}"
        );
        assert_eq!(
            hit_count(&hits, "/feed.json"),
            0,
            "the scope check must fail before any fetch"
        );
    }

    #[tokio::test]
    async fn ad_hoc_http_ref_appends_well_known_filename_and_merges() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let well_known = format!("/nix-package-index.v{PACKAGE_INDEX_VERSION}.json");
        let (hits, server) = pending.serve(vec![
            (well_known.clone(), index_json("adhoc-pkg")),
            (
                "/feed.json".to_owned(),
                feed_json(vec![feed_entry("fw1", Some(&format!("{base}/index.json")))]),
            ),
            ("/index.json".to_owned(), index_json("feed-pkg")),
        ]);

        let client = reqwest::Client::new();
        let servers = vec![feed_server(
            "feedsrv",
            &format!("{base}/feed.json"),
            10,
            true,
        )];
        let ad_hoc = vec![AdHocIndexRef {
            id: "custom-0".to_owned(),
            reference: base.clone(),
        }];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &ad_hoc, Some("fw1"), 256)
            .await
            .expect("BUG: ad-hoc and configured servers should merge together");

        server.abort();

        assert_eq!(merged.by_name.get("adhoc-pkg").map(Vec::len), Some(1));
        assert_eq!(merged.by_name.get("feed-pkg").map(Vec::len), Some(1));
        assert_eq!(
            hit_count(&hits, &well_known),
            1,
            "an ad-hoc http reference keeps base-URL semantics (filename appended)"
        );
    }

    #[tokio::test]
    async fn ad_hoc_file_ref_reads_document_verbatim() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let reference = write_index(
            dir.path(),
            "adhoc.json",
            &[],
            r#"{"name":"adhoc-pkg","version":"1.0.0","store_path":"/nix/store/adhoc"}"#,
        );

        let client = reqwest::Client::new();
        let ad_hoc = vec![AdHocIndexRef {
            id: "custom-0".to_owned(),
            reference,
        }];
        let merged = fetch_and_merge_indexes_with_cap(&client, &[], &ad_hoc, None, 256)
            .await
            .expect("BUG: an ad-hoc file reference should merge");

        assert_eq!(merged.by_name.get("adhoc-pkg").map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn ad_hoc_failure_is_fatal() {
        let dead = dead_http_url().await;

        let client = reqwest::Client::new();
        let ad_hoc = vec![AdHocIndexRef {
            id: "custom-0".to_owned(),
            reference: dead,
        }];
        let err = fetch_and_merge_indexes_with_cap(&client, &[], &ad_hoc, None, 256)
            .await
            .expect_err("an unreachable ad-hoc reference must abort");

        assert!(
            matches!(err, FetchIndexesError::Fetch { .. }),
            "expected the ad-hoc fetch error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn index_server_requests_exact_custom_path() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (hits, server) = pending.serve(vec![(
            "/custom-index-name.json".to_owned(),
            index_json("clock"),
        )]);

        let client = reqwest::Client::new();
        let servers = vec![required_server(
            "direct",
            &format!("{base}/custom-index-name.json"),
            10,
        )];
        let merged = fetch_and_merge_indexes_with_cap(&client, &servers, &[], None, 256)
            .await
            .expect("BUG: a direct index server should merge");

        server.abort();

        assert_eq!(merged.by_name.get("clock").map(Vec::len), Some(1));
        assert_eq!(
            hit_count(&hits, "/custom-index-name.json"),
            1,
            "a direct index_url must be requested verbatim, not appended to"
        );
        assert_eq!(
            hits.lock().expect("BUG: hits lock").len(),
            1,
            "no other path may be requested"
        );
    }

    /// `std::io::Write` sink shared with the test so a scoped `tracing`
    /// subscriber's formatted output can be asserted on.
    #[derive(Clone, Default)]
    struct SharedLogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedLogBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("BUG: log buffer lock").extend(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogBuffer {
        type Writer = SharedLogBuffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[tokio::test]
    async fn optional_feed_degrade_warning_names_server_firmware_and_cause() {
        let pending = bind_route_server().await;
        let base = pending.base_url();
        let (_hits, server) = pending.serve(vec![(
            "/feed.json".to_owned(),
            feed_json(vec![feed_entry("other-fw", None)]),
        )]);

        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let healthy = write_index(
            dir.path(),
            "healthy.json",
            &[],
            r#"{"name":"clock","version":"1.0.0","store_path":"/nix/store/clock"}"#,
        );

        let client = reqwest::Client::new();
        let servers = vec![
            required_server("healthy", &healthy, 10),
            feed_server("flaky", &format!("{base}/feed.json"), 20, false),
        ];

        let buffer = SharedLogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);

        // Parallel tests hit the same warn callsite; a thread caught
        // mid-registration computes its interest on that thread — no
        // subscriber there — and can cache the event as disabled AFTER
        // this dispatch's registration rebuilt it (tracing-core 0.1.33
        // `Rebuilder::JustOne`). Rebuilding under our subscriber repairs
        // the cache; retry in case another registration re-clobbers it
        // mid-fetch.
        let mut merged = None;
        let mut log = String::new();
        for _ in 0..5 {
            buffer.0.lock().expect("BUG: log buffer lock").clear();
            let guard = tracing::dispatcher::set_default(&dispatch);
            tracing::callsite::rebuild_interest_cache();
            merged = Some(
                fetch_and_merge_indexes_with_cap(&client, &servers, &[], Some("fw1"), 256)
                    .await
                    .expect("BUG: the optional feed failure must degrade, not abort"),
            );
            drop(guard);
            log = String::from_utf8(buffer.0.lock().expect("BUG: log buffer lock").clone())
                .expect("BUG: utf8 log output");
            if !log.is_empty() {
                break;
            }
        }
        server.abort();

        let merged = merged.expect("BUG: at least one fetch attempt ran");
        assert_eq!(merged.by_name.get("clock").map(Vec::len), Some(1));
        assert!(log.contains("flaky"), "warning must name the server: {log}");
        assert!(
            log.contains("fw1"),
            "warning must name the target firmware: {log}"
        );
        assert!(
            log.contains("no package feed entry"),
            "warning must carry the cause: {log}"
        );
    }
}

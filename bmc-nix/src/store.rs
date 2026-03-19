// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::{debug, info};

use crate::types::{FactoryServerEntry, FactoryTarball, ResolvedPackage};

/// Errors that can occur when copying store paths.
#[derive(Debug, thiserror::Error)]
pub enum CopyStorePathsError {
    #[error("nix copy failed for cache '{cache}': {source}")]
    NixCommand {
        cache: String,
        #[source]
        source: std::io::Error,
    },
    #[error("nix copy exited with {status} for cache '{cache}': {stderr}")]
    NixCopyFailed {
        cache: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("nix path-info failed: {0}")]
    PathInfoFailed(#[source] std::io::Error),
    #[error("store path for '{name}' not found: {store_path}")]
    MissingStorePath { name: String, store_path: String },
}

/// Abstraction over command execution for testability.
///
/// Uses native async fn (RPITIT), which means this trait is NOT
/// object-safe. Use generics (`impl CommandRunner` or `<R: CommandRunner>`)
/// instead of `&dyn CommandRunner`.
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &str,
        args: &[&str],
    ) -> impl std::future::Future<Output = Result<std::process::Output, std::io::Error>> + Send;
}

/// Default implementation using `tokio::process::Command`.
#[derive(Debug)]
pub struct TokioCommandRunner;

impl CommandRunner for TokioCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<std::process::Output, std::io::Error> {
        tokio::process::Command::new(program)
            .args(args)
            .output()
            .await
    }
}

/// Progress callback for store copy operations.
pub trait CopyProgress: Send + Sync {
    fn on_path_copied(&self, index: usize, total: usize, store_path: &str);
}

/// Check if a store path is already registered in the local Nix store.
///
/// Uses `nix path-info` rather than filesystem checks — a directory
/// may exist on disk but not be registered in the Nix database.
async fn is_path_in_store(
    runner: &impl CommandRunner,
    store_path: &str,
) -> Result<bool, CopyStorePathsError> {
    let output = runner
        .run("nix", &["path-info", store_path])
        .await
        .map_err(CopyStorePathsError::PathInfoFailed)?;
    Ok(output.status.success())
}

/// Verify that all store paths are registered in the local Nix store.
///
/// Returns an error listing the first missing path. Call this after
/// `copy_store_paths` to catch kept packages whose store paths were
/// garbage-collected or otherwise lost.
pub async fn verify_store_paths(
    runner: &impl CommandRunner,
    packages: &[ResolvedPackage],
) -> Result<(), CopyStorePathsError> {
    for pkg in packages {
        if !is_path_in_store(runner, &pkg.store_path).await? {
            return Err(CopyStorePathsError::MissingStorePath {
                name: pkg.name.clone(),
                store_path: pkg.store_path.clone(),
            });
        }
    }
    Ok(())
}

/// Group packages by cache URL for batched `nix copy` invocations.
fn group_by_cache(packages: &[ResolvedPackage]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in packages {
        if let Some(url) = &pkg.cache_url {
            groups
                .entry(url.clone())
                .or_default()
                .push(pkg.store_path.clone());
        }
    }
    groups
}

/// Copy store paths from binary caches to the local Nix store.
///
/// Groups packages by `cache_url` and runs one `nix copy` per cache.
/// Skips paths that are already registered in the local store.
pub async fn copy_store_paths(
    runner: &impl CommandRunner,
    packages: &[ResolvedPackage],
    progress: Option<&dyn CopyProgress>,
) -> Result<(), CopyStorePathsError> {
    let groups = group_by_cache(packages);
    let total_paths: usize = groups.values().map(Vec::len).sum();
    let mut copied: usize = 0;

    for (cache_url, store_paths) in &groups {
        // Filter out paths already in store
        let mut to_copy = Vec::new();
        for path in store_paths {
            if is_path_in_store(runner, path).await? {
                debug!(store_path = %path, "already in store, skipping");
            } else {
                to_copy.push(path.as_str());
            }
        }

        if to_copy.is_empty() {
            copied += store_paths.len();
            continue;
        }

        info!(
            cache = %cache_url,
            count = to_copy.len(),
            "copying store paths"
        );

        let mut args = vec!["copy", "--from", cache_url.as_str()];
        args.extend(to_copy.iter());

        let output =
            runner
                .run("nix", &args)
                .await
                .map_err(|source| CopyStorePathsError::NixCommand {
                    cache: cache_url.clone(),
                    source,
                })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(CopyStorePathsError::NixCopyFailed {
                cache: cache_url.clone(),
                status: output.status,
                stderr,
            });
        }

        for path in &to_copy {
            copied += 1;
            if let Some(p) = progress {
                p.on_path_copied(copied, total_paths, path);
            }
        }
    }

    Ok(())
}

/// Errors that can occur during store initialization.
#[derive(Debug, thiserror::Error)]
pub enum InitStoreError {
    #[error("failed to fetch factory index from {url}: {source}")]
    FactoryIndexFetch {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("invalid factory index JSON: {source}")]
    FactoryIndexParse {
        #[source]
        source: reqwest::Error,
    },
    #[error("no factory tarball for BOS version '{0}'")]
    MissingBosVersion(String),
    #[error("tarball download failed: {source}")]
    DownloadFailed {
        #[source]
        source: reqwest::Error,
    },
    #[error("tarball extraction failed: {0}")]
    ExtractionFailed(#[source] std::io::Error),
    #[error("tar exited with {status}: {stderr}")]
    TarFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },
}

/// Result of store initialization.
#[derive(Debug)]
pub struct InitStoreResult {
    pub profile_path: PathBuf,
}

/// Progress callback for tarball download.
pub trait DownloadProgress: Send + Sync {
    fn on_bytes_downloaded(&self, downloaded: u64, total: Option<u64>);
    fn on_phase(&self, phase: &str);
}

/// Find the factory tarball matching the given BOS version.
#[must_use]
pub fn find_tarball_for_version<'a>(
    tarballs: &'a [FactoryTarball],
    bos_version: &str,
) -> Option<&'a FactoryTarball> {
    tarballs.iter().find(|t| t.bos_version == bos_version)
}

/// Initialize the Nix store from a factory tarball.
///
/// 1. Fetch factory index from the factory server
/// 2. Find tarball matching current BOS version
/// 3. Download tarball
/// 4. Extract to root filesystem
/// 5. Return the `profile_path` from the tarball metadata
pub async fn init_store(
    client: &reqwest::Client,
    factory_server: &FactoryServerEntry,
    bos_version: &str,
    progress: Option<&dyn DownloadProgress>,
) -> Result<InitStoreResult, InitStoreError> {
    // Step 1: Fetch factory index
    if let Some(p) = progress {
        p.on_phase("fetching factory index");
    }

    let factory_index: crate::types::FactoryIndex = client
        .get(&factory_server.index_url)
        .send()
        .await
        .map_err(|source| InitStoreError::FactoryIndexFetch {
            url: factory_server.index_url.clone(),
            source,
        })?
        .json()
        .await
        .map_err(|source| InitStoreError::FactoryIndexParse { source })?;

    // Step 2: Find matching tarball
    let tarball = find_tarball_for_version(&factory_index.tarballs, bos_version)
        .ok_or_else(|| InitStoreError::MissingBosVersion(bos_version.to_owned()))?;

    // Step 3: Download tarball
    if let Some(p) = progress {
        p.on_phase("downloading tarball");
    }

    let response = client
        .get(&tarball.download_url)
        .send()
        .await
        .map_err(|source| InitStoreError::DownloadFailed { source })?;

    let tarball_bytes = response
        .bytes()
        .await
        .map_err(|source| InitStoreError::DownloadFailed { source })?;

    // Step 4: Extract to root
    if let Some(p) = progress {
        p.on_phase("extracting tarball");
    }

    let mut child = tokio::process::Command::new("tar")
        .args(["xzf", "-", "-C", "/"])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(InitStoreError::ExtractionFailed)?;

    {
        use tokio::io::AsyncWriteExt;
        let stdin = child.stdin.as_mut().expect("BUG: stdin should be piped");
        stdin
            .write_all(&tarball_bytes)
            .await
            .map_err(InitStoreError::ExtractionFailed)?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(InitStoreError::ExtractionFailed)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(InitStoreError::TarFailed {
            status: output.status,
            stderr,
        });
    }

    Ok(InitStoreResult {
        profile_path: PathBuf::from(&tarball.profile_path),
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;
    use crate::types::{InstalledBy, PinStrategy};

    /// Mock command runner that records invocations and returns configurable output.
    struct MockCommandRunner {
        /// Store paths that should be reported as "already in store".
        existing_paths: Vec<String>,
        /// Recorded command invocations (program, args).
        invocations: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    }

    impl MockCommandRunner {
        fn new(existing_paths: Vec<String>) -> Self {
            Self {
                existing_paths,
                invocations: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn invocations(&self) -> Vec<(String, Vec<String>)> {
            self.invocations
                .lock()
                .expect("BUG: mutex poisoned")
                .clone()
        }
    }

    impl CommandRunner for MockCommandRunner {
        async fn run(
            &self,
            program: &str,
            args: &[&str],
        ) -> Result<std::process::Output, std::io::Error> {
            self.invocations.lock().expect("BUG: mutex poisoned").push((
                program.to_owned(),
                args.iter().map(|s| (*s).to_owned()).collect(),
            ));

            // Simulate nix path-info: succeed if path is in existing_paths
            if program == "nix" && args.first() == Some(&"path-info") {
                let path = args.get(1).unwrap_or(&"");
                let exists = self.existing_paths.iter().any(|p| p == path);
                let code = i32::from(!exists);
                return Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(code << 8),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }

            // Simulate nix copy: always succeed
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    fn test_resolved(name: &str, store_path: &str, cache_url: Option<&str>) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: cache_url.map(String::from),
            cache_name: "local".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    #[test]
    fn group_by_cache_produces_correct_batches() {
        let packages = vec![
            test_resolved("a", "/nix/store/a", Some("https://cache-1.example.com")),
            test_resolved("b", "/nix/store/b", Some("https://cache-1.example.com")),
            test_resolved("c", "/nix/store/c", Some("https://cache-2.example.com")),
        ];
        let groups = group_by_cache(&packages);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups["https://cache-1.example.com"].len(), 2);
        assert_eq!(groups["https://cache-2.example.com"].len(), 1);
    }

    #[test]
    fn group_by_cache_skips_empty_url() {
        let packages = vec![test_resolved("a", "/nix/store/a", None)];
        let groups = group_by_cache(&packages);
        assert!(groups.is_empty());
    }

    #[tokio::test]
    async fn already_present_paths_skipped() {
        let runner = MockCommandRunner::new(vec!["/nix/store/a".into()]);
        let packages = vec![
            test_resolved("a", "/nix/store/a", Some("https://cache.example.com")),
            test_resolved("b", "/nix/store/b", Some("https://cache.example.com")),
        ];
        copy_store_paths(&runner, &packages, None)
            .await
            .expect("BUG: copy should succeed");

        let invocations = runner.invocations();
        // Should have 2 path-info checks + 1 nix copy (only for /nix/store/b)
        let copy_calls: Vec<_> = invocations
            .iter()
            .filter(|(_, args)| args.first().map(String::as_str) == Some("copy"))
            .collect();
        assert_eq!(copy_calls.len(), 1);
        let copy_args = &copy_calls[0].1;
        assert!(
            copy_args.contains(&"/nix/store/b".to_owned()),
            "should copy /nix/store/b"
        );
        assert!(
            !copy_args.contains(&"/nix/store/a".to_owned()),
            "should not copy /nix/store/a (already present)"
        );
    }

    #[tokio::test]
    async fn nix_copy_command_assembled_correctly() {
        let runner = MockCommandRunner::new(vec![]);
        let packages = vec![
            test_resolved("a", "/nix/store/aaa", Some("https://cache.example.com")),
            test_resolved("b", "/nix/store/bbb", Some("https://cache.example.com")),
        ];
        copy_store_paths(&runner, &packages, None)
            .await
            .expect("BUG: copy should succeed");

        let invocations = runner.invocations();
        let copy_calls: Vec<_> = invocations
            .iter()
            .filter(|(_, args)| args.first().map(String::as_str) == Some("copy"))
            .collect();
        assert_eq!(copy_calls.len(), 1);
        assert_eq!(copy_calls[0].0, "nix");
        assert_eq!(copy_calls[0].1[0], "copy");
        assert_eq!(copy_calls[0].1[1], "--from");
        assert_eq!(copy_calls[0].1[2], "https://cache.example.com");
    }

    #[test]
    fn find_tarball_for_version_matches() {
        let tarballs = vec![
            FactoryTarball {
                bos_version: "1.0.0".into(),
                download_url: "https://example.com/1.0.0.tar.gz".into(),
                profile_path: "/nix/var/nix/gcroots/profiles/bmc".into(),
            },
            FactoryTarball {
                bos_version: "2.0.0".into(),
                download_url: "https://example.com/2.0.0.tar.gz".into(),
                profile_path: "/nix/var/nix/gcroots/profiles/bmc".into(),
            },
        ];
        let found = find_tarball_for_version(&tarballs, "1.0.0");
        assert!(found.is_some());
        assert_eq!(
            found.expect("BUG: just checked is_some").download_url,
            "https://example.com/1.0.0.tar.gz"
        );
    }

    #[test]
    fn find_tarball_for_version_missing_returns_none() {
        let tarballs = vec![FactoryTarball {
            bos_version: "1.0.0".into(),
            download_url: "https://example.com/1.0.0.tar.gz".into(),
            profile_path: "/nix/var/nix/gcroots/profiles/bmc".into(),
        }];
        let found = find_tarball_for_version(&tarballs, "9.9.9");
        assert!(found.is_none());
    }
}

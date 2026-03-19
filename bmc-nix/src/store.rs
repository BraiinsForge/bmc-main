// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use tracing::info;

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
    fn on_path_copied(&self, index: u32, total: u32, store_path: &str);
}

/// Check which store paths are registered in the local Nix store.
///
/// Queries all paths in a single `nix path-info` invocation.
/// Returns the set of paths that are present. If `nix path-info`
/// exits non-zero (some paths missing), parses stdout for the
/// subset that was found.
async fn paths_in_store(
    runner: &impl CommandRunner,
    store_paths: &[&str],
) -> Result<HashSet<String>, CopyStorePathsError> {
    if store_paths.is_empty() {
        return Ok(HashSet::new());
    }

    let mut args: Vec<&str> = vec!["path-info"];
    args.extend(store_paths);

    let output = runner
        .run("nix", &args)
        .await
        .map_err(CopyStorePathsError::PathInfoFailed)?;

    if output.status.success() {
        return Ok(store_paths.iter().map(|s| (*s).to_owned()).collect());
    }

    // Some paths missing — parse stdout for which paths were found
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Group packages by cache URL for batched `nix copy` invocations.
fn group_by_cache(packages: &[ResolvedPackage]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in packages {
        if !pkg.cache_url.is_empty() {
            groups
                .entry(pkg.cache_url.clone())
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
    let total_paths: u32 = groups
        .values()
        .map(|v| u32::try_from(v.len()).expect("BUG: store path count exceeds u32"))
        .sum();
    let mut copied: u32 = 0;

    for (cache_url, store_paths) in &groups {
        let path_refs: Vec<&str> = store_paths.iter().map(String::as_str).collect();
        let present = paths_in_store(runner, &path_refs).await?;
        let to_copy: Vec<&str> = store_paths
            .iter()
            .filter(|p| !present.contains(p.as_str()))
            .map(String::as_str)
            .collect();

        if to_copy.is_empty() {
            copied += u32::try_from(store_paths.len()).expect("BUG: store path count exceeds u32");
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
    #[error("download stalled: no data received for {timeout_secs}s")]
    DownloadStalled { timeout_secs: u64 },
    #[error("failed to write tarball to disk: {0}")]
    WriteFailed(#[source] std::io::Error),
    #[error("tarball extraction failed: {0}")]
    ExtractionFailed(#[source] std::io::Error),
    #[error("tar exited with {status}: {stderr}")]
    TarFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },
}

/// If no bytes arrive for this duration, the download is considered stalled.
const READ_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Result of store initialization.
#[derive(Debug)]
pub struct InitStoreResult {
    pub profile_path: PathBuf,
}

/// Progress callback for tarball download.
pub trait DownloadProgress: Send + Sync {
    fn on_bytes_downloaded(&self, downloaded: usize, total: Option<usize>);
    fn on_extracting(&self);
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
/// 3. Download tarball to `download_dir` (streaming to disk)
/// 4. Extract to root filesystem
/// 5. Clean up downloaded tarball
/// 6. Return the `profile_path` from the tarball metadata
pub async fn init_store(
    client: &reqwest::Client,
    factory_server: &FactoryServerEntry,
    bos_version: &str,
    download_dir: &Path,
    progress: Option<&dyn DownloadProgress>,
) -> Result<InitStoreResult, InitStoreError> {
    use tokio::io::AsyncWriteExt;

    // Step 1: Fetch factory index
    let factory_index: crate::types::FactoryIndex = client
        .get(&factory_server.index_url)
        .send()
        .await
        .map_err(|source| InitStoreError::FactoryIndexFetch {
            url: factory_server.index_url.clone(),
            source,
        })?
        .error_for_status()
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

    // Step 3: Download tarball to disk
    let tarball_path = download_dir.join("init-tarball.tar.gz");
    std::fs::create_dir_all(download_dir).map_err(InitStoreError::WriteFailed)?;

    let mut response = client
        .get(&tarball.download_url)
        .send()
        .await
        .map_err(|source| InitStoreError::DownloadFailed { source })?
        .error_for_status()
        .map_err(|source| InitStoreError::DownloadFailed { source })?;

    let total_size = response
        .content_length()
        .and_then(|n| usize::try_from(n).ok());
    let mut downloaded: usize = 0;
    let mut file = tokio::fs::File::create(&tarball_path)
        .await
        .map_err(InitStoreError::WriteFailed)?;

    while let Some(chunk) = tokio::time::timeout(READ_IDLE_TIMEOUT, response.chunk())
        .await
        .map_err(|_| InitStoreError::DownloadStalled {
            timeout_secs: READ_IDLE_TIMEOUT.as_secs(),
        })?
        .map_err(|source| InitStoreError::DownloadFailed { source })?
    {
        file.write_all(&chunk)
            .await
            .map_err(InitStoreError::WriteFailed)?;
        downloaded += chunk.len();
        if let Some(p) = progress {
            p.on_bytes_downloaded(downloaded, total_size);
        }
    }
    file.flush().await.map_err(InitStoreError::WriteFailed)?;
    drop(file);

    // Step 4: Extract to root
    if let Some(p) = progress {
        p.on_extracting();
    }

    let output = tokio::process::Command::new("tar")
        .arg("xzf")
        .arg(&tarball_path)
        .arg("-C")
        .arg("/")
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(InitStoreError::ExtractionFailed)?;

    if !output.status.success() {
        // Wipe partially-extracted store to avoid inconsistent state on retry.
        // Best-effort — if this fails, the next attempt with wipe_store=true
        // will handle it.
        tracing::warn!("tar extraction failed, wiping partial store");
        let _ = tokio::process::Command::new("rm")
            .args(["-rf", "/nix/store", "/nix/var"])
            .status()
            .await;

        // Truncate stderr — tar can produce megabytes of output when
        // many files fail, which causes display rendering issues.
        let full_stderr = String::from_utf8_lossy(&output.stderr);
        let stderr: String = full_stderr.chars().take(500).collect();
        return Err(InitStoreError::TarFailed {
            status: output.status,
            stderr,
        });
    }

    // Step 5: Clean up tarball
    let _ = tokio::fs::remove_file(&tarball_path).await;

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

            // Simulate nix path-info: supports batch queries
            if program == "nix" && args.first() == Some(&"path-info") {
                let queried_paths = &args[1..];
                let mut stdout_lines = Vec::new();
                let mut all_found = true;
                for path in queried_paths {
                    if self.existing_paths.iter().any(|p| p == path) {
                        stdout_lines.push(*path);
                    } else {
                        all_found = false;
                    }
                }
                let code = i32::from(!all_found);
                let stdout = stdout_lines.join("\n").into_bytes();
                return Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(code << 8),
                    stdout,
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

    fn test_resolved(name: &str, store_path: &str, cache_url: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: cache_url.into(),
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
            test_resolved("a", "/nix/store/a", "https://cache-1.example.com"),
            test_resolved("b", "/nix/store/b", "https://cache-1.example.com"),
            test_resolved("c", "/nix/store/c", "https://cache-2.example.com"),
        ];
        let groups = group_by_cache(&packages);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups["https://cache-1.example.com"].len(), 2);
        assert_eq!(groups["https://cache-2.example.com"].len(), 1);
    }

    #[test]
    fn group_by_cache_skips_empty_url() {
        let packages = vec![test_resolved("a", "/nix/store/a", "")];
        let groups = group_by_cache(&packages);
        assert!(groups.is_empty());
    }

    #[tokio::test]
    async fn already_present_paths_skipped() {
        let runner = MockCommandRunner::new(vec!["/nix/store/a".into()]);
        let packages = vec![
            test_resolved("a", "/nix/store/a", "https://cache.example.com"),
            test_resolved("b", "/nix/store/b", "https://cache.example.com"),
        ];
        copy_store_paths(&runner, &packages, None)
            .await
            .expect("BUG: copy should succeed");

        let invocations = runner.invocations();
        // Should have 1 batch path-info check + 1 nix copy (only for /nix/store/b)
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
            test_resolved("a", "/nix/store/aaa", "https://cache.example.com"),
            test_resolved("b", "/nix/store/bbb", "https://cache.example.com"),
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

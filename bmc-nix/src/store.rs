// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::types::{FactoryServerEntry, FactoryTarball, ResolvedPackage};

/// Errors that can occur when verifying or realising store paths.
#[derive(Debug, thiserror::Error)]
pub enum StorePathError {
    #[error("nix-store --check-validity failed: {0}")]
    CheckValidityFailed(#[source] std::io::Error),
    #[error("store path for '{name}' not found: {store_path}")]
    MissingStorePath { name: String, store_path: String },
    #[error("nix-store --realise failed to start: {0}")]
    RealiseFailed(#[source] std::io::Error),
    #[error("nix-store --realise exited with {status}: {stderr}")]
    RealiseExited {
        status: std::process::ExitStatus,
        stderr: String,
    },
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

/// Check which store paths are registered in the local Nix store.
///
/// Queries all paths in a single `nix-store --check-validity` invocation.
/// Returns the set of paths that are present. If the batch check exits
/// non-zero, checks paths individually to identify the present subset.
async fn paths_in_store(
    runner: &impl CommandRunner,
    store_paths: &[&str],
) -> Result<HashSet<String>, StorePathError> {
    if store_paths.is_empty() {
        return Ok(HashSet::new());
    }

    let mut args: Vec<&str> = vec!["--check-validity", "--"];
    args.extend(store_paths);

    let output = runner
        .run("nix-store", &args)
        .await
        .map_err(StorePathError::CheckValidityFailed)?;

    if output.status.success() {
        return Ok(store_paths.iter().map(|s| (*s).to_owned()).collect());
    }

    let mut present = HashSet::new();

    for &store_path in store_paths {
        let output = runner
            .run("nix-store", &["--check-validity", "--", store_path])
            .await
            .map_err(StorePathError::CheckValidityFailed)?;
        if output.status.success() {
            present.insert(store_path.to_owned());
        }
    }

    Ok(present)
}

/// Verify that all store paths are registered in the local Nix store.
///
/// Returns an error listing the first missing path. Call this after
/// `copy_store_paths` to catch kept packages whose store paths were
/// garbage-collected or otherwise lost.
pub async fn verify_store_paths(
    runner: &impl CommandRunner,
    packages: &[ResolvedPackage],
) -> Result<(), StorePathError> {
    let all_paths: Vec<&str> = packages.iter().map(|p| p.store_path.as_str()).collect();
    let present = paths_in_store(runner, &all_paths).await?;

    for pkg in packages {
        if !present.contains(&pkg.store_path) {
            return Err(StorePathError::MissingStorePath {
                name: pkg.name.clone(),
                store_path: pkg.store_path.clone(),
            });
        }
    }
    Ok(())
}

/// Progress callback for store path realization.
pub trait RealizeProgress: Send + Sync {
    fn on_realization_started(&self, total_paths: usize);
    fn on_realization_finished(&self);
}

/// Realise store paths via `nix-store --realise`.
///
/// Collects unique store paths from `packages`, deduplicates and sorts
/// them, then invokes one `nix-store --realise` command. Missing paths
/// are fetched from configured Nix substituters.
///
/// Returns `Ok(())` immediately for an empty package list without
/// spawning any command.
pub async fn realize_store_paths(
    runner: &impl CommandRunner,
    packages: &[ResolvedPackage],
    progress: Option<&dyn RealizeProgress>,
) -> Result<(), StorePathError> {
    use std::collections::BTreeSet;

    let paths: BTreeSet<&str> = packages.iter().map(|p| p.store_path.as_str()).collect();
    if paths.is_empty() {
        return Ok(());
    }

    if let Some(p) = progress {
        p.on_realization_started(paths.len());
    }

    let mut args: Vec<&str> = vec!["--realise"];
    args.extend(paths.iter().copied());

    let output = runner
        .run("nix-store", &args)
        .await
        .map_err(StorePathError::RealiseFailed)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(StorePathError::RealiseExited {
            status: output.status,
            stderr,
        });
    }

    if let Some(p) = progress {
        p.on_realization_finished();
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
    #[error("invalid factory index JSON from {url}: {source}")]
    FactoryIndexParse {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported factory index version {version} from {url}")]
    UnsupportedFactoryVersion { url: String, version: u32 },
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

/// Parse and validate a factory-index response body.
///
/// Pure function — no I/O. Returns `FactoryIndexParse` on bad JSON,
/// `UnsupportedFactoryVersion` when `version != FACTORY_INDEX_VERSION`,
/// otherwise the parsed [`crate::types::FactoryIndex`].
fn parse_and_validate_factory_index(
    url: &str,
    body: &[u8],
) -> Result<crate::types::FactoryIndex, InitStoreError> {
    let parsed: crate::types::FactoryIndex =
        serde_json::from_slice(body).map_err(|source| InitStoreError::FactoryIndexParse {
            url: url.to_owned(),
            source,
        })?;
    if parsed.version != crate::index::FACTORY_INDEX_VERSION {
        return Err(InitStoreError::UnsupportedFactoryVersion {
            url: url.to_owned(),
            version: parsed.version,
        });
    }
    Ok(parsed)
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
    let factory_url = crate::index::make_factory_url(&factory_server.base_url);
    let factory_body = client
        .get(&factory_url)
        .send()
        .await
        .map_err(|source| InitStoreError::FactoryIndexFetch {
            url: factory_url.clone(),
            source,
        })?
        .error_for_status()
        .map_err(|source| InitStoreError::FactoryIndexFetch {
            url: factory_url.clone(),
            source,
        })?
        .bytes()
        .await
        .map_err(|source| InitStoreError::FactoryIndexFetch {
            url: factory_url.clone(),
            source,
        })?;
    let factory_index = parse_and_validate_factory_index(&factory_url, &factory_body)?;

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

            if program == "nix-store" && args.first() == Some(&"--check-validity") {
                let queried_paths: Vec<&str> =
                    args[1..].iter().copied().filter(|a| *a != "--").collect();
                let all_found = queried_paths
                    .iter()
                    .all(|path| self.existing_paths.iter().any(|p| p == path));
                let code = i32::from(!all_found);
                return Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(code << 8),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }

            if program == "nix-store" && args.first() == Some(&"--realise") {
                return Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(0),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }

            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(1 << 8),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    fn test_resolved(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    #[tokio::test]
    async fn verify_store_paths_all_present_succeeds() {
        let runner = MockCommandRunner::new(vec!["/nix/store/a".into(), "/nix/store/b".into()]);
        let packages = vec![
            test_resolved("a", "/nix/store/a"),
            test_resolved("b", "/nix/store/b"),
        ];
        verify_store_paths(&runner, &packages)
            .await
            .expect("BUG: all paths present, should succeed");
    }

    #[tokio::test]
    async fn verify_store_paths_missing_returns_error() {
        let runner = MockCommandRunner::new(vec!["/nix/store/a".into()]);
        let packages = vec![
            test_resolved("a", "/nix/store/a"),
            test_resolved("b", "/nix/store/b"),
        ];
        let err = verify_store_paths(&runner, &packages)
            .await
            .expect_err("BUG: /nix/store/b missing, should fail");
        assert!(
            matches!(err, StorePathError::MissingStorePath { .. }),
            "expected MissingStorePath, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn verify_store_paths_reports_actual_missing_path_after_batch_failure() {
        let cli_path = "/nix/store/aaa-bmc-nix-cli";
        let missing_path = "/nix/store/bbb-weather-widget";
        let runner = MockCommandRunner::new(vec![cli_path.into()]);
        let packages = vec![
            test_resolved("bmc-nix-cli", cli_path),
            test_resolved("weather-widget", missing_path),
        ];

        let err = verify_store_paths(&runner, &packages)
            .await
            .expect_err("BUG: weather-widget path missing, should fail");

        match err {
            StorePathError::MissingStorePath { name, store_path } => {
                assert_eq!(name, "weather-widget");
                assert_eq!(store_path, missing_path);
            }
            StorePathError::CheckValidityFailed(source) => {
                panic!("expected MissingStorePath, got CheckValidityFailed: {source}");
            }
            StorePathError::RealiseFailed(source) => {
                panic!("expected MissingStorePath, got RealiseFailed: {source}");
            }
            StorePathError::RealiseExited { status, stderr } => {
                panic!("expected MissingStorePath, got RealiseExited({status}): {stderr}");
            }
        }
    }

    #[tokio::test]
    async fn validity_check_uses_nix_store_without_nix_command_feature() {
        let runner = MockCommandRunner::new(vec!["/nix/store/a".into()]);
        let packages = vec![test_resolved("a", "/nix/store/a")];
        verify_store_paths(&runner, &packages)
            .await
            .expect("BUG: path present, should succeed");

        let invocations = runner.invocations.lock().expect("BUG: mutex poisoned");
        let (program, args) = invocations
            .first()
            .expect("BUG: expected a nix-store check-validity invocation");
        assert_eq!(program, "nix-store");
        assert_eq!(
            args,
            &vec![
                "--check-validity".to_owned(),
                "--".to_owned(),
                "/nix/store/a".to_owned()
            ],
            "store paths must follow a `--` terminator"
        );
    }

    #[tokio::test]
    async fn verify_store_paths_empty_list_succeeds() {
        let runner = MockCommandRunner::new(vec![]);
        verify_store_paths(&runner, &[])
            .await
            .expect("BUG: empty list should always succeed");
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

    #[tokio::test]
    async fn realize_store_paths_uses_single_nix_store_realise_invocation() {
        let runner = MockCommandRunner::new(vec![]);
        let packages = vec![
            test_resolved("b", "/nix/store/b"),
            test_resolved("a", "/nix/store/a"),
            test_resolved("a-dup", "/nix/store/a"),
        ];
        realize_store_paths(&runner, &packages, None)
            .await
            .expect("BUG: realise should succeed");

        let invocations = runner.invocations.lock().expect("BUG: mutex poisoned");
        assert_eq!(invocations.len(), 1, "expected exactly one invocation");
        let (program, args) = &invocations[0];
        assert_eq!(program, "nix-store");
        assert_eq!(
            args,
            &vec![
                "--realise".to_owned(),
                "/nix/store/a".to_owned(),
                "/nix/store/b".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn realize_store_paths_empty_list_does_not_spawn_nix_store() {
        let runner = MockCommandRunner::new(vec![]);
        realize_store_paths(&runner, &[], None)
            .await
            .expect("BUG: empty list should succeed");

        let invocations = runner.invocations.lock().expect("BUG: mutex poisoned");
        assert_eq!(
            invocations.len(),
            0,
            "expected no invocations for empty input"
        );
    }

    #[test]
    fn parse_factory_index_accepts_v1() {
        let body = br#"{"version": 1, "tarballs": []}"#;
        let parsed = parse_and_validate_factory_index("http://test", body)
            .expect("BUG: v1 factory index should parse");
        assert_eq!(parsed.version, 1);
        assert!(parsed.tarballs.is_empty());
    }

    #[test]
    fn parse_factory_index_rejects_unsupported_version() {
        let body = br#"{"version": 99, "tarballs": []}"#;
        let err = parse_and_validate_factory_index("http://test", body)
            .expect_err("BUG: version 99 should be rejected");
        assert!(
            matches!(
                err,
                InitStoreError::UnsupportedFactoryVersion { version: 99, .. }
            ),
            "expected UnsupportedFactoryVersion {{ version: 99 }}, got {err:?}"
        );
    }

    #[test]
    fn parse_factory_index_rejects_bad_json() {
        let body = b"not json";
        let err = parse_and_validate_factory_index("http://test", body)
            .expect_err("BUG: garbage bytes should not parse");
        assert!(
            matches!(err, InitStoreError::FactoryIndexParse { .. }),
            "expected FactoryIndexParse, got {err:?}"
        );
    }
}

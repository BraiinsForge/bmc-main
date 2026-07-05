// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod progress;

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
    #[error("nix-store --realise failed ({status}): {}", .messages.join("; "))]
    RealiseExited {
        status: std::process::ExitStatus,
        /// Human-readable error messages parsed from nix `internal-json`
        /// output (ANSI-stripped), e.g. the failing store path and the
        /// reason a substituter could not be reached. Never empty: when nix
        /// emits no error-level message a bounded raw-output snippet is used.
        messages: Vec<String>,
    },
    #[error("nix-store --realise --dry-run failed to start: {0}")]
    EstimateFailed(#[source] std::io::Error),
    #[error("nix-store --realise --dry-run failed ({status}): {}", .messages.join("; "))]
    EstimateExited {
        status: std::process::ExitStatus,
        /// Same construction as [`StorePathError::RealiseExited::messages`].
        messages: Vec<String>,
    },
    #[error("could not parse dry-run fetch summary: {line}")]
    EstimateSummaryUnparsed { line: String },
    #[error("no substituter provides: {}", .paths.join(", "))]
    UnsubstitutablePaths { paths: Vec<String> },
}

/// Upper bound on retained raw stderr from a streamed command.
///
/// Error-level diagnostics are captured separately; the raw buffer only
/// feeds a small failure-fallback snippet, so a noisy `internal-json`
/// realization must not grow it without bound.
const MAX_RETAINED_STDERR_BYTES: usize = 64 * 1024;

/// Append `line` (plus a newline) to `buf` while keeping `buf` within
/// `cap` bytes. Once the cap is reached, further lines are dropped so the
/// retained head stays bounded.
fn append_bounded_stderr(buf: &mut Vec<u8>, line: &str, cap: usize) {
    if buf.len() >= cap {
        return;
    }
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    if buf.len() > cap {
        buf.truncate(cap);
    }
}

/// Run `command` to completion, retaining at most
/// [`MAX_RETAINED_STDERR_BYTES`] of stdout and of stderr. Both pipes are
/// drained concurrently and in full, so a child that dumps a large log on
/// failure neither deadlocks on a filled pipe nor forces the parent to
/// buffer the whole stream in RAM — only the bounded head of each is kept
/// for a diagnostic snippet. This is the memory-bounded counterpart to
/// [`tokio::process::Command::output`], for the memory-constrained device.
pub(crate) async fn output_bounded(
    mut command: tokio::process::Command,
) -> Result<std::process::Output, std::io::Error> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let stdout_handle = child
        .stdout
        .take()
        .expect("BUG: stdout was piped but is missing");
    let stderr_handle = child
        .stderr
        .take()
        .expect("BUG: stderr was piped but is missing");

    let stdout_task = tokio::task::spawn(drain_bounded(stdout_handle));
    let stderr_bytes = drain_bounded(stderr_handle).await?;
    let stdout_bytes = stdout_task
        .await
        .expect("BUG: stdout drain task panicked")?;
    let status = child.wait().await?;

    Ok(std::process::Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
}

/// Drain `handle` line-by-line to EOF, retaining only the bounded head.
/// Reading every line keeps the pipe from filling while capping memory.
async fn drain_bounded(
    handle: impl tokio::io::AsyncRead + Unpin,
) -> Result<Vec<u8>, std::io::Error> {
    use tokio::io::AsyncBufReadExt as _;

    let mut reader = tokio::io::BufReader::new(handle);
    let mut retained: Vec<u8> = Vec::new();
    let mut line_buf: Vec<u8> = Vec::new();
    loop {
        line_buf.clear();
        if reader.read_until(b'\n', &mut line_buf).await? == 0 {
            break;
        }
        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
            if line_buf.last() == Some(&b'\r') {
                line_buf.pop();
            }
        }
        let line = String::from_utf8_lossy(&line_buf);
        append_bounded_stderr(&mut retained, &line, MAX_RETAINED_STDERR_BYTES);
    }
    Ok(retained)
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

    /// Run a command with stderr streamed line-by-line to a callback.
    ///
    /// Returns the full `std::process::Output` (stdout captured, stderr
    /// accumulated from the streamed lines).
    fn run_with_stderr_lines<F>(
        &self,
        program: &str,
        args: &[&str],
        on_line: F,
    ) -> impl std::future::Future<Output = Result<std::process::Output, std::io::Error>> + Send
    where
        F: FnMut(&str) + Send;
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

    async fn run_with_stderr_lines<F>(
        &self,
        program: &str,
        args: &[&str],
        mut on_line: F,
    ) -> Result<std::process::Output, std::io::Error>
    where
        F: FnMut(&str) + Send,
    {
        use tokio::io::AsyncBufReadExt as _;
        use tokio::io::AsyncReadExt as _;

        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout_handle = child
            .stdout
            .take()
            .expect("BUG: stdout was piped but is missing");
        let stderr_handle = child
            .stderr
            .take()
            .expect("BUG: stderr was piped but is missing");

        // Drain stdout concurrently so its pipe buffer can never fill while
        // the current task is blocked reading stderr line-by-line. nix-store
        // --realise writes realized store paths to stdout; for a large
        // realization the 64 KB pipe buffer can fill, causing the child to
        // block on stdout while the parent is also blocked on stderr → hang.
        let stdout_task = tokio::task::spawn(async move {
            let mut buf = Vec::new();
            tokio::io::BufReader::new(stdout_handle)
                .read_to_end(&mut buf)
                .await
                .map(|_| buf)
        });

        let mut stderr_reader = tokio::io::BufReader::new(stderr_handle);
        let mut stderr_bytes: Vec<u8> = Vec::new();
        let mut line_buf: Vec<u8> = Vec::new();

        // Read stderr as raw bytes and convert lossily: nix is free to
        // emit non-UTF-8 (e.g. file names), and one bad byte must not
        // abort the run — the error return would kill_on_drop a child
        // that may be about to succeed.
        loop {
            line_buf.clear();
            if stderr_reader.read_until(b'\n', &mut line_buf).await? == 0 {
                break;
            }
            if line_buf.last() == Some(&b'\n') {
                line_buf.pop();
                if line_buf.last() == Some(&b'\r') {
                    line_buf.pop();
                }
            }
            let line = String::from_utf8_lossy(&line_buf);
            on_line(&line);
            append_bounded_stderr(&mut stderr_bytes, &line, MAX_RETAINED_STDERR_BYTES);
        }

        let stdout = stdout_task
            .await
            .expect("BUG: stdout drain task panicked")?;
        let status = child.wait().await?;

        Ok(std::process::Output {
            status,
            stdout,
            stderr: stderr_bytes,
        })
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
    fn on_download_status(&self, snapshot: &progress::DownloadSnapshot);
}

/// Collect the unique store paths of `packages`, sorted.
fn unique_store_paths(packages: &[ResolvedPackage]) -> std::collections::BTreeSet<&str> {
    packages.iter().map(|p| p.store_path.as_str()).collect()
}

/// Failure messages for a non-zero `nix-store` exit: the collected
/// error-level diagnostics, or — when nix emitted no error-level message we
/// could parse — a bounded snippet of raw output so the failure is never
/// silent.
fn realise_failure_messages(
    diagnostics: progress::RealizeDiagnostics,
    output: &std::process::Output,
    command: &str,
) -> Vec<String> {
    let mut messages = diagnostics.into_messages();
    if messages.is_empty() {
        let raw = String::from_utf8_lossy(&output.stderr);
        let snippet: String = raw.trim().chars().take(500).collect();
        messages.push(if snippet.is_empty() {
            format!("{command} exited with {}", output.status)
        } else {
            snippet
        });
    }
    messages
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
    let paths = unique_store_paths(packages);
    if paths.is_empty() {
        return Ok(());
    }

    if let Some(p) = progress {
        p.on_realization_started(paths.len());
    }

    let mut args: Vec<&str> = vec!["--log-format", "internal-json", "--realise", "--"];
    args.extend(paths.iter().copied());

    let mut tracker = progress::DownloadStatusTracker::default();
    let mut diagnostics = progress::RealizeDiagnostics::default();
    let output = runner
        .run_with_stderr_lines("nix-store", &args, |line| {
            diagnostics.ingest_line(line);
            if let Some(snapshot) = tracker.ingest_line(line)
                && let Some(p) = progress
            {
                p.on_download_status(&snapshot);
            }
        })
        .await
        .map_err(StorePathError::RealiseFailed)?;

    if !output.status.success() {
        return Err(StorePathError::RealiseExited {
            status: output.status,
            messages: realise_failure_messages(diagnostics, &output, "nix-store --realise"),
        });
    }

    if let Some(p) = progress {
        p.on_realization_finished();
    }

    Ok(())
}

/// What a realization of the given packages would download.
///
/// Sizes are parsed from nix's dry-run summary, which prints one decimal
/// in binary units, so values carry a rounding error of up to ~0.05 of the
/// printed unit (e.g. ±~50 KiB for a MiB-scale download).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RealizeEstimate {
    /// Number of store paths that would be fetched from substituters.
    pub fetch_paths: usize,
    /// Compressed download size in bytes.
    pub download_bytes: u64,
    /// Unpacked (NAR) size in bytes.
    pub unpacked_bytes: u64,
}

/// Estimate what realising the store paths of `packages` would download,
/// via `nix-store --realise --dry-run`. Queries the configured
/// substituters but fetches nothing.
///
/// Paths already present in the store contribute nothing; a fully present
/// package set yields an all-zero estimate. A path that no substituter
/// provides fails with [`StorePathError::UnsubstitutablePaths`] — the
/// corresponding real realization would fail, so the estimate doubles as
/// a pre-flight check.
///
/// Returns an all-zero estimate immediately for an empty package list
/// without spawning any command.
pub async fn estimate_realization(
    runner: &impl CommandRunner,
    packages: &[ResolvedPackage],
) -> Result<RealizeEstimate, StorePathError> {
    let paths = unique_store_paths(packages);
    if paths.is_empty() {
        return Ok(RealizeEstimate::default());
    }

    let mut args: Vec<&str> = vec![
        "--log-format",
        "internal-json",
        "--realise",
        "--dry-run",
        "--",
    ];
    args.extend(paths.iter().copied());

    let mut diagnostics = progress::RealizeDiagnostics::default();
    let mut estimate = progress::DryRunEstimate::default();
    let output = runner
        .run_with_stderr_lines("nix-store", &args, |line| {
            diagnostics.ingest_line(line);
            estimate.ingest_line(line);
        })
        .await
        .map_err(StorePathError::EstimateFailed)?;

    if !output.status.success() {
        return Err(StorePathError::EstimateExited {
            status: output.status,
            messages: realise_failure_messages(
                diagnostics,
                &output,
                "nix-store --realise --dry-run",
            ),
        });
    }

    if let Some(line) = estimate.unparsed_summary() {
        return Err(StorePathError::EstimateSummaryUnparsed {
            line: line.to_owned(),
        });
    }

    if estimate.has_unsubstitutable() {
        return Err(StorePathError::UnsubstitutablePaths {
            paths: estimate.into_unsubstitutable(),
        });
    }

    Ok(RealizeEstimate {
        fetch_paths: estimate.fetch_path_count(),
        download_bytes: estimate.download_bytes(),
        unpacked_bytes: estimate.unpacked_bytes(),
    })
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
    #[error("initialized store already exists at {path}; pass --wipe to replace it")]
    StoreAlreadyExists { path: String },
    #[error("failed to prepare staging path {path}: {source}")]
    StagePrepareFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("tarball extraction failed: {0}")]
    ExtractionFailed(#[source] std::io::Error),
    #[error("tar exited with {status}: {stderr}")]
    TarFailed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("factory tarball did not contain a nix subtree at {path}")]
    MissingNixSubtree { path: String },
    #[error("failed to promote staged Nix store from {from} to {to}: {source}")]
    PromoteFailed {
        from: String,
        to: String,
        #[source]
        source: std::io::Error,
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

pub(crate) fn stderr_snippet(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).chars().take(500).collect()
}

pub(crate) fn path_string(path: &Path) -> String {
    path.display().to_string()
}

fn remove_path_if_exists(path: &Path) -> Result<(), std::io::Error> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn prepare_promoted_store_path(stage_dir: &Path, wipe_store: bool) -> Result<(), InitStoreError> {
    let promoted_store = stage_dir.join("nix");
    match std::fs::symlink_metadata(&promoted_store) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(InitStoreError::StagePrepareFailed {
                path: path_string(&promoted_store),
                source,
            });
        }
    }

    if !wipe_store {
        return Err(InitStoreError::StoreAlreadyExists {
            path: path_string(&promoted_store),
        });
    }

    remove_path_if_exists(&promoted_store).map_err(|source| InitStoreError::StagePrepareFailed {
        path: path_string(&promoted_store),
        source,
    })
}

async fn extract_staged(tarball_path: &Path, stage_dir: &Path) -> Result<(), InitStoreError> {
    std::fs::create_dir_all(stage_dir).map_err(|source| InitStoreError::StagePrepareFailed {
        path: path_string(stage_dir),
        source,
    })?;

    let staging = stage_dir.join("nix.tmp");
    remove_path_if_exists(&staging).map_err(|source| InitStoreError::StagePrepareFailed {
        path: path_string(&staging),
        source,
    })?;
    std::fs::create_dir(&staging).map_err(|source| InitStoreError::StagePrepareFailed {
        path: path_string(&staging),
        source,
    })?;

    let result = async {
        let output = tokio::process::Command::new("tar")
            .arg("xzf")
            .arg(tarball_path)
            .arg("-C")
            .arg(&staging)
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(InitStoreError::ExtractionFailed)?;

        if !output.status.success() {
            return Err(InitStoreError::TarFailed {
                status: output.status,
                stderr: stderr_snippet(&output.stderr),
            });
        }

        // Validate the tarball before promoting: a tarball without a
        // `nix/` subtree is rejected here, so a malformed download never
        // gets promoted. Any non-`nix` entries are ignored and removed
        // with the staging directory.
        let staged_nix = staging.join("nix");
        if !staged_nix.is_dir() {
            return Err(InitStoreError::MissingNixSubtree {
                path: path_string(&staged_nix),
            });
        }

        let promoted_nix = stage_dir.join("nix");
        std::fs::rename(&staged_nix, &promoted_nix).map_err(|source| {
            InitStoreError::PromoteFailed {
                from: path_string(&staged_nix),
                to: path_string(&promoted_nix),
                source,
            }
        })?;

        Ok(())
    }
    .await;

    if result.is_err() {
        let _ = remove_path_if_exists(&staging);
    }
    result?;

    if let Err(err) = remove_path_if_exists(&staging) {
        tracing::warn!(
            "failed to remove staging directory {}: {err}",
            staging.display()
        );
    }

    Ok(())
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
/// 4. Extract to `<stage_dir>/nix.tmp`
/// 5. Promote `<stage_dir>/nix.tmp/nix` to `<stage_dir>/nix`
/// 6. Clean up downloaded tarball
/// 7. Return the `profile_path` from the tarball metadata
pub async fn init_store(
    client: &reqwest::Client,
    factory_server: &FactoryServerEntry,
    bos_version: &str,
    download_dir: &Path,
    stage_dir: &Path,
    wipe_store: bool,
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

    // Step 4: Extract into staging and promote the store atomically
    if let Some(p) = progress {
        p.on_extracting();
    }

    let staging = stage_dir.join("nix.tmp");
    remove_path_if_exists(&staging).map_err(|source| InitStoreError::StagePrepareFailed {
        path: path_string(&staging),
        source,
    })?;
    prepare_promoted_store_path(stage_dir, wipe_store)?;
    extract_staged(&tarball_path, stage_dir).await?;

    // Step 5: Clean up tarball
    let _ = tokio::fs::remove_file(&tarball_path).await;

    Ok(InitStoreResult {
        profile_path: PathBuf::from(&tarball.profile_path),
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;

    use super::*;
    use crate::types::InstalledBy;

    /// Exit outcome the mock reports for a `--realise` invocation.
    #[derive(Clone, Copy)]
    enum RealiseOutcome {
        Success,
        Failure,
    }

    /// Mock command runner that records invocations and returns configurable output.
    struct MockCommandRunner {
        /// Store paths that should be reported as "already in store".
        existing_paths: Vec<String>,
        /// Recorded command invocations (program, args).
        invocations: std::sync::Mutex<Vec<(String, Vec<String>)>>,
        /// Lines to emit as stderr for `--realise` calls.
        realise_stderr_lines: Vec<String>,
        /// Exit outcome for `--realise` calls.
        realise_outcome: RealiseOutcome,
    }

    impl MockCommandRunner {
        fn new(existing_paths: Vec<String>) -> Self {
            Self {
                existing_paths,
                invocations: std::sync::Mutex::new(Vec::new()),
                realise_stderr_lines: Vec::new(),
                realise_outcome: RealiseOutcome::Success,
            }
        }

        fn with_realise_stderr(mut self, lines: Vec<String>) -> Self {
            self.realise_stderr_lines = lines;
            self
        }

        fn with_realise_failure(mut self, lines: Vec<String>) -> Self {
            self.realise_stderr_lines = lines;
            self.realise_outcome = RealiseOutcome::Failure;
            self
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

            if program == "nix-store" && args.contains(&"--realise") {
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

        async fn run_with_stderr_lines<F>(
            &self,
            program: &str,
            args: &[&str],
            mut on_line: F,
        ) -> Result<std::process::Output, std::io::Error>
        where
            F: FnMut(&str) + Send,
        {
            self.invocations.lock().expect("BUG: mutex poisoned").push((
                program.to_owned(),
                args.iter().map(|s| (*s).to_owned()).collect(),
            ));

            if program == "nix-store" && args.contains(&"--realise") {
                let mut stderr_bytes: Vec<u8> = Vec::new();
                for line in &self.realise_stderr_lines {
                    on_line(line);
                    stderr_bytes.extend_from_slice(line.as_bytes());
                    stderr_bytes.push(b'\n');
                }
                let code = match self.realise_outcome {
                    RealiseOutcome::Success => 0,
                    RealiseOutcome::Failure => 1,
                };
                return Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(code << 8),
                    stdout: Vec::new(),
                    stderr: stderr_bytes,
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
            pinned: None,
        }
    }

    #[tokio::test]
    async fn run_with_stderr_lines_tolerates_non_utf8_stderr() {
        let mut lines: Vec<String> = Vec::new();
        let output = TokioCommandRunner
            .run_with_stderr_lines("sh", &["-c", r"printf 'bad \377 byte\nok\n' >&2"], |line| {
                lines.push(line.to_owned());
            })
            .await
            .expect("BUG: a non-UTF-8 stderr byte must not abort the run");
        assert!(output.status.success());
        assert_eq!(
            lines,
            vec!["bad \u{FFFD} byte".to_owned(), "ok".to_owned()],
            "every stderr line must be surfaced, bad bytes replaced"
        );
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
            StorePathError::RealiseExited { status, messages } => {
                panic!("expected MissingStorePath, got RealiseExited({status}): {messages:?}");
            }
            StorePathError::EstimateFailed(source) => {
                panic!("expected MissingStorePath, got EstimateFailed: {source}");
            }
            StorePathError::EstimateExited { status, messages } => {
                panic!("expected MissingStorePath, got EstimateExited({status}): {messages:?}");
            }
            StorePathError::EstimateSummaryUnparsed { line } => {
                panic!("expected MissingStorePath, got EstimateSummaryUnparsed: {line}");
            }
            StorePathError::UnsubstitutablePaths { paths } => {
                panic!("expected MissingStorePath, got UnsubstitutablePaths: {paths:?}");
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
    fn append_bounded_stderr_caps_retained_bytes() {
        let cap = 64;
        let mut buf: Vec<u8> = Vec::new();
        for _ in 0..1000 {
            append_bounded_stderr(&mut buf, "a noisy progress line", cap);
        }
        assert!(
            buf.len() <= cap,
            "retained stderr must stay within the cap, got {}",
            buf.len()
        );
    }

    #[test]
    fn append_bounded_stderr_keeps_short_output_intact() {
        let mut buf: Vec<u8> = Vec::new();
        append_bounded_stderr(&mut buf, "line one", MAX_RETAINED_STDERR_BYTES);
        append_bounded_stderr(&mut buf, "line two", MAX_RETAINED_STDERR_BYTES);
        assert_eq!(buf, b"line one\nline two\n");
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
                "--log-format".to_owned(),
                "internal-json".to_owned(),
                "--realise".to_owned(),
                "--".to_owned(),
                "/nix/store/a".to_owned(),
                "/nix/store/b".to_owned(),
            ],
            "store paths must follow a `--` terminator"
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

    struct SnapshotCollector {
        snapshots: std::sync::Mutex<Vec<progress::DownloadSnapshot>>,
    }

    impl SnapshotCollector {
        fn new() -> Self {
            Self {
                snapshots: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn snapshots(&self) -> Vec<progress::DownloadSnapshot> {
            self.snapshots.lock().expect("BUG: mutex poisoned").clone()
        }
    }

    impl RealizeProgress for SnapshotCollector {
        fn on_realization_started(&self, _total_paths: usize) {}
        fn on_realization_finished(&self) {}
        fn on_download_status(&self, snapshot: &progress::DownloadSnapshot) {
            self.snapshots
                .lock()
                .expect("BUG: mutex poisoned")
                .push(snapshot.clone());
        }
    }

    #[tokio::test]
    async fn realize_store_paths_streams_internal_json_download_progress() {
        let stderr_lines = vec![
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache"]}"#.to_owned(),
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache/nar/a"]}"#.to_owned(),
            r#"@nix {"action":"result","id":43,"type":105,"fields":[300,1000,1,0]}"#.to_owned(),
            r#"@nix {"action":"stop","id":43}"#.to_owned(),
            r#"@nix {"action":"stop","id":42}"#.to_owned(),
        ];
        let runner = MockCommandRunner::new(vec![]).with_realise_stderr(stderr_lines);
        let packages = vec![test_resolved("a", "/nix/store/a")];
        let collector = SnapshotCollector::new();

        realize_store_paths(&runner, &packages, Some(&collector))
            .await
            .expect("BUG: realise should succeed");

        let snapshots = collector.snapshots();

        // file-transfer start → active snapshot with store_path context
        let active_snapshot = snapshots
            .iter()
            .find(|s| !s.active.is_empty())
            .expect("BUG: expected at least one active snapshot");
        assert_eq!(
            active_snapshot.active[0].store_path.as_deref(),
            Some("/nix/store/a")
        );

        // progress update → downloaded 300, total Some(1000), remaining Some(700)
        let progress_snapshot = snapshots
            .iter()
            .find(|s| s.downloaded_bytes == 300)
            .expect("BUG: expected a progress snapshot with 300 downloaded bytes");
        assert_eq!(progress_snapshot.total_bytes, Some(1000));
        assert_eq!(progress_snapshot.remaining_bytes, Some(700));

        // final (after both stops) → no active transfers, aggregate totals retained
        let final_snapshot = snapshots
            .last()
            .expect("BUG: expected at least one snapshot");
        assert!(
            final_snapshot.active.is_empty(),
            "final snapshot must have no active transfers"
        );
        assert_eq!(final_snapshot.downloaded_bytes, 300);
        assert_eq!(final_snapshot.total_bytes, Some(1000));
        assert_eq!(final_snapshot.remaining_bytes, Some(700));
    }

    #[tokio::test]
    async fn realize_store_paths_reports_readable_error_on_failure() {
        let stderr_lines = vec![
            r#"@nix {"action":"start","id":42,"level":3,"parent":0,"text":"","type":108,"fields":["/nix/store/a","https://cache.example.com"]}"#.to_owned(),
            r#"@nix {"action":"start","id":43,"level":3,"parent":42,"text":"","type":101,"fields":["https://cache.example.com/nar/a.nar.xz"]}"#.to_owned(),
            r#"@nix {"action":"msg","level":0,"msg":"error: unable to download 'https://cache.example.com/nar/a.nar.xz': Couldn't resolve host name (6)"}"#.to_owned(),
            r#"@nix {"action":"msg","level":0,"msg":"error: build of '/nix/store/a' failed"}"#.to_owned(),
        ];
        let runner = MockCommandRunner::new(vec![]).with_realise_failure(stderr_lines);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let err = realize_store_paths(&runner, &packages, None)
            .await
            .expect_err("BUG: realise failed, should error");

        let StorePathError::RealiseExited { messages, .. } = &err else {
            panic!("expected RealiseExited, got: {err:?}");
        };
        assert!(
            messages
                .iter()
                .any(|m| m.contains("cache.example.com/nar/a.nar.xz")
                    && m.contains("Couldn't resolve host name")),
            "expected the failing URL and reason among messages, got: {messages:?}"
        );

        // The user-facing Display must read the nix error, not raw JSON.
        let rendered = err.to_string();
        assert!(
            rendered.contains("Couldn't resolve host name"),
            "rendered error must contain the reason, got: {rendered}"
        );
        assert!(
            !rendered.contains("@nix") && !rendered.contains("\"action\""),
            "raw internal-json must not leak into the user-facing error: {rendered}"
        );
    }

    #[tokio::test]
    async fn realize_store_paths_failure_without_parsable_error_falls_back_to_raw() {
        let runner = MockCommandRunner::new(vec![])
            .with_realise_failure(vec!["something opaque went wrong".to_owned()]);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let err = realize_store_paths(&runner, &packages, None)
            .await
            .expect_err("BUG: realise failed, should error");

        let StorePathError::RealiseExited { messages, .. } = &err else {
            panic!("expected RealiseExited, got: {err:?}");
        };
        assert_eq!(messages, &["something opaque went wrong".to_owned()]);
    }

    #[tokio::test]
    async fn estimate_realization_parses_dry_run_summary() {
        let runner = MockCommandRunner::new(vec![]).with_realise_stderr(vec![
            r#"@nix {"action":"msg","level":3,"msg":"this path will be fetched (57.2 KiB download, 273.1 KiB unpacked):"}"#.to_owned(),
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/a"}"#.to_owned(),
        ]);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let estimate = estimate_realization(&runner, &packages)
            .await
            .expect("BUG: dry run succeeded, should estimate");

        assert_eq!(
            estimate,
            RealizeEstimate {
                fetch_paths: 1,
                download_bytes: 58573,
                unpacked_bytes: 279654,
            }
        );

        let invocations = runner.invocations.lock().expect("BUG: mutex poisoned");
        let (program, args) = invocations
            .first()
            .expect("BUG: expected a nix-store invocation");
        assert_eq!(program, "nix-store");
        assert_eq!(
            args,
            &vec![
                "--log-format".to_owned(),
                "internal-json".to_owned(),
                "--realise".to_owned(),
                "--dry-run".to_owned(),
                "--".to_owned(),
                "/nix/store/a".to_owned(),
            ],
            "store paths must follow `--dry-run` and a `--` terminator"
        );
    }

    #[tokio::test]
    async fn estimate_realization_all_present_reports_zeros() {
        let runner = MockCommandRunner::new(vec![]);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let estimate = estimate_realization(&runner, &packages)
            .await
            .expect("BUG: nothing to fetch, should estimate zeros");

        assert_eq!(estimate, RealizeEstimate::default());
    }

    #[tokio::test]
    async fn estimate_realization_empty_list_does_not_spawn_nix_store() {
        let runner = MockCommandRunner::new(vec![]);

        let estimate = estimate_realization(&runner, &[])
            .await
            .expect("BUG: empty list should always succeed");

        assert_eq!(estimate, RealizeEstimate::default());
        let invocations = runner.invocations.lock().expect("BUG: mutex poisoned");
        assert!(
            invocations.is_empty(),
            "no command must be spawned for an empty package list"
        );
    }

    #[tokio::test]
    async fn estimate_realization_unsubstitutable_path_is_an_error() {
        let runner = MockCommandRunner::new(vec![]).with_realise_stderr(vec![
            r#"@nix {"action":"msg","level":3,"msg":"don't know how to build these paths:"}"#
                .to_owned(),
            r#"@nix {"action":"msg","level":3,"msg":"  /nix/store/a"}"#.to_owned(),
        ]);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let err = estimate_realization(&runner, &packages)
            .await
            .expect_err("BUG: unsubstitutable path must fail the estimate");

        let StorePathError::UnsubstitutablePaths { paths } = &err else {
            panic!("expected UnsubstitutablePaths, got: {err:?}");
        };
        assert_eq!(paths, &["/nix/store/a".to_owned()]);
    }

    #[tokio::test]
    async fn estimate_realization_unparsed_summary_is_an_error() {
        let runner = MockCommandRunner::new(vec![]).with_realise_stderr(vec![
            r#"@nix {"action":"msg","level":3,"msg":"these 3 paths will be fetched (sizes unknown):"}"#.to_owned(),
        ]);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let err = estimate_realization(&runner, &packages)
            .await
            .expect_err("BUG: unparseable summary must not yield a zero estimate");

        assert!(
            matches!(err, StorePathError::EstimateSummaryUnparsed { .. }),
            "expected EstimateSummaryUnparsed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn estimate_realization_failure_reports_diagnostics() {
        let runner = MockCommandRunner::new(vec![]).with_realise_failure(vec![
            r#"@nix {"action":"msg","level":0,"msg":"error: cannot reach substituter"}"#.to_owned(),
        ]);
        let packages = vec![test_resolved("a", "/nix/store/a")];

        let err = estimate_realization(&runner, &packages)
            .await
            .expect_err("BUG: dry run failed, should error");

        let StorePathError::EstimateExited { messages, .. } = &err else {
            panic!("expected EstimateExited, got: {err:?}");
        };
        assert_eq!(messages, &["error: cannot reach substituter".to_owned()]);
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

    fn make_test_tarball(
        root: &Path,
        entries: &[(&str, &[u8])],
    ) -> Result<PathBuf, std::io::Error> {
        let source = root.join("archive-root");
        std::fs::create_dir_all(&source)?;
        for (relative, contents) in entries {
            let path = source.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, contents)?;
        }

        let tarball = root.join("init-tarball.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("czf")
            .arg(&tarball)
            .arg("-C")
            .arg(&source)
            .arg(".")
            .status()?;
        assert!(status.success(), "BUG: tarball fixture creation failed");
        Ok(tarball)
    }

    #[tokio::test]
    async fn extract_staged_promotes_nix_atomically_and_wipes_stale_staging() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let stage = tmp.path().join("stage");
        std::fs::create_dir_all(stage.join("nix.tmp/garbage")).expect("BUG: setup");

        let tarball =
            make_test_tarball(tmp.path(), &[("nix/store/foo", b"x"), ("etc/marker", b"y")])
                .expect("BUG: tarball fixture");

        extract_staged(&tarball, &stage)
            .await
            .expect("BUG: extraction should succeed");

        assert!(stage.join("nix/store/foo").exists());
        assert!(
            !stage.join("etc").exists() && !stage.join("nix.tmp").exists(),
            "non-nix tarball entries must never leave the staging directory"
        );
    }
}

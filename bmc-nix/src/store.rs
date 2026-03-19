// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::PathBuf;

use crate::types::{FactoryServerEntry, FactoryTarball};

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
    use super::*;

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

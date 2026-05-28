// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::PathBuf;
use std::sync::Arc;

use reqwest::Client;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::downloader::FileDownloader;
use crate::firmware::{
    DownloadEvent, FirmwareDownloadError, FirmwareIndex, FirmwareResolver, UpgradeDetail,
};

/// Progress state emitted during firmware download.
#[derive(Debug)]
pub enum DownloadState {
    Progress { downloaded_mb: f32, total_mb: f32 },
    Finished { hash: String },
    Failed(FirmwareUpgradeError),
}

/// Errors from the shared firmware upgrade pipeline (check/download/verify).
#[derive(Debug, Error, Clone, PartialEq)]
pub enum FirmwareUpgradeError {
    #[error("firmware image checksum mismatch: expected {expected}, got {actual}")]
    DownloadedImageHashMismatch { expected: String, actual: String },
    #[error("failed to verify downloaded firmware")]
    VerifyFailed,
    #[error("not enough disk space: {0}")]
    NotEnoughSpace(String),
    #[error("failed to download firmware: {0}")]
    FailedToDownload(String),
    #[error("upgrade check failed: {0}")]
    CheckFailed(#[from] FirmwareDownloadError),
}

/// Check that enough disk space is available at the given path.
///
/// Iterates through path ancestors to find the mount point, then checks
/// available space against the required size.
#[cfg(feature = "disk-check")]
pub fn check_disk_space(
    path: &std::path::Path,
    required_size: u64,
) -> Result<(), FirmwareUpgradeError> {
    use sysinfo::Disks;

    let disks = Disks::new_with_refreshed_list();
    let disk = path
        .ancestors()
        .find_map(|ancestor| {
            disks
                .list()
                .iter()
                .find(|disk| disk.mount_point() == ancestor)
        })
        .ok_or_else(|| {
            FirmwareUpgradeError::NotEnoughSpace(format!(
                "failed to determine disk for path: {}",
                path.display()
            ))
        })?;

    let available = disk.available_space();
    if available < required_size {
        return Err(FirmwareUpgradeError::NotEnoughSpace(format!(
            "required: {required_size} bytes, available: {available} bytes at {}",
            path.display()
        )));
    }

    Ok(())
}

/// Shared firmware upgrade pipeline: check, download, verify.
///
/// Sysupgrade execution is the caller's responsibility.
/// `FirmwareResolver` is wrapped in `Arc` so the spawned download task
/// can reference it without requiring `Clone` on `T`.
#[derive(Debug)]
pub struct FirmwareUpgrader<T: FirmwareIndex> {
    firmware_resolver: Arc<FirmwareResolver<T>>,
    upgrade_image_path: PathBuf,
    client: Client,
}

impl<T: FirmwareIndex> FirmwareUpgrader<T> {
    /// Create a new firmware upgrader.
    ///
    /// `upgrade_image_path` is where the downloaded firmware image will be
    /// stored. `client` is the HTTP client — callers may configure TLS
    /// settings as needed (e.g. `danger_accept_invalid_certs` for pre-NTP
    /// environments).
    #[must_use]
    pub fn new(index: T, upgrade_image_path: PathBuf, client: Client) -> Self {
        Self {
            firmware_resolver: Arc::new(FirmwareResolver::new(index)),
            upgrade_image_path,
            client,
        }
    }

    /// Returns the path where the firmware image is stored.
    #[must_use]
    pub fn upgrade_image_path(&self) -> &std::path::Path {
        &self.upgrade_image_path
    }

    /// Check if a firmware upgrade is available for the given platform and version.
    pub async fn check_for_upgrade(
        &self,
        platform: bmc_platform::BosPlatform,
        version: String,
    ) -> Result<Option<UpgradeDetail>, FirmwareDownloadError> {
        self.firmware_resolver
            .check_for_upgrade(&self.client, platform, version)
            .await
    }

    /// Download firmware and verify its hash.
    ///
    /// Emits `DownloadState` events via the returned channel. The download
    /// runs in a spawned task. When `disk-check` feature is enabled, checks
    /// available disk space before starting.
    ///
    /// On success, emits `DownloadState::Finished { hash }`.
    /// On failure (including hash mismatch), emits `DownloadState::Failed`
    /// and cleans up the partially downloaded file.
    #[must_use]
    pub fn download_firmware(
        &self,
        url: String,
        expected_hash: String,
        file_size: u64,
    ) -> UnboundedReceiver<DownloadState> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let path = self.upgrade_image_path.clone();
        let client = self.client.clone();
        let resolver = Arc::clone(&self.firmware_resolver);

        tokio::task::spawn(async move {
            tracing::info!(hash = %expected_hash, "starting firmware download");

            #[cfg(feature = "disk-check")]
            if let Err(e) = check_disk_space(&path, file_size) {
                tracing::warn!(error = %e, "insufficient disk space for upgrade");
                let _ = tx.send(DownloadState::Failed(e));
                return;
            }

            let file_downloader = match FileDownloader::init(&path).await {
                Ok(dl) => dl,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to create firmware file");
                    let _ = tx.send(DownloadState::Failed(
                        FirmwareUpgradeError::FailedToDownload(format!(
                            "unable to create file: {e}"
                        )),
                    ));
                    return;
                }
            };

            let mut event_rx = resolver.download_firmware(&client, &url, file_downloader);

            #[expect(clippy::cast_precision_loss)]
            let total_mb = file_size as f32 / 1_000_000.0;
            let mut downloaded_bytes: usize = 0;
            let mut firmware_checksum = String::new();

            while let Some(event) = event_rx.recv().await {
                match event {
                    Ok(DownloadEvent::BytesWritten(bytes)) => {
                        downloaded_bytes += bytes;

                        #[expect(clippy::cast_precision_loss)]
                        let downloaded_mb = downloaded_bytes as f32 / 1_000_000.0;

                        let _ = tx.send(DownloadState::Progress {
                            downloaded_mb,
                            total_mb,
                        });
                    }
                    Ok(DownloadEvent::Finished { checksum }) => {
                        firmware_checksum = checksum;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "firmware download failed");
                        let _ = tx.send(DownloadState::Failed(
                            FirmwareUpgradeError::FailedToDownload(err.to_string()),
                        ));
                        return;
                    }
                }
            }

            // In-memory hash comparison (digest computed during download)
            if expected_hash.to_lowercase() == firmware_checksum.to_lowercase() {
                tracing::info!(hash = %firmware_checksum, total_mb, "firmware download finished");
                let _ = tx.send(DownloadState::Finished {
                    hash: firmware_checksum,
                });
            } else {
                tracing::warn!(
                    expected = %expected_hash,
                    actual = %firmware_checksum,
                    "firmware hash mismatch"
                );
                // FileDownloader's Drop won't clean up because finish() was
                // called. Remove the file explicitly.
                let _ = tokio::fs::remove_file(&path).await;
                let _ = tx.send(DownloadState::Failed(
                    FirmwareUpgradeError::DownloadedImageHashMismatch {
                        expected: expected_hash,
                        actual: firmware_checksum,
                    },
                ));
            }
        });

        rx
    }

    /// Re-verify the downloaded firmware hash by reading from disk.
    ///
    /// This is a defensive check against write corruption — the download
    /// step already compares the in-memory digest, but this re-reads the
    /// file and computes a fresh hash.
    pub async fn verify_firmware(&self, hash: &str) -> Result<(), FirmwareUpgradeError> {
        tracing::info!(hash, "verifying firmware hash on disk");

        FileDownloader::verify_hash(&self.upgrade_image_path, hash)
            .await
            .map_err(|err| {
                tracing::warn!(hash, error = %err, "firmware verification failed");
                match err {
                    crate::downloader::DownloaderError::HashMismatch { expected, actual } => {
                        FirmwareUpgradeError::DownloadedImageHashMismatch { expected, actual }
                    }
                    crate::downloader::DownloaderError::FailedToReadFile(_) => {
                        FirmwareUpgradeError::VerifyFailed
                    }
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::{FirmwareDownloadError, FirmwareIndex, UpgradeMetadata};
    use bmc_platform::BosPlatform;
    use chrono::NaiveDate;
    use reqwest::Client;
    use std::path::PathBuf;

    #[derive(Debug)]
    struct MockIndex {
        result: Option<Vec<UpgradeMetadata>>,
    }

    #[async_trait::async_trait]
    impl FirmwareIndex for MockIndex {
        async fn get_available_releases(
            &self,
            _client: &Client,
            _platform: BosPlatform,
            _version: String,
        ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
            Ok(self.result.clone())
        }
    }

    fn test_metadata() -> UpgradeMetadata {
        UpgradeMetadata::new(
            "abc123".to_owned(),
            "25.06".to_owned(),
            NaiveDate::from_ymd_opt(2026, 3, 25).expect("BUG: invalid date"),
            "Test release".to_owned(),
            "https://example.com/firmware.bin".to_owned(),
            10_000_000,
        )
    }

    #[tokio::test]
    async fn check_for_upgrade_returns_detail_when_available() {
        let index = MockIndex {
            result: Some(vec![test_metadata()]),
        };
        let upgrader = FirmwareUpgrader::new(
            index,
            PathBuf::from("/tmp/test-firmware.bin"),
            Client::new(),
        );

        let result = upgrader
            .check_for_upgrade(BosPlatform::BraiinsBmc, "25.04".to_owned())
            .await;

        let detail = result
            .expect("BUG: check failed")
            .expect("BUG: expected Some");
        assert_eq!(detail.latest_release.version, "25.06");
    }

    #[tokio::test]
    async fn check_for_upgrade_returns_none_when_no_upgrade() {
        let index = MockIndex { result: None };
        let upgrader = FirmwareUpgrader::new(
            index,
            PathBuf::from("/tmp/test-firmware.bin"),
            Client::new(),
        );

        let result = upgrader
            .check_for_upgrade(BosPlatform::BraiinsBmc, "25.06".to_owned())
            .await;

        assert!(result.expect("BUG: check failed").is_none());
    }

    #[tokio::test]
    async fn verify_firmware_fails_on_missing_file() {
        let index = MockIndex { result: None };
        let upgrader = FirmwareUpgrader::new(
            index,
            PathBuf::from("/tmp/nonexistent-firmware-file.bin"),
            Client::new(),
        );

        let result = upgrader.verify_firmware("abc123").await;
        assert!(result.is_err());
    }
}

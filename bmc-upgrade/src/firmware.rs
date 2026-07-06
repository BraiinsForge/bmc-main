// Copyright (C) 2025  Braiins Systems s.r.o.

use chrono::NaiveDate;
use reqwest::Client;
use std::fmt::Debug;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::downloader::Downloader;

/// Maximum time the download may make no progress — no response headers
/// after the request is sent, or no further body bytes mid-stream — before
/// it is abandoned. A stalled TCP connection (alive but silent) would
/// otherwise hold the upgrade run gate open forever. The timer resets on
/// every received chunk, so a slow-but-progressing link is never penalised.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[async_trait::async_trait]
pub trait FirmwareIndex: Send + Sync + Debug + 'static {
    async fn get_available_releases(
        &self,
        client: &Client,
        platform: bmc_platform::BosPlatform,
        version: String,
    ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError>;
}

#[derive(Debug)]
pub enum DownloadEvent {
    BytesWritten(usize),
    Finished { checksum: String },
}

#[derive(Debug)]
pub struct FirmwareResolver<T: FirmwareIndex> {
    index: T,
}

impl<T> FirmwareResolver<T>
where
    T: FirmwareIndex,
{
    pub fn new(index: T) -> Self {
        Self { index }
    }

    pub fn download_firmware<U: Downloader>(
        &self,
        client: &Client,
        url: &str,
        mut downloader: U,
    ) -> UnboundedReceiver<anyhow::Result<DownloadEvent>> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let url = url.to_owned();
        let client = client.clone();
        _ = tokio::spawn(async move {
            let send = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, client.get(url).send());
            let Ok(Ok(mut response)) = send.await else {
                _ = tx.send(Err(anyhow::anyhow!("Failed to get firmware")));
                return;
            };

            loop {
                let chunk =
                    match tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, response.chunk()).await {
                        Ok(Ok(Some(chunk))) => chunk,
                        Ok(Ok(None)) => break,
                        Ok(Err(_)) | Err(_) => {
                            _ = tx.send(Err(anyhow::anyhow!("Failed to download firmware")));
                            return;
                        }
                    };

                let chunk_length = chunk.len();
                if downloader.write_chunk(chunk).await.is_err() {
                    _ = tx.send(Err(anyhow::anyhow!("Failed to download firmware")));
                    return;
                }

                _ = tx.send(Ok(DownloadEvent::BytesWritten(chunk_length)));
            }
            match downloader.finish().await {
                Ok(checksum) => _ = tx.send(Ok(DownloadEvent::Finished { checksum })),
                Err(_) => _ = tx.send(Err(anyhow::anyhow!("Failed to download firmware"))),
            }
        });

        rx
    }

    pub async fn check_for_upgrade(
        &self,
        client: &Client,
        platform: bmc_platform::BosPlatform,
        version: String,
    ) -> Result<Option<UpgradeDetail>, FirmwareDownloadError> {
        let Some(release_info) = self
            .index
            .get_available_releases(client, platform, version)
            .await?
        else {
            return Ok(None);
        };

        let Some(latest_release) = release_info.first().cloned() else {
            return Ok(None);
        };

        let previous_releases: Vec<ReleaseInfo> = release_info
            .into_iter()
            .skip(1)
            .map(|release| ReleaseInfo {
                description: release.description,
                version: release.version,
            })
            .collect();

        Ok(Some(UpgradeDetail {
            latest_release,
            previous_releases,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct UpgradeMetadata {
    pub hash: String,
    pub version: String,
    pub release_date: NaiveDate,
    pub description: String,
    pub url: String,
    pub file_size: usize,
}

impl UpgradeMetadata {
    #[must_use]
    pub fn new(
        hash: String,
        version: String,
        release_date: NaiveDate,
        description: String,
        url: String,
        file_size: usize,
    ) -> Self {
        Self {
            hash,
            version,
            release_date,
            description,
            url,
            file_size,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UpgradeDetail {
    pub latest_release: UpgradeMetadata,
    pub previous_releases: Vec<ReleaseInfo>,
}

#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub version: String,
    pub description: String,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum FirmwareDownloadError {
    #[error("failed to download index")]
    IndexDownloadFailed,
    #[error("failed to get available releases")]
    FetchUpgradeDetails,
    #[error("invalid version")]
    InvalidVersion,
    #[error("platform has no upgrade asset")]
    UnsupportedPlatform,
}

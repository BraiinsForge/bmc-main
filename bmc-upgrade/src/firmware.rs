// Copyright (C) 2025  Braiins Systems s.r.o.
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
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_mins(2);

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

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[derive(Debug)]
    struct StubIndex;

    #[async_trait::async_trait]
    impl FirmwareIndex for StubIndex {
        async fn get_available_releases(
            &self,
            _client: &Client,
            _platform: bmc_platform::BosPlatform,
            _version: String,
        ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
            Ok(None)
        }
    }

    struct NullDownloader;

    #[async_trait::async_trait]
    impl Downloader for NullDownloader {
        type Error = std::io::Error;

        async fn write_chunk(&mut self, _chunk: Bytes) -> std::io::Result<()> {
            Ok(())
        }

        async fn finish(self) -> std::io::Result<String> {
            Ok(String::new())
        }
    }

    // A TCP peer that accepts the connection but never sends a byte back,
    // modelling a stalled-but-alive link. `start_paused` lets the idle-timeout
    // timer fire in virtual time, so the guard is exercised in ~0 real seconds.
    // Deleting the `tokio::time::timeout` around the request would hang here.
    #[tokio::test(start_paused = true)]
    async fn download_aborts_when_the_server_never_responds() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: bind loopback listener");
        let addr = listener.local_addr().expect("BUG: read listener addr");
        tokio::spawn(async move {
            let _conn = listener.accept().await.expect("BUG: accept connection");
            std::future::pending::<()>().await;
        });

        let resolver = FirmwareResolver::new(StubIndex);
        let url = format!("http://{addr}/firmware.bin");
        let mut rx = resolver.download_firmware(&Client::new(), &url, NullDownloader);

        assert!(
            matches!(rx.recv().await, Some(Err(_))),
            "a silent server must abort via the idle timeout, not hang forever"
        );
    }

    // A TCP peer that sends complete response headers and a first body chunk,
    // then stalls forever with more promised via `Content-Length`. The request
    // `send()` succeeds, so this exercises the *per-chunk* `response.chunk()`
    // idle timeout independently of the header timeout above — deleting that
    // second `tokio::time::timeout` would hang here.
    #[tokio::test(start_paused = true)]
    async fn download_aborts_when_the_body_stalls_mid_stream() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("BUG: bind loopback listener");
        let addr = listener.local_addr().expect("BUG: read listener addr");
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.expect("BUG: accept connection");
            let mut buf = [0_u8; 1024];
            let _ = conn.read(&mut buf).await;
            let _ = conn
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\ndata")
                .await;
            std::future::pending::<()>().await;
        });

        let resolver = FirmwareResolver::new(StubIndex);
        let url = format!("http://{addr}/firmware.bin");
        let mut rx = resolver.download_firmware(&Client::new(), &url, NullDownloader);

        // Progress events may arrive for the first chunk; the download must then
        // abort once the body stalls, never close cleanly nor hang.
        loop {
            match rx.recv().await {
                Some(Ok(_)) => continue,
                Some(Err(_)) => break,
                None => panic!("a stalled body must abort via the chunk idle timeout"),
            }
        }
    }
}

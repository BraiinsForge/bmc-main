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

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use headers::HeaderMapExt as _;
use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, RANGE,
    TRANSFER_ENCODING,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{DownloadProgress, HTTP_IDLE_TIMEOUT, InitStoreError, check_feed_size};

/// Retry budget sized for boser's 30-minute sysupgrade deadline.
/// A signed download retries at most `MAX_SIGNED_RETRIES` times.
/// An attempt ends only when its stream fails or sits idle for `HTTP_IDLE_TIMEOUT`;
/// one that keeps receiving bytes is waited out, however slow.
/// A streak of failures without progress sleeps `RETRY_DELAYS` in turn
/// and gives up after the fourth,
/// so stalls alone cost about 17 minutes of idle timeouts plus a minute of sleeps.
/// `Retry-After` is ignored, and every connect error short of a builder
/// or redirect failure (TLS and DNS included) counts as transient.
const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];
const MAX_SIGNED_RETRIES: usize = 16;
/// A server that keeps failing ranged requests may still serve plain ones.
/// Transport errors keep the range: they say nothing about the server,
/// and a network blip must not cost the partial.
const RANGED_ERRORS_BEFORE_PLAIN: usize = 2;
/// Far above a real sidecar (a signature and two headers), so the cap only
/// bounds how much of a garbage file is read before it is judged corrupt.
const MAX_METADATA_BYTES: u64 = 4_096;

/// Which signed entity the retained partial belongs to.
/// `etag` and `total` are known only once a server has answered.
#[derive(Serialize, Deserialize)]
struct DownloadMetadata {
    signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total: Option<u64>,
}

impl DownloadMetadata {
    fn new(signature: &str) -> Self {
        Self {
            signature: signature.to_owned(),
            etag: None,
            total: None,
        }
    }
}

/// The download directory holds three fixed names: the verified final tarball,
/// the `.part` file being transferred and the `.metadata.json` sidecar
/// recording which signed entity that partial belongs to.
struct DownloadPaths {
    dir: PathBuf,
    final_file: PathBuf,
    part: PathBuf,
    metadata: PathBuf,
}

impl DownloadPaths {
    fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
            final_file: dir.join("init-tarball.tar.gz"),
            part: dir.join("init-tarball.tar.gz.part"),
            metadata: dir.join("init-tarball.tar.gz.metadata.json"),
        }
    }

    /// Anything but a file at a fixed name would fail every later run,
    /// so it is removed instead of waiting for someone to clean it up.
    fn remove_non_files(&self) -> Result<(), InitStoreError> {
        for path in [&self.final_file, &self.part, &self.metadata] {
            match std::fs::metadata(path) {
                Ok(metadata) if !metadata.is_file() => {
                    tracing::warn!(path = %path.display(), "removing non-file init download artifact");
                    remove_artifact(path)?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(InitStoreError::DownloadFileIo(error)),
            }
        }
        Ok(())
    }

    fn part_len(&self) -> Result<u64, InitStoreError> {
        match std::fs::metadata(&self.part) {
            Ok(metadata) => Ok(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(InitStoreError::DownloadFileIo(error)),
        }
    }

    fn read_metadata(&self) -> Result<Option<DownloadMetadata>, InitStoreError> {
        let file = match File::open(&self.metadata) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(InitStoreError::DownloadFileIo(error)),
        };
        let mut bytes = Vec::new();
        file.take(MAX_METADATA_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(InitStoreError::DownloadFileIo)?;
        match serde_json::from_slice(&bytes) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) => {
                tracing::warn!(%error, "discarding unreadable init download metadata");
                Ok(None)
            }
        }
    }

    fn publish_metadata(&self, metadata: &DownloadMetadata) -> Result<(), InitStoreError> {
        remove_artifact(&self.metadata)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.metadata)
            .map_err(InitStoreError::DownloadFileIo)?;
        file.write_all(&serde_json::to_vec(metadata).expect("BUG: serialize download metadata"))
            .map_err(InitStoreError::DownloadFileIo)?;
        file.sync_all().map_err(InitStoreError::DownloadFileIo)?;
        drop(file);
        crate::fs_sync::fsync_dir(&self.dir).map_err(InitStoreError::DownloadFileIo)
    }

    /// Keep the retained partial only when its sidecar names `signature`;
    /// otherwise drop both, so no partial exists without a matching sidecar.
    fn prepare_signed(&self, signature: &str) -> Result<DownloadMetadata, InitStoreError> {
        match self.read_metadata()? {
            Some(metadata) if metadata.signature == signature => Ok(metadata),
            Some(_) | None => {
                remove_artifact(&self.part)?;
                remove_artifact(&self.metadata)?;
                Ok(DownloadMetadata::new(signature))
            }
        }
    }

    fn open_part(
        &self,
        disposition: ResponseDisposition,
    ) -> Result<tokio::fs::File, InitStoreError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(matches!(disposition, ResponseDisposition::Resume { .. }))
            .truncate(matches!(disposition, ResponseDisposition::Restart { .. }))
            .open(&self.part)
            .map_err(InitStoreError::WriteFailed)?;
        Ok(tokio::fs::File::from_std(file))
    }

    fn clear_all(&self) -> Result<(), InitStoreError> {
        for path in [&self.final_file, &self.part, &self.metadata] {
            remove_artifact(path)?;
        }
        Ok(())
    }
}

/// A symlink is unlinked, never followed; a directory goes with its contents.
fn remove_artifact(path: &Path) -> Result<(), InitStoreError> {
    let removed = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) => Err(error),
    };
    match removed {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(InitStoreError::DownloadFileIo(error)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    Made,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CleanRetry {
    Available,
    Spent,
}

#[derive(Default)]
struct FailureBudget {
    failures: usize,
    consecutive_no_progress_failures: usize,
}

impl FailureBudget {
    fn observe_progress(&mut self) {
        self.consecutive_no_progress_failures = 0;
    }

    async fn retry(&mut self, error: InitStoreError) -> Result<(), InitStoreError> {
        self.failures += 1;
        let Some(delay) = RETRY_DELAYS.get(self.failures - 1) else {
            return Err(error);
        };
        tracing::warn!(failure = self.failures, ?delay, %error, "retrying init download");
        tokio::time::sleep(*delay).await;
        Ok(())
    }

    fn signed_retry_delay(&mut self, progress: Progress) -> Option<Duration> {
        self.failures += 1;
        let delay_index = match progress {
            Progress::Made => {
                self.observe_progress();
                0
            }
            Progress::None => {
                self.consecutive_no_progress_failures += 1;
                self.consecutive_no_progress_failures - 1
            }
        };
        if self.failures > MAX_SIGNED_RETRIES {
            return None;
        }
        RETRY_DELAYS.get(delay_index).copied()
    }

    async fn retry_signed(
        &mut self,
        error: InitStoreError,
        progress: Progress,
    ) -> Result<(), InitStoreError> {
        let Some(delay) = self.signed_retry_delay(progress) else {
            return Err(error);
        };
        tracing::warn!(failure = self.failures, ?delay, ?progress, %error, "retrying init download");
        tokio::time::sleep(delay).await;
        Ok(())
    }
}

enum AttemptError {
    Transient(InitStoreError),
    Fatal(InitStoreError),
}

fn retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

/// A definitive answer to a ranged request. 501 is the one server error that
/// names the request rather than the server, so retrying it ranged is pointless.
fn rejects_range(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::NOT_IMPLEMENTED || !retryable_status(status)
}

async fn send(
    request: reqwest::RequestBuilder,
    error: impl Fn(reqwest::Error) -> InitStoreError,
) -> Result<reqwest::Response, AttemptError> {
    let response = tokio::time::timeout(HTTP_IDLE_TIMEOUT, request.send())
        .await
        .map_err(|_| {
            AttemptError::Transient(InitStoreError::DownloadStalled {
                timeout_secs: HTTP_IDLE_TIMEOUT.as_secs(),
            })
        })?;
    response.map_err(|source| {
        let fatal = source.is_builder() || source.is_redirect();
        if fatal {
            AttemptError::Fatal(error(source))
        } else {
            AttemptError::Transient(error(source))
        }
    })
}

pub(super) async fn fetch_feed(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, InitStoreError> {
    let mut budget = FailureBudget::default();
    loop {
        match fetch_feed_once(client, url).await {
            Ok(bytes) => return Ok(bytes),
            Err(AttemptError::Fatal(error)) => return Err(error),
            Err(AttemptError::Transient(error)) => budget.retry(error).await?,
        }
    }
}

async fn fetch_feed_once(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, AttemptError> {
    let response = send(client.get(url), |source| InitStoreError::PackageFeedFetch {
        url: url.to_owned(),
        source,
    })
    .await?;
    let status = response.status();
    let mut response = response.error_for_status().map_err(|source| {
        let error = InitStoreError::PackageFeedFetch {
            url: url.to_owned(),
            source,
        };
        if retryable_status(status) {
            AttemptError::Transient(error)
        } else {
            AttemptError::Fatal(error)
        }
    })?;
    let declared = response.content_length();
    if let Some(len) = declared {
        check_feed_size(url, len).map_err(AttemptError::Fatal)?;
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::time::timeout(HTTP_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| {
                AttemptError::Transient(InitStoreError::DownloadStalled {
                    timeout_secs: HTTP_IDLE_TIMEOUT.as_secs(),
                })
            })?
            .map_err(|source| {
                AttemptError::Transient(InitStoreError::PackageFeedFetch {
                    url: url.to_owned(),
                    source,
                })
            })?;
        let Some(chunk) = chunk else { break };
        check_feed_size(url, (bytes.len() + chunk.len()) as u64).map_err(AttemptError::Fatal)?;
        bytes.extend_from_slice(&chunk);
    }
    if declared.is_some_and(|len| bytes.len() as u64 != len) {
        return Err(AttemptError::Transient(InitStoreError::DownloadProtocol(
            "short package-feed response".to_owned(),
        )));
    }
    Ok(bytes)
}

async fn verify(path: &Path, key: &str, signature: &str) -> Result<(), InitStoreError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(InitStoreError::DownloadFileIo)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(InitStoreError::DownloadFileIo)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let hash: [u8; 32] = digest
        .finish()
        .as_ref()
        .try_into()
        .expect("BUG: SHA-256 digest is 32 bytes");
    crate::signature::verify(key, &hash, signature)
        .map_err(|source| InitStoreError::SignatureVerificationFailed { source })
}

fn content_length(response: &reqwest::Response) -> Result<Option<u64>, InitStoreError> {
    if !response.headers().contains_key(CONTENT_LENGTH) {
        return Ok(None);
    }
    response
        .content_length()
        .map(Some)
        .ok_or_else(|| InitStoreError::DownloadProtocol("invalid Content-Length".to_owned()))
}

fn identity_encoding(response: &reqwest::Response) -> bool {
    let mut values = response.headers().get_all(CONTENT_ENCODING).iter();
    match (values.next(), values.next()) {
        (None, _) => true,
        (Some(value), None) => value.as_bytes().eq_ignore_ascii_case(b"identity"),
        _ => false,
    }
}

fn framed(response: &reqwest::Response) -> bool {
    !matches!(
        response.version(),
        reqwest::Version::HTTP_09 | reqwest::Version::HTTP_10 | reqwest::Version::HTTP_11
    ) || response.headers().contains_key(CONTENT_LENGTH)
        || response.headers().contains_key(TRANSFER_ENCODING)
}

/// A strong entity tag the server can be asked to honour in `If-Range`;
/// RFC 7233 forbids weak tags there.
fn strong_etag(response: &reqwest::Response) -> Option<String> {
    let value = response.headers().get(ETAG)?.to_str().ok()?;
    is_strong_etag(value).then(|| value.to_owned())
}

fn is_strong_etag(value: &str) -> bool {
    value.parse::<headers::ETag>().is_ok() && !value.starts_with("W/")
}

fn range_segment(response: &reqwest::Response, offset: u64) -> Option<(u64, u64)> {
    if response.headers().get_all(CONTENT_RANGE).iter().count() != 1 {
        return None;
    }
    let range = response.headers().typed_get::<headers::ContentRange>()?;
    let ((start, end), total) = (range.bytes_range()?, range.bytes_len()?);
    if start != offset || end < start || end >= total {
        return None;
    }
    let len = end - start + 1;
    if content_length(response)
        .ok()?
        .is_some_and(|declared| declared != len)
    {
        return None;
    }
    Some((len, total))
}

#[derive(Clone, Copy)]
enum ResponseDisposition {
    Resume { length: u64, total: u64 },
    Restart { length: Option<u64> },
}

impl ResponseDisposition {
    fn start(self, requested_offset: u64) -> u64 {
        match self {
            Self::Resume { .. } => requested_offset,
            Self::Restart { .. } => 0,
        }
    }
}

/// A 206 the retained partial can continue from, or None when the server
/// answered a different range or a different entity than the sidecar records.
fn resume_disposition(
    response: &reqwest::Response,
    offset: u64,
    metadata: &DownloadMetadata,
) -> Option<ResponseDisposition> {
    if !identity_encoding(response) {
        return None;
    }
    let (length, total) = range_segment(response, offset)?;
    if metadata.total.is_some_and(|recorded| recorded != total) {
        return None;
    }
    if let (Some(recorded), Some(etag)) = (&metadata.etag, response.headers().get(ETAG))
        && etag.as_bytes() != recorded.as_bytes()
    {
        return None;
    }
    Some(ResponseDisposition::Resume { length, total })
}

/// A chunked 200 leaves the total unknown until a range response names it.
fn record_total(
    paths: &DownloadPaths,
    metadata: &mut DownloadMetadata,
    disposition: ResponseDisposition,
) -> Result<(), InitStoreError> {
    if let ResponseDisposition::Resume { total, .. } = disposition
        && metadata.total.is_none()
    {
        metadata.total = Some(total);
        paths.publish_metadata(metadata)?;
    }
    Ok(())
}

/// The declared length of a 200 that can be streamed to disk from zero,
/// None when chunked; anything else names why not.
fn restart_length(response: &reqwest::Response) -> Result<Option<u64>, InitStoreError> {
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        return Err(InitStoreError::DownloadProtocol(format!(
            "unexpected tarball response status {status}"
        )));
    }
    if !identity_encoding(response) {
        return Err(InitStoreError::DownloadProtocol(
            "tarball response is content-encoded".to_owned(),
        ));
    }
    if !framed(response) {
        return Err(InitStoreError::DownloadProtocol(
            "tarball response has neither Content-Length nor Transfer-Encoding".to_owned(),
        ));
    }
    content_length(response)
}

async fn stream_part(
    paths: &DownloadPaths,
    mut response: reqwest::Response,
    disposition: ResponseDisposition,
    requested_offset: u64,
    progress: Option<&dyn DownloadProgress>,
) -> Result<(), AttemptError> {
    let offset = disposition.start(requested_offset);
    let (expected, total) = match disposition {
        ResponseDisposition::Resume { length, total } => (Some(length), Some(total)),
        ResponseDisposition::Restart { length } => (length, length),
    };
    let mut file = paths.open_part(disposition).map_err(AttemptError::Fatal)?;
    let mut received = 0_u64;
    let body_result = loop {
        let chunk = match tokio::time::timeout(HTTP_IDLE_TIMEOUT, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break Ok(()),
            Ok(Err(source)) => {
                break Err(AttemptError::Transient(InitStoreError::DownloadFailed {
                    source,
                }));
            }
            Err(_) => {
                break Err(AttemptError::Transient(InitStoreError::DownloadStalled {
                    timeout_secs: HTTP_IDLE_TIMEOUT.as_secs(),
                }));
            }
        };
        file.write_all(&chunk)
            .await
            .map_err(|error| AttemptError::Fatal(InitStoreError::WriteFailed(error)))?;
        received += chunk.len() as u64;
        if let Some(progress) = progress
            && let Some(downloaded) = offset
                .checked_add(received)
                .and_then(|n| usize::try_from(n).ok())
        {
            progress.on_bytes_downloaded(downloaded, total.and_then(|n| usize::try_from(n).ok()));
        }
    };
    file.flush()
        .await
        .map_err(|error| AttemptError::Fatal(InitStoreError::WriteFailed(error)))?;
    body_result?;
    if expected.is_some_and(|len| received < len) {
        return Err(AttemptError::Transient(InitStoreError::DownloadProtocol(
            "short tarball response".to_owned(),
        )));
    }
    if let Some(len) = expected
        && received > len
    {
        file.set_len(offset + len)
            .await
            .map_err(|error| AttemptError::Fatal(InitStoreError::WriteFailed(error)))?;
        return Err(AttemptError::Fatal(InitStoreError::DownloadProtocol(
            "tarball response exceeded declared length".to_owned(),
        )));
    }
    Ok(())
}

pub(super) async fn download(
    client: &reqwest::Client,
    url: &str,
    dir: &Path,
    signature: Option<(&str, &str)>,
    progress: Option<&dyn DownloadProgress>,
) -> Result<PathBuf, InitStoreError> {
    let paths = DownloadPaths::new(dir);
    paths.remove_non_files()?;
    if let Some((key, signature)) = signature {
        download_signed(client, url, &paths, key, signature, progress).await
    } else {
        paths.clear_all()?;
        let result = download_transfer(client, url, &paths, progress).await;
        if result.is_err()
            && let Err(error) = paths.clear_all()
        {
            tracing::warn!(%error, "failed to clean unsigned download");
        }
        result.map(|()| paths.part.clone())
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "every branch of the retry loop decides what happens to the same partial"
)]
async fn download_signed(
    client: &reqwest::Client,
    url: &str,
    paths: &DownloadPaths,
    key: &str,
    signature: &str,
    progress: Option<&dyn DownloadProgress>,
) -> Result<PathBuf, InitStoreError> {
    let mut clean_retry = CleanRetry::Available;
    if let Some(final_file) = reuse_final(paths, key, signature).await? {
        return Ok(final_file);
    }
    let mut metadata = paths.prepare_signed(signature)?;
    let retained = paths.part_len()?;
    if retained > 0
        && let (Some(progress), Ok(downloaded)) = (progress, usize::try_from(retained))
    {
        let total = metadata.total.and_then(|n| usize::try_from(n).ok());
        progress.on_bytes_downloaded(downloaded, total);
    }
    let mut budget = FailureBudget::default();
    // A refused resume keeps the partial until a plain 200 replaces it,
    // so a server that fails the plain request too leaves it for the next run.
    let mut resume_refused = false;
    loop {
        let offset = if resume_refused { 0 } else { paths.part_len()? };
        let ranged = offset > 0;
        let request = tarball_request(client, url, offset, metadata.etag.as_deref());
        let response = match send(request, |source| InitStoreError::DownloadFailed { source }).await
        {
            Ok(response) => response,
            Err(AttemptError::Fatal(error)) => return Err(error),
            Err(AttemptError::Transient(error)) => {
                budget.retry_signed(error, Progress::None).await?;
                continue;
            }
        };
        let status = response.status();
        if ranged && status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            match verify_candidate(paths, key, signature, &mut clean_retry, progress).await? {
                Some(final_file) => return Ok(final_file),
                None => continue,
            }
        }
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(source) if ranged && rejects_range(status) => {
                tracing::warn!(%status, %source, "ranged tarball request rejected; retrying without a range");
                resume_refused = true;
                continue;
            }
            Err(source) if retryable_status(status) => {
                budget
                    .retry_signed(InitStoreError::DownloadFailed { source }, Progress::None)
                    .await?;
                if ranged && budget.consecutive_no_progress_failures >= RANGED_ERRORS_BEFORE_PLAIN {
                    tracing::warn!(%status, "ranged tarball requests keep failing; retrying without a range");
                    resume_refused = true;
                }
                continue;
            }
            Err(source) => return Err(InitStoreError::DownloadFailed { source }),
        };
        let disposition = if ranged && status == reqwest::StatusCode::PARTIAL_CONTENT {
            let Some(disposition) = resume_disposition(&response, offset, &metadata) else {
                resume_refused = true;
                continue;
            };
            record_total(paths, &mut metadata, disposition)?;
            disposition
        } else {
            let length = match restart_length(&response) {
                Ok(length) => length,
                Err(error) if ranged => {
                    tracing::warn!(%status, %error, "unusable reply to a ranged tarball request; retrying without a range");
                    resume_refused = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            resume_refused = false;
            accept_restart(paths, &mut metadata, &response, length, progress)?
        };
        let start = disposition.start(offset);
        match stream_part(paths, response, disposition, offset, progress).await {
            Ok(()) => {}
            Err(AttemptError::Fatal(error)) => return Err(error),
            Err(AttemptError::Transient(error)) => {
                budget
                    .retry_signed(error, progress_since(paths, start)?)
                    .await?;
                continue;
            }
        }
        if progress_since(paths, start)? == Progress::Made {
            budget.observe_progress();
        }
        match disposition {
            ResponseDisposition::Resume { total, .. } => {
                if paths.part_len()? < total {
                    continue;
                }
                match verify_candidate(paths, key, signature, &mut clean_retry, progress).await? {
                    Some(final_file) => return Ok(final_file),
                    None => continue,
                }
            }
            ResponseDisposition::Restart { .. } => {
                return verify_fresh(paths, key, signature).await;
            }
        }
    }
}

/// Reuse a final tarball that still verifies.
/// One that does not is most likely left over from an older release,
/// which says nothing about the server, so dropping it spends no clean retry.
async fn reuse_final(
    paths: &DownloadPaths,
    key: &str,
    signature: &str,
) -> Result<Option<PathBuf>, InitStoreError> {
    if !paths
        .final_file
        .try_exists()
        .map_err(InitStoreError::DownloadFileIo)?
    {
        return Ok(None);
    }
    match verify(&paths.final_file, key, signature).await {
        Ok(()) => {
            remove_artifact(&paths.part)?;
            remove_artifact(&paths.metadata)?;
            Ok(Some(paths.final_file.clone()))
        }
        Err(InitStoreError::SignatureVerificationFailed { .. }) => {
            paths.clear_all()?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn tarball_request(
    client: &reqwest::Client,
    url: &str,
    offset: u64,
    etag: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = client.get(url).header(ACCEPT_ENCODING, "identity");
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
        // A sidecar tag that does not qualify would fail the request builder
        // on every run, so it is left out rather than sent.
        if let Some(etag) = etag.filter(|etag| is_strong_etag(etag)) {
            request = request.header(IF_RANGE, etag);
        }
    }
    request
}

/// A fresh download that fails verification is definitive:
/// no retained bytes were involved, so a clean retry would fetch the same bytes.
async fn verify_fresh(
    paths: &DownloadPaths,
    key: &str,
    signature: &str,
) -> Result<PathBuf, InitStoreError> {
    match verify(&paths.part, key, signature).await {
        Ok(()) => {
            promote(paths).await?;
            Ok(paths.final_file.clone())
        }
        Err(InitStoreError::SignatureVerificationFailed { source }) => {
            remove_artifact(&paths.part)?;
            remove_artifact(&paths.metadata)?;
            Err(InitStoreError::SignatureVerificationFailed { source })
        }
        Err(error) => Err(error),
    }
}

fn progress_since(paths: &DownloadPaths, start: u64) -> Result<Progress, InitStoreError> {
    Ok(if paths.part_len()? > start {
        Progress::Made
    } else {
        Progress::None
    })
}

/// Record the identity of a fresh 200 in place of whatever was retained; the
/// progress reset is reported only when retained bytes had been reported.
fn accept_restart(
    paths: &DownloadPaths,
    metadata: &mut DownloadMetadata,
    response: &reqwest::Response,
    length: Option<u64>,
    progress: Option<&dyn DownloadProgress>,
) -> Result<ResponseDisposition, InitStoreError> {
    metadata.etag = strong_etag(response);
    metadata.total = length;
    if paths.part_len()? > 0 {
        discard_partial(paths, progress)?;
    } else {
        remove_artifact(&paths.part)?;
    }
    paths.publish_metadata(metadata)?;
    Ok(ResponseDisposition::Restart { length })
}

fn discard_partial(
    paths: &DownloadPaths,
    progress: Option<&dyn DownloadProgress>,
) -> Result<(), InitStoreError> {
    remove_artifact(&paths.part)?;
    if let Some(progress) = progress {
        progress.on_bytes_downloaded(0, None);
    }
    Ok(())
}

/// Verify the complete retained partial: promote it, or discard it and spend
/// the one clean retry (None), failing definitively once that is spent.
async fn verify_candidate(
    paths: &DownloadPaths,
    key: &str,
    signature: &str,
    clean_retry: &mut CleanRetry,
    progress: Option<&dyn DownloadProgress>,
) -> Result<Option<PathBuf>, InitStoreError> {
    match verify(&paths.part, key, signature).await {
        Ok(()) => {
            promote(paths).await?;
            Ok(Some(paths.final_file.clone()))
        }
        Err(InitStoreError::SignatureVerificationFailed { source }) => {
            if *clean_retry == CleanRetry::Spent {
                remove_artifact(&paths.part)?;
                remove_artifact(&paths.metadata)?;
                return Err(InitStoreError::SignatureVerificationFailed { source });
            }
            *clean_retry = CleanRetry::Spent;
            discard_partial(paths, progress)?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

async fn promote(paths: &DownloadPaths) -> Result<(), InitStoreError> {
    tokio::fs::File::open(&paths.part)
        .await
        .map_err(InitStoreError::DownloadFileIo)?
        .sync_all()
        .await
        .map_err(InitStoreError::DownloadFileIo)?;
    std::fs::rename(&paths.part, &paths.final_file).map_err(InitStoreError::DownloadFileIo)?;
    crate::fs_sync::fsync_dir(&paths.dir).map_err(InitStoreError::DownloadFileIo)?;
    remove_artifact(&paths.metadata)
}

async fn download_transfer(
    client: &reqwest::Client,
    url: &str,
    paths: &DownloadPaths,
    progress: Option<&dyn DownloadProgress>,
) -> Result<(), InitStoreError> {
    let mut budget = FailureBudget::default();
    loop {
        let response = match send(
            client.get(url).header(ACCEPT_ENCODING, "identity"),
            |source| InitStoreError::DownloadFailed { source },
        )
        .await
        {
            Ok(response) => response,
            Err(AttemptError::Fatal(error)) => return Err(error),
            Err(AttemptError::Transient(error)) => {
                budget.retry(error).await?;
                continue;
            }
        };
        let status = response.status();
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(source) if retryable_status(status) => {
                budget
                    .retry(InitStoreError::DownloadFailed { source })
                    .await?;
                continue;
            }
            Err(source) => return Err(InitStoreError::DownloadFailed { source }),
        };
        let disposition = ResponseDisposition::Restart {
            length: restart_length(&response)?,
        };
        match stream_part(paths, response, disposition, 0, progress).await {
            Ok(()) => return Ok(()),
            Err(AttemptError::Fatal(error)) => return Err(error),
            Err(AttemptError::Transient(error)) => budget.retry(error).await?,
        }
    }
}

pub(super) fn cleanup(dir: &Path) -> Result<(), InitStoreError> {
    DownloadPaths::new(dir).clear_all()
}

#[cfg(test)]
mod timeout_tests;

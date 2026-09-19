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

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::*;

async fn serve_then_stall(
    response: &'static [u8],
) -> (String, oneshot::Receiver<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("BUG: bind loopback listener");
    let url = format!(
        "http://{}",
        listener.local_addr().expect("BUG: read loopback address")
    );
    let (ready_tx, ready_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("BUG: accept request");
        let mut request = [0_u8; 1];
        stream
            .read_exact(&mut request)
            .await
            .expect("BUG: read request");
        stream
            .write_all(response)
            .await
            .expect("BUG: write response");
        ready_tx.send(()).expect("BUG: observe stalled response");
        std::future::pending::<()>().await;
    });
    (url, ready_rx, server)
}

#[tokio::test]
async fn response_header_stall_is_transient() {
    let (url, ready, server) = serve_then_stall(b"").await;
    let request = reqwest::Client::new().get(url);
    let attempt = tokio::spawn(async move {
        send(request, |source| InitStoreError::DownloadFailed { source }).await
    });
    ready.await.expect("BUG: request reached loopback server");
    tokio::time::pause();
    tokio::time::advance(HTTP_IDLE_TIMEOUT + Duration::from_millis(1)).await;

    assert!(matches!(
        attempt.await.expect("BUG: download task panicked"),
        Err(AttemptError::Transient(
            InitStoreError::DownloadStalled { .. }
        ))
    ));
    server.abort();
}

#[tokio::test]
async fn tarball_body_stall_retains_written_prefix() {
    let (url, ready, server) = serve_then_stall(
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: keep-alive\r\n\r\nhello",
    )
    .await;
    let request = reqwest::Client::new().get(url);
    let Ok(response) = send(request, |source| InitStoreError::DownloadFailed { source }).await
    else {
        panic!("expected response headers before the body stall");
    };
    ready.await.expect("BUG: response reached download client");
    tokio::time::pause();
    let temp = tempfile::tempdir().expect("BUG: create download directory");
    let paths = DownloadPaths::new(temp.path());
    let part = paths.part.clone();
    let attempt = tokio::spawn(async move {
        stream_part(
            &paths,
            response,
            ResponseDisposition::Restart { length: Some(10) },
            0,
            None,
        )
        .await
    });

    assert!(matches!(
        attempt.await.expect("BUG: download task panicked"),
        Err(AttemptError::Transient(
            InitStoreError::DownloadStalled { .. }
        ))
    ));
    assert_eq!(
        std::fs::read(part).expect("BUG: read retained partial"),
        b"hello"
    );
    server.abort();
}

#[test]
fn signed_retry_budget_bounds_progressing_interruptions() {
    let mut budget = FailureBudget::default();
    for _ in 0..MAX_SIGNED_RETRIES {
        assert_eq!(
            budget.signed_retry_delay(Progress::Made),
            Some(RETRY_DELAYS[0])
        );
    }
    assert_eq!(budget.signed_retry_delay(Progress::Made), None);
}

#[test]
fn signed_retry_budget_resets_only_the_no_progress_streak() {
    let mut budget = FailureBudget::default();
    assert_eq!(
        budget.signed_retry_delay(Progress::None),
        Some(RETRY_DELAYS[0])
    );
    assert_eq!(
        budget.signed_retry_delay(Progress::None),
        Some(RETRY_DELAYS[1])
    );
    assert_eq!(
        budget.signed_retry_delay(Progress::Made),
        Some(RETRY_DELAYS[0])
    );
    for delay in RETRY_DELAYS {
        assert_eq!(budget.signed_retry_delay(Progress::None), Some(delay));
    }
    assert_eq!(budget.signed_retry_delay(Progress::None), None);
}

#[test]
fn signed_retry_budget_resets_after_successful_progress() {
    let mut budget = FailureBudget::default();
    assert_eq!(
        budget.signed_retry_delay(Progress::None),
        Some(RETRY_DELAYS[0])
    );
    assert_eq!(
        budget.signed_retry_delay(Progress::None),
        Some(RETRY_DELAYS[1])
    );
    budget.observe_progress();
    for delay in RETRY_DELAYS {
        assert_eq!(budget.signed_retry_delay(Progress::None), Some(delay));
    }
    assert_eq!(budget.signed_retry_delay(Progress::None), None);
}

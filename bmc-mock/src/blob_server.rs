// Copyright (C) 2026  Braiins Systems s.r.o.

//! Loopback HTTP server serving a deterministic firmware blob in throttled
//! chunks, so the real download pipeline runs offline with visible
//! progress. The fail path closes the connection before writing any HTTP
//! response so the client's send() errors (a mid-stream drop would be
//! reported as a hash mismatch by the download pipeline instead).

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::debug;

use crate::pacing::UpgradePacing;

const BLOB_SIZE: usize = 24_000_000;
// Paired with blob(); the blob_hash_constant_matches_generator test
// verifies the pair.
const BLOB_SHA256: &str = "f828b304909d5afda58e678369cecb41e147c11b931723364bec5bc075aa4497";
const CHUNK_SIZE: usize = 256 * 1024;
const FAIL_PATH: &str = "/firmware-fail.tar";

fn blob() -> Vec<u8> {
    #[expect(clippy::cast_possible_truncation, reason = "modulo 251 fits u8")]
    (0..BLOB_SIZE).map(|i| (i % 251) as u8).collect()
}

#[derive(Debug, Clone)]
pub struct BlobServer {
    pub url: String,
    pub fail_url: String,
    pub hash: String,
    pub size: usize,
}

pub async fn spawn(pacing: UpgradePacing) -> std::io::Result<BlobServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let chunk_delay = pacing.blob_chunk_delay();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(handle_connection(stream, chunk_delay));
        }
    });

    Ok(BlobServer {
        url: format!("http://{addr}/firmware.tar"),
        fail_url: format!("http://{addr}{FAIL_PATH}"),
        hash: BLOB_SHA256.to_owned(),
        size: BLOB_SIZE,
    })
}

async fn handle_connection(mut stream: TcpStream, chunk_delay: Duration) {
    let mut request = Vec::new();
    let mut buf = [0_u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }

    let request_text = String::from_utf8_lossy(&request);
    if request_text
        .lines()
        .next()
        .is_some_and(|l| l.contains(FAIL_PATH))
    {
        debug!("blob server dropping connection for fail path");
        return;
    }

    let header =
        format!("HTTP/1.1 200 OK\r\ncontent-length: {BLOB_SIZE}\r\nconnection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).await.is_err() {
        return;
    }

    let body = blob();
    for chunk in body.chunks(CHUNK_SIZE) {
        if stream.write_all(chunk).await.is_err() {
            return;
        }
        tokio::time::sleep(chunk_delay).await;
    }
    _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn blob_hash_constant_matches_generator() {
        let mut hasher = Sha256::new();
        hasher.update(blob());
        let actual = hex_encode(&hasher.finalize());
        assert_eq!(
            actual, BLOB_SHA256,
            "update BLOB_SHA256 to the printed value if the generator changed"
        );
    }

    fn hex_encode(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
    }

    #[tokio::test]
    async fn serves_full_blob_with_matching_hash() {
        let server = spawn(UpgradePacing::Instant)
            .await
            .expect("BUG: blob server spawn");
        let body = reqwest::get(&server.url)
            .await
            .expect("BUG: request failed")
            .bytes()
            .await
            .expect("BUG: body read failed");
        assert_eq!(body.len(), server.size);
        let mut hasher = Sha256::new();
        hasher.update(&body);
        assert_eq!(hex_encode(&hasher.finalize()), server.hash);
    }

    #[tokio::test]
    async fn fail_url_errors_before_any_response() {
        let server = spawn(UpgradePacing::Instant)
            .await
            .expect("BUG: blob server spawn");
        let result = reqwest::get(&server.fail_url).await;
        assert!(result.is_err(), "send() must fail, got {result:?}");
    }
}

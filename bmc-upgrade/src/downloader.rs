// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    fmt::Debug,
    io::Cursor,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use data_encoding::HEXUPPER;
use ring::digest::{Context, SHA256};
use thiserror::Error;
use tokio::{
    fs::File,
    io::{AsyncWrite, AsyncWriteExt, BufWriter},
};

use crate::utils::file_hash;

pub type FileDownloader = DownloadWriter<BufWriter<File>>;

#[async_trait::async_trait]
pub trait Downloader: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync;

    async fn write_chunk(&mut self, chunk: Bytes) -> Result<(), Self::Error>;
    async fn finish(mut self) -> Result<String, Self::Error>;
}

pub struct DownloadWriter<T: AsyncWrite + Unpin + Send + Sync + 'static> {
    writer: T,
    context: Context,
    download_finished: bool,
    path: Option<PathBuf>, // optional for file-specific logic
}

impl<T: AsyncWrite + Unpin + Send + Sync + 'static> Debug for DownloadWriter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadWriter")
            .field("download_finished", &self.download_finished)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl<T: AsyncWrite + Unpin + Send + Sync + 'static> Downloader for DownloadWriter<T> {
    type Error = std::io::Error;

    async fn write_chunk(&mut self, chunk: Bytes) -> std::io::Result<()> {
        self.writer.write_all(&chunk).await?;
        self.context.update(&chunk);
        Ok(())
    }

    async fn finish(mut self) -> std::io::Result<String> {
        self.writer.flush().await?;
        self.download_finished = true;

        let digest = self.context.clone().finish();
        Ok(HEXUPPER.encode(digest.as_ref()))
    }
}

impl FileDownloader {
    pub async fn init(path: impl AsRef<Path> + Send) -> std::io::Result<Self> {
        let context = Context::new(&SHA256);

        let path = path.as_ref().to_owned();
        let file = File::create(path.clone()).await?;
        let writer = BufWriter::new(file);

        Ok(Self {
            writer,
            context,
            download_finished: false,
            path: Some(path),
        })
    }

    pub async fn verify_hash(path: impl AsRef<Path>, hash: &str) -> Result<(), DownloaderError> {
        let file_hash = file_hash(path)
            .await
            .map_err(DownloaderError::FailedToReadFile)?
            .to_lowercase();

        let hash = hash.to_lowercase();

        if file_hash != hash {
            return Err(DownloaderError::HashMismatch {
                expected: hash,
                actual: file_hash,
            });
        }
        Ok(())
    }
}

// NOTE: in memory writer, used for testing
impl DownloadWriter<BufWriter<Cursor<Vec<u8>>>> {
    #[expect(dead_code)]
    fn new() -> Self {
        let context = Context::new(&SHA256);
        let buffer = Cursor::new(Vec::new());
        let writer = BufWriter::new(buffer);

        Self {
            writer,
            context,
            download_finished: false,
            path: None, // no file path needed
        }
    }
}

impl<T: AsyncWrite + Unpin + Send + Sync + 'static> Drop for DownloadWriter<T> {
    fn drop(&mut self) {
        if !self.download_finished
            && let Some(ref path) = self.path
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Error, Debug)]
pub enum DownloaderError {
    #[error("image checksum mismatch. Expected {expected}, actual {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("failed to read file")]
    FailedToReadFile(#[from] std::io::Error),
}

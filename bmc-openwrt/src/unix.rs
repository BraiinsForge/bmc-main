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

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use bmc::shutdown::UPGRADE_HOLD;
use bmc_support::ArchiveFormat;
use tokio::io::{AsyncRead, DuplexStream, ReadBuf};
use tokio::sync::oneshot::{self, Receiver, error::TryRecvError};
use tokio::task;
use tokio_util::io::SyncIoBridge;
use tracing::{error, info};

use crate::{signal, sys};

const REBOOT_COMMAND: &str = "reboot";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Sys error: {0}")]
    Sys(#[from] sys::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub async fn call_command<T>(command_name: T, args: &[T]) -> Result<(), Error>
where
    T: ToString + Sync + Send,
{
    sys::call_command_to_string(command_name, args)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

pub async fn system_reboot() -> Result<(), Error> {
    call_command(REBOOT_COMMAND, &[]).await
}

// During a system upgrade the shutdown of the Axum web server is delayed by
// sleeping, so the server survives sysupgrade's SIGTERM long enough to
// deliver the last progress events to clients. Outside an upgrade the server
// shuts down immediately.
pub async fn handle_graceful_shutdown(upgrade_in_progress: &AtomicBool) {
    let signal = signal::wait_for_first_signal(signal::SHUTDOWN_SIGNALS).await;

    if !upgrade_in_progress.load(Ordering::SeqCst) {
        info!("{:?} signal received. Shutting down", signal);
        return;
    }

    info!(
        "{:?} signal received. Waiting for {:?}s, then shutting down",
        signal,
        UPGRADE_HOLD.as_secs()
    );
    tokio::time::sleep(UPGRADE_HOLD).await;
    info!("Timeout reached. Forcefully shutting down...");
}

/// Buffer size for the duplex channel between the blocking archive writer
/// and the async reader; also bounds the archive's peak memory footprint.
const SUPPORT_ARCHIVE_BUF_SIZE: usize = 8 * 1024;

/// Async reader over the duplex stream fed by the blocking collector.
///
/// On EOF it checks the producer's result and turns a collection failure into
/// an [`io::Error`], so a mid-collection failure aborts the download instead
/// of silently truncating it.
struct SupportArchiveReader {
    inner: DuplexStream,
    result_rx: Receiver<anyhow::Result<()>>,
    finished: bool,
}

impl AsyncRead for SupportArchiveReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // NOTE: EOF resolution is one-shot: try_recv consumes the producer result, so once it has
        // been read we report a clean EOF for any further polls. Readers may legally poll past EOF,
        // and the inner stream's EOF is sticky, so a second try_recv would otherwise observe Closed.
        if self.finished {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) if buf.filled().is_empty() => {
                self.finished = true;
                match self.result_rx.try_recv() {
                    Ok(Ok(())) => Poll::Ready(Ok(())),
                    Ok(Err(err)) => Poll::Ready(Err(io::Error::other(format!(
                        "support archive collection failed: {err}"
                    )))),
                    // NOTE: only reachable if collect() panicked mid-stream; fail
                    // loud rather than serve a truncated archive as a clean download.
                    Err(TryRecvError::Empty) => Poll::Ready(Err(io::Error::other(
                        "support archive collector did not report before end of stream",
                    ))),
                    Err(TryRecvError::Closed) => Poll::Ready(Err(io::Error::other(
                        "support archive collection task terminated unexpectedly",
                    ))),
                }
            }
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub fn get_support_archive(
    format: &'static dyn ArchiveFormat,
) -> impl AsyncRead + Send + Unpin + 'static {
    let (reader, writer) = tokio::io::duplex(SUPPORT_ARCHIVE_BUF_SIZE);
    let (tx, rx) = oneshot::channel();

    task::spawn_blocking(move || {
        let mut sync_writer = SyncIoBridge::new(writer);
        let result = crate::support::SUPPORT_CONFIG.collect(&mut sync_writer, format, false);
        if let Err(ref err) = result {
            let client_gone = err.chain().any(|cause| {
                cause
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
            });
            if client_gone {
                info!("Support archive download cancelled by client");
            } else {
                error!("Support archive collection failed: {err}");
            }
        }
        let _ = tx.send(result);
    });

    SupportArchiveReader {
        inner: reader,
        result_rx: rx,
        finished: false,
    }
}

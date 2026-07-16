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

//! Shared logging setup for BMC binaries.
//!
//! Every rotated log file has exactly one writer process: `file-rotate`
//! is not multi-process safe, so a log path must never be opened by two
//! processes at once. Binaries either own a rotated file ([`init_file`]),
//! log to stderr ([`init_console`]), or combine both behind a sidecar
//! flock ([`init_file_and_console`]), which falls back to stderr only
//! when the file is contended.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::sync::Mutex;

use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};
use tracing_subscriber::filter::{FilterExt, LevelFilter, Targets};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

pub mod flock;

/// Event target that routes captured widget output into the widget log
/// file instead of the main one.
pub const WIDGET_OUTPUT_TARGET: &str = "widget_output";

/// Rotate the log file once it surpasses this size.
const LOG_ROTATE_THRESHOLD: usize = 512 * 1024;

/// Number of rotated (compressed) files to keep.
const LOG_ROTATE_FILES_KEEP: usize = 9;

/// Result of [`init_file_and_console`]: keeps the sidecar log lock alive
/// for the logging lifetime.
#[derive(Debug)]
pub struct FileConsoleGuard {
    _lock: Option<flock::FileLock>,
}

fn lock_path_for(log_path: &Path) -> io::Result<std::path::PathBuf> {
    let file_name = log_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "log path has no file name"))?;
    let mut lock_name = OsString::from(file_name);
    lock_name.push(".lock");
    Ok(log_path.with_file_name(lock_name))
}

fn try_lock_log_file(log_path: &Path) -> io::Result<Option<flock::FileLock>> {
    let lock_path = lock_path_for(log_path)?;
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;

    flock::try_lock_file(file)
}

fn open_locked_log_file(path: &Path) -> Result<(flock::FileLock, FileRotate<AppendCount>), String> {
    let Some(lock) =
        try_lock_log_file(path).map_err(|err| format!("failed to acquire log lock: {err}"))?
    else {
        return Err("log lock is already held by another process".to_owned());
    };

    let log_file = open_log_file(path).map_err(|err| format!("failed to open log file: {err}"))?;
    Ok((lock, log_file))
}

/// Message-only stderr layer for events with target `console_target`,
/// plus warnings and errors from any target.
fn console_layer<S>(console_target: &'static str) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(
            Targets::new()
                .with_default(LevelFilter::WARN)
                .with_target(console_target, LevelFilter::TRACE),
        )
}

/// Open `path` as a size-rotated log writer, creating parent directories.
fn open_log_file(path: &Path) -> io::Result<FileRotate<AppendCount>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(FileRotate::new(
        path,
        AppendCount::new(LOG_ROTATE_FILES_KEEP),
        ContentLimit::BytesSurpassed(LOG_ROTATE_THRESHOLD),
        Compression::OnRotate(0),
        None,
    ))
}

/// Initialize tracing to stderr, without ANSI escapes.
pub fn init_console() {
    tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

/// Initialize tracing to a rotated log file at `path`.
pub fn init_file(path: &Path) -> io::Result<()> {
    let log_file = open_log_file(path)?;
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(log_file))
        .with_filter(env_filter());

    tracing_subscriber::registry().with(file_layer).init();
    tracing::info!(log_path = %path.display(), "file logging initialized");
    Ok(())
}

/// Initialize tracing to a rotated log file at `path` plus a
/// message-only stderr echo of events with target `console_target`.
///
/// The rotated file is guarded by a non-blocking sidecar flock so a
/// concurrent process never opens the same rotated log. On lock
/// contention (or any file setup failure) logging falls back to the
/// console layer only and the returned guard carries the reason.
///
/// The console layer prints `console_target` events plus every `WARN`+
/// event from any target to stderr. Stderr is therefore the human
/// diagnostic channel and may carry library warnings; stdout stays
/// reserved for machine-readable command output. Callers that pipe stdout
/// are unaffected by this stderr noise.
pub fn init_file_and_console(path: &Path, console_target: &'static str) -> FileConsoleGuard {
    match open_locked_log_file(path) {
        Ok((lock, log_file)) => {
            // The console target is always kept, even when `RUST_LOG`
            // would otherwise filter it out, so CLI diagnostics always
            // reach the persisted log.
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(log_file))
                .with_filter(
                    env_filter().or(Targets::new().with_target(console_target, LevelFilter::TRACE)),
                );

            tracing_subscriber::registry()
                .with(file_layer)
                .with(console_layer(console_target))
                .init();
            tracing::info!(log_path = %path.display(), "file logging initialized");
            FileConsoleGuard { _lock: Some(lock) }
        }
        Err(reason) => {
            tracing_subscriber::registry()
                .with(console_layer(console_target))
                .init();
            eprintln!("file logging disabled: {reason}");
            FileConsoleGuard { _lock: None }
        }
    }
}

/// Initialize tracing for a process that owns two rotated log files:
/// its own events go to `path`, captured widget output (events with
/// target [`WIDGET_OUTPUT_TARGET`]) goes message-only to
/// `widget_log_path`.
pub fn init_file_with_widget_capture(path: &Path, widget_log_path: &Path) -> io::Result<()> {
    let own_file = open_log_file(path)?;
    let widget_file = open_log_file(widget_log_path)?;

    let own_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(own_file))
        .with_filter(
            env_filter().and(
                Targets::new()
                    .with_default(LevelFilter::TRACE)
                    .with_target(WIDGET_OUTPUT_TARGET, LevelFilter::OFF),
            ),
        );

    // Captured lines carry the widget's own timestamp and level, so this
    // layer emits the message only.
    let widget_layer = fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_writer(Mutex::new(widget_file))
        .with_filter(Targets::new().with_target(WIDGET_OUTPUT_TARGET, LevelFilter::TRACE));

    tracing_subscriber::registry()
        .with(own_layer)
        .with(widget_layer)
        .init();
    tracing::info!(log_path = %path.display(), "file logging initialized");
    Ok(())
}

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::open_log_file;

    #[test]
    fn open_log_file_creates_parent_and_appends() {
        let td = tempfile::tempdir().expect("BUG: tempdir");
        let log_path = td.path().join("var/log/bmc/test.log");

        {
            let mut file = open_log_file(&log_path).expect("BUG: open log file");
            writeln!(file, "first").expect("BUG: write first log line");
        }
        {
            let mut file = open_log_file(&log_path).expect("BUG: reopen log file");
            writeln!(file, "second").expect("BUG: write second log line");
        }

        let contents = std::fs::read_to_string(&log_path).expect("BUG: read log file");
        assert_eq!(contents, "first\nsecond\n");
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared logging setup for BMC binaries.
//!
//! Every rotated log file has exactly one writer process: `file-rotate`
//! is not multi-process safe, so a log path must never be opened by two
//! processes at once. Binaries either own a rotated file ([`init_file`])
//! or log to stderr ([`init_console`]).

use std::io;
use std::path::Path;
use std::sync::Mutex;

use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};
use tracing_subscriber::filter::{FilterExt, LevelFilter, Targets};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

/// Event target that routes captured widget output into the widget log
/// file instead of the main one.
pub const WIDGET_OUTPUT_TARGET: &str = "widget_output";

/// Rotate the log file once it surpasses this size.
const LOG_ROTATE_THRESHOLD: usize = 512 * 1024;

/// Number of rotated (compressed) files to keep.
const LOG_ROTATE_FILES_KEEP: usize = 9;

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

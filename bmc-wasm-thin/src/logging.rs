// Copyright (C) 2026  Braiins Systems s.r.o.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Mutex;

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

pub const LOG_PATH: &str = "/var/log/bmc/bmc-wasm-thin.log";

pub fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

pub fn init() {
    let path = Path::new(LOG_PATH);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()));

    match open_log_file(path) {
        Ok(log_file) => {
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(log_file));
            let stderr_layer = fmt::layer().with_ansi(false).with_writer(std::io::stderr);

            tracing_subscriber::registry()
                .with(file_layer)
                .with(stderr_layer)
                .with(filter)
                .init();
            tracing::info!(log_path = %path.display(), "file logging initialized");
        }
        Err(err) => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
            tracing::warn!(
                ?err,
                log_path = %path.display(),
                "failed to open log file; using stderr only"
            );
        }
    }
}

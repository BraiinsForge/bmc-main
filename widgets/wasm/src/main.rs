// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! WASM widget - runs WebAssembly applications via bmc-wasm-runtime
//!
//! This widget loads and executes .wasm files, rendering their output
//! to a Wayland surface via DMA-BUF. Uses the same two-FBO pipeline
//! as the settings widget for FemtoVG Y-flip correction.

mod egl;
mod wayland;

use std::fs::OpenOptions;
use std::sync::Mutex;

use anyhow::Result;
use tracing_subscriber::{EnvFilter, filter::LevelFilter, fmt, prelude::*};

const WIDGET_LOG_PATH: &str = "/var/log/bmc/wasm-widget.log";

fn main() -> Result<()> {
    // Initialize logging to a dedicated file.
    // Widget stdout/stderr are inherited from BMC but may not be visible,
    // so we write directly to a log file for reliable debugging.
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(WIDGET_LOG_PATH)
        .expect("BUG: failed to open wasm-widget log file");

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()));

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(log_file)),
        )
        .with(filter)
        .init();

    tracing::info!("Starting WASM widget {}", env!("GIT_VERSION"),);

    // Connect to Wayland and run
    let mut client = wayland::WaylandClient::connect()?;

    tracing::info!("Connected to Wayland display");

    // Run the event loop
    client.run()?;

    Ok(())
}

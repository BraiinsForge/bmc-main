// Copyright (C) 2026  Braiins Systems s.r.o.
//
//! WASM widget - runs WebAssembly applications via bmc-wasm-runtime
//!
//! This widget loads and executes .wasm files, rendering their output
//! to a Wayland surface via DMA-BUF. Uses the same two-FBO pipeline
//! as the settings widget for FemtoVG Y-flip correction.

mod egl;
mod wayland;

use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    tracing::info!("Starting WASM widget");

    // Connect to Wayland and run
    let mut client = wayland::WaylandClient::connect()?;

    tracing::info!("Connected to Wayland display");

    // Run the event loop
    client.run()?;

    Ok(())
}

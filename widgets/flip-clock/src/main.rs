// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Flip-clock widget - GPU accelerated clock with split-flap animation
//!
//! This widget uses OpenGL ES for rendering and DMA-BUF for zero-copy
//! buffer sharing with the compositor.

mod digits;
mod digits3d;
mod egl;
pub mod ipc;
mod renderer;
mod wayland;

use std::fs::OpenOptions;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use tracing_subscriber::{EnvFilter, filter::LevelFilter, fmt, prelude::*};

/// Animation mode for the flip-clock
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum AnimationMode {
    /// 2D flat flip animation with textures
    Flat,
    /// 3D extruded digits with perspective
    #[default]
    Extruded,
}

/// Flip-clock widget - GPU accelerated clock with split-flap animation
#[derive(Parser, Debug)]
#[command(name = "flip-clock", about, version)]
struct Args {
    /// Run in standalone mode without IPC
    #[arg(long)]
    standalone: bool,

    /// Animation mode (standalone only)
    #[arg(short, long, value_enum, default_value_t = AnimationMode::default())]
    mode: AnimationMode,
}

const WIDGET_LOG_PATH: &str = "/var/log/bmc/flip-clock-widget.log";

fn main() -> Result<()> {
    // Initialize logging to a dedicated file.
    // Widget stdout/stderr are inherited from BMC but may not be visible,
    // so we write directly to a log file for reliable debugging.
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(WIDGET_LOG_PATH)
        .expect("BUG: failed to open flip-clock-widget log file");

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()));

    // Initialize logging
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(log_file)),
        )
        .with(filter)
        .init();

    let args = Args::parse();

    if args.standalone {
        run_standalone(args.mode)?;
    } else {
        run_with_protocol()?;
    }

    Ok(())
}

fn run_standalone(mode: AnimationMode) -> Result<()> {
    tracing::info!("Starting flip-clock widget in standalone mode (mode: {mode:?})");

    let timezone = Arc::new(RwLock::new(String::from("UTC")));
    let shutdown = Arc::new(AtomicBool::new(false));

    let mut client = wayland::WaylandClient::connect(mode, 640, 480, timezone, shutdown)?;
    tracing::info!("Connected to Wayland display");

    client.run(None)?;

    Ok(())
}

fn run_with_protocol() -> Result<()> {
    let (instance_id, config) = ipc::read_config()?;

    tracing::info!(
        "Starting flip-clock widget (mode: {:?}, {}x{}, tz: {})",
        config.mode,
        config.width,
        config.height,
        config.timezone
    );

    let timezone = Arc::new(RwLock::new(config.timezone));
    let shutdown = Arc::new(AtomicBool::new(false));

    let (protocol_client, handler) =
        ipc::setup_protocol(&instance_id, Arc::clone(&timezone), Arc::clone(&shutdown))?;

    let mut client = wayland::WaylandClient::connect(
        config.mode,
        config.width,
        config.height,
        timezone,
        shutdown,
    )?;
    tracing::info!("Connected to Wayland display");

    client.run(Some((protocol_client, handler)))?;

    Ok(())
}

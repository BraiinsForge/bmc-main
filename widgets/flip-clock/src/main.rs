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
mod layout;
mod renderer;
mod wayland;

use std::fs::OpenOptions;
use std::sync::Mutex;

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
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(WIDGET_LOG_PATH)
        .expect("BUG: failed to open flip-clock-widget log file");

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
    wayland::connect_standalone(mode, 640, 480, String::from("UTC"))
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

    wayland::connect_production(
        &instance_id,
        config.mode,
        config.width,
        config.height,
        config.timezone,
    )
}

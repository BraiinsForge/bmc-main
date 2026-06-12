// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Flip-clock widget - GPU accelerated clock with split-flap animation
//!
//! This widget uses OpenGL ES for rendering and DMA-BUF for zero-copy
//! buffer sharing with the compositor.

mod digits;
mod digits3d;
mod egl;
mod font;
mod layout;
mod renderer;
mod wayland;
pub mod widget_protocol;

use anyhow::Result;
use clap::{Parser, ValueEnum};

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

fn main() -> Result<()> {
    bmc_log::init_console();

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
    // Connect first, then decode widget-specific state from the initial
    // configure batch. Previously this was reversed (read env vars, then
    // connect with known geometry).
    let (surface, initial) = wayland::connect_production()?;
    let mode = widget_protocol::animation_mode_from_params(&initial.params)?;
    let timezone_override = widget_protocol::timezone_override_from_params(&initial.params)?;

    tracing::info!(
        "Starting flip-clock widget (mode: {:?}, {}x{}, tz_override: {:?})",
        mode,
        initial.width,
        initial.height,
        timezone_override,
    );

    wayland::run_production(surface, mode, timezone_override, &initial.settings)
}

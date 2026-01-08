// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_widget_digital_clock::{Config, DigitalClockWidget, ipc};
use clap::Parser;

#[derive(Parser)]
#[command(name = "digital-clock", about = "Digital clock widget")]
struct Args {
    #[arg(long, help = "Run in standalone mode without IPC")]
    standalone: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    if args.standalone {
        run_standalone()?;
    } else {
        run_with_wayland()?;
    }

    Ok(())
}

fn run_standalone() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let widget = DigitalClockWidget::new(Config::default())?;
    widget.run()?;
    Ok(())
}

fn run_with_wayland() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (instance_id, config) = ipc::read_config()?;
    let widget = DigitalClockWidget::new(config)?;

    // Timer must be kept alive for the duration of the widget
    let (_wayland_timer, _shutdown_flag) = ipc::setup_wayland_events(
        &instance_id,
        widget.date_format(),
        widget.timezone(),
        widget.is_24_format(),
    )?;

    widget.run()?;
    Ok(())
}

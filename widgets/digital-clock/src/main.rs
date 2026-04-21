// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_widget_digital_clock::{Config, DigitalClockWidget, widget_protocol};
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
    // Connect to the compositor first; the configure batch drives the
    // widget's initial config, so we can't instantiate the widget until
    // that has arrived.
    let (protocol_client, config) = widget_protocol::connect_and_read_config()?;
    let widget = DigitalClockWidget::new(config)?;

    // Timer must be kept alive for the duration of the widget
    let (_wayland_timer, _shutdown_flag) = widget_protocol::spawn_runtime_handler(
        protocol_client,
        widget.date_format(),
        widget.timezone(),
        widget.is_24_format(),
    );

    widget.run()?;
    Ok(())
}

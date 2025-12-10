// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_widget_digital_clock::{Config, DigitalClockWidget, ipc};
use clap::Parser;

#[derive(Parser)]
#[command(name = "digital-clock", about = "Digital clock widget")]
struct Args {
    #[arg(long, help = "Run in standalone mode without IPC")]
    standalone: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    if args.standalone {
        run_standalone()?;
    } else {
        run_with_ipc().await?;
    }

    Ok(())
}

fn run_standalone() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let widget = DigitalClockWidget::new(Config::default())?;
    widget.run()?;
    Ok(())
}

async fn run_with_ipc() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (client, config) = ipc::connect().await?;
    let widget = DigitalClockWidget::new(config)?;

    tokio::spawn(ipc::run(
        client,
        widget.date_format(),
        widget.timezone(),
        widget.is_24_format(),
    ));

    widget.run()?;
    Ok(())
}

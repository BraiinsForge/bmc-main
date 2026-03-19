// Copyright (C) 2026  Braiins Systems s.r.o.

mod virtual_display;

use bmc_nix_init::InitWindow;
use virtual_display::VirtualDisplayPlatform;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let platform = VirtualDisplayPlatform::new(480, 272)?;
    slint::platform::set_platform(Box::new(platform)).expect("BUG: failed to set Slint platform");

    let window = InitWindow::new().expect("BUG: failed to create init window");
    window.set_status_text("Initializing...".into());

    slint::run_event_loop().expect("BUG: slint event loop failed");

    Ok(())
}

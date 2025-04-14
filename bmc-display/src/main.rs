// Copyright (C) 2025  Braiins Systems s.r.o.
use bmc_display as _;
use flume as _;
use tracing as _;

use anyhow::Result;

#[allow(warnings)]
mod generated {
    slint::include_modules!();
}

fn main() -> Result<()> {
    #[cfg(feature = "standalone")]
    run_display()?;

    Ok(())
}

#[cfg(feature = "standalone")]
fn run_display() -> Result<()> {
    use bmc_display::virtual_display::VirtualDisplay;
    use slint::ComponentHandle;

    let (main_window, _) = VirtualDisplay::create()?;

    main_window.run()?;
    Ok(())
}

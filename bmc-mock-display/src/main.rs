// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_display::display_driver::DisplayHandle;
use bmc_mock_display::VirtualDisplay;
use slint::ComponentHandle;
use tracing as _;

fn main() -> Result<()> {
    let (main_window, display_driver) = VirtualDisplay::create()?;

    display_driver.init()?;

    main_window.run()?;
    Ok(())
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_mock_display::VirtualDisplay;
use slint::ComponentHandle;

fn main() -> Result<()> {
    let (main_window, _) = VirtualDisplay::create()?;

    main_window.run()?;
    Ok(())
}

// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_overlay_device_info::DeviceInfoOverlay;
use bmc_system_overlay::run_standalone;

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(DeviceInfoOverlay::default()))
}

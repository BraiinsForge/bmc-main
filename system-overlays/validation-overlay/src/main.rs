// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_system_overlay::{ValidationOverlay, run_standalone};

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(ValidationOverlay::default()))
}

// Copyright (C) 2026  Braiins Systems s.r.o.

use anyhow::Result;
use clap::Parser as _;

use bmc_wasm_thin::args::{Config, RawArgs};

fn main() {
    if let Err(e) = real_main() {
        tracing::error!("{e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    bmc_wasm_thin::logging::init();

    let raw = RawArgs::parse();
    let config = Config::from_raw(raw)?;
    bmc_wasm_thin::run(config)
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_core::{BmcManager, Configuration, log};
use clap::Parser;

mod cli;
mod mockfs;

pub fn init() -> Result<(impl BmcManager, Configuration)> {
    log::init();

    let config = cli::Config::parse();

    let mockfs = mockfs::MockFs::new(&config.mockfs_path);
    mockfs.init()?;

    let config = config.into();

    Ok((MockManager, config))
}

pub fn get_manager() -> impl BmcManager {
    MockManager
}

struct MockManager;

impl BmcManager for MockManager {
    fn version(&self) -> String {
        "Hello from Mock".to_string()
    }
}

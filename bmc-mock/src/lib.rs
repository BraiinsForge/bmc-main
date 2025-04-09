// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use async_trait::async_trait;
use bmc::entry::Initializer;
use bmc::{BmcManager, Configuration, log};
use clap::Parser;

mod cli;
mod mockfs;

pub struct MockInitializer;

#[async_trait]
impl Initializer for MockInitializer {
    async fn initialize(self) -> Result<(impl BmcManager, Configuration)> {
        log::init();

        let config = cli::Config::parse();

        let mockfs = mockfs::MockFs::new(&config.mockfs_path);
        mockfs.init()?;

        let config = config.into();

        Ok((MockManager, config))
    }
}

pub struct MockManager;

impl BmcManager for MockManager {
    fn version(&self) -> String {
        "Hello from Mock".to_string()
    }
}

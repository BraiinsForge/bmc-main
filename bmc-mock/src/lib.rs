// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow as _;
use anyhow::Result;
use bmc::BmcManager;
use bmc::entry::Initializer;
use bmc::{BmcManager, Configuration, log, session as bmcSession};
use bmc_display as _;
use bmc_mock_display as _;
use clap::Parser;
use slint as _;
use tokio as _;

mod cli;
mod mockfs;
mod session;
pub struct MockInitializer;

const DEFAULT_PASSWORD: &str = "root";

impl Initializer for MockInitializer {
    async fn initialize(
        self,
    ) -> Result<(impl BmcManager, Configuration, impl bmcSession::Manager)> {
        log::init();

        let config = cli::Config::parse();

        let mockfs = mockfs::MockFs::new(&config.mockfs_path);
        mockfs.init()?;

        let config = config.into();

        let session_manager = session::MockSessionManager::new(DEFAULT_PASSWORD.to_string());

        Ok((MockManager, config, session_manager))
    }
}

#[derive(Debug)]
pub struct MockManager;

impl BmcManager for MockManager {
    fn version(&self) -> String {
        "Hello from Mock".to_owned()
    }
}

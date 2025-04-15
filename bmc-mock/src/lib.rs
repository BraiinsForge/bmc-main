// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow as _;
use anyhow::Result;
use bmc::BmcManager;
use bmc::entry::Initializer;
use bmc::{BmcManager, Configuration, log, session as bmcSession};
use bmc_display as _;
use bmc_mock_display as _;
use clap::Parser;
pub use session::MockSessionManager;
use slint as _;
use std::sync::Arc;
use tokio as _;
use tracing::info;

mod cli;
mod mockfs;
mod session;
pub struct MockInitializer;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Initializer for MockInitializer {
    async fn initialize(self) -> Result<(impl BmcManager, Configuration)> {
        log::init();

        let config = cli::Config::parse();

        let mockfs = mockfs::MockFs::new(&config.mockfs_path);
        mockfs.init()?;

        let config = config.into();

        let session_manager = session::MockSessionManager::new(DEFAULT_PASSWORD.to_string());

        Ok((
            MockManager {
                session_manager: Arc::new(session_manager),
            },
            config,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct MockManager {
    pub session_manager: MockSessionManager,
}

impl BmcManager for MockManager {
    type Error = Error;
    type SessionManager = MockSessionManager;

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    fn version(&self) -> String {
        format!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Setting password to {:?}", password);
        Ok(())
    }
}

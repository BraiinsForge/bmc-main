// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow as _;
use anyhow::Result;
use bmc::BmcManager;
use bmc_display as _;
use bmc_mock_display as _;
pub use session::MockSessionManager;
use slint as _;
use tokio as _;
use tokio as _;
use tracing::info;

mod cli;
mod mockfs;
mod session;

pub use mockfs::MockFs;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct MockManager {
    pub session_manager: MockSessionManager,
}

#[async_trait::async_trait]
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

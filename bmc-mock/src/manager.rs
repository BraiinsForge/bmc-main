// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc::time::Timezone;
use bmc_platform::BmcPlatform;
use std::{path::Path, time::Duration};
use tracing::info;

use crate::{MockSessionManager, mockfs::MockFs};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Manager {
    mockfs: MockFs,
    pub session_manager: MockSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
}

impl Manager {
    #[must_use]
    pub fn new(mockfs: MockFs, session_manager: MockSessionManager) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(Timezone::default());
        Self {
            mockfs,
            session_manager,
            timezone_sender,
        }
    }
}

#[async_trait::async_trait]
impl bmc::BmcManager for Manager {
    type Error = Error;
    type SessionManager = MockSessionManager;

    fn version(&self) -> String {
        "0.1.0".to_owned()
    }

    fn platform(&self) -> BmcPlatform {
        BmcPlatform::BraiinsBmc
    }

    async fn upgrade(&self, keep_settings: bool, _upgrade_image_path: &Path) -> anyhow::Result<()> {
        info!(
            "Performing system upgrade (keep_settings={})...",
            keep_settings
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(())
    }

    async fn check_and_remove_upgrade_marker(&self) -> bool {
        self.mockfs.upgrade_result().exists()
    }

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Setting password to {:?}", password);

        Ok(())
    }

    fn timezone(&self) -> Timezone {
        self.timezone_sender.borrow().clone()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()> {
        self.timezone_sender.send_if_modified(|current| {
            if *current != timezone {
                *current = timezone;
                return true;
            }
            false
        });

        Ok(())
    }

    fn watch_timezone_updates(&self) -> tokio::sync::watch::Receiver<Timezone> {
        self.timezone_sender.subscribe()
    }
}

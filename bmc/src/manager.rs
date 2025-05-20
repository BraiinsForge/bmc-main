// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{fmt::Debug, path::Path};

use bmc_platform::BmcPlatform;
use tokio::sync::watch;

use crate::time::Timezone;

#[async_trait::async_trait]
pub trait BmcManager: Sync + Send + 'static + Debug {
    type SessionManager: crate::session::Manager;
    type Error: std::error::Error + Send + Sync;

    fn version(&self) -> String;

    fn platform(&self) -> BmcPlatform;

    async fn upgrade(&self, keep_settings: bool, upgrade_image_path: &Path) -> anyhow::Result<()>;

    // Checks if a system upgrade was performed
    async fn check_and_remove_upgrade_marker(&self) -> bool;

    fn session_manager(&self) -> Self::SessionManager;

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error>;

    fn timezone(&self) -> Timezone;

    fn timezone_list(&self) -> impl Iterator<Item = Timezone> {
        Timezone::timezone_list()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()>;

    fn watch_timezone_updates(&self) -> watch::Receiver<Timezone>;
}

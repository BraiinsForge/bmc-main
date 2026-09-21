// Copyright (C) 2026  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

//! Test doubles shared by the unit tests: a [`BmcManager`] whose every method
//! panics, for code under test that must hold a manager without consulting it.

use std::path::Path;

use axum_extra::extract::cookie::Cookie;
use bmc_platform::{BosPlatform, BosVersion};
use bmc_shared_time::time::Timezone;
use tokio::sync::watch;

use crate::bootloader_config::BootloaderConfig;
use crate::manager::{BmcManager, UpgradeError, UpgradeMarker};
use crate::session;

const UNREACHABLE: &str = "BUG: the test stub must not be consulted by the code under test";

#[derive(Debug, Clone)]
pub(crate) struct StubSession;

impl session::Handle for StubSession {
    fn is_valid(&self) -> bool {
        unimplemented!("{UNREACHABLE}")
    }
    fn id(&self) -> String {
        unimplemented!("{UNREACHABLE}")
    }
}

#[derive(Debug, Default)]
pub(crate) struct StubSessionManager;

#[async_trait::async_trait]
impl session::Manager for StubSessionManager {
    type Error = std::io::Error;
    type Session = StubSession;
    const SESSION_TIMEOUT: u32 = 0;

    async fn login(&self, _password: &str) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn logout(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn logout_all_related(&self, _session: Self::Session) -> Result<(), Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn extend(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn find(&self, _cookies: &[Cookie<'_>]) -> Result<Self::Session, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
}

#[derive(Debug)]
pub(crate) struct StubManager;

#[async_trait::async_trait]
impl BmcManager for StubManager {
    type SessionManager = StubSessionManager;
    type Error = std::io::Error;

    async fn version(&self) -> Option<BosVersion> {
        Some(BosVersion::new(&"current", &"current"))
    }
    fn platform(&self) -> BosPlatform {
        BosPlatform::Bmc1
    }
    async fn upgrade(
        &self,
        _keep_settings: bool,
        _upgrade_image_path: &Path,
        _progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn consume_upgrade_marker(&self) -> UpgradeMarker {
        unimplemented!("{UNREACHABLE}")
    }
    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
        unimplemented!("{UNREACHABLE}")
    }
    fn session_manager(&self) -> Self::SessionManager {
        unimplemented!("{UNREACHABLE}")
    }
    async fn check_password(&self, _password: Option<&str>) -> Result<bool, Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn set_password(&self, _password: Option<String>) -> Result<(), Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    fn timezone(&self) -> Timezone {
        unimplemented!("{UNREACHABLE}")
    }
    fn publish_timezone(&self, _timezone: Timezone) -> bool {
        unimplemented!("{UNREACHABLE}")
    }
    async fn set_timezone(&self, _timezone: Timezone) -> anyhow::Result<()> {
        unimplemented!("{UNREACHABLE}")
    }
    fn watch_timezone_updates(&self) -> watch::Receiver<Timezone> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn factory_reset(&self, _hard: bool) -> Result<(), Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn reboot(&self) -> anyhow::Result<()> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn handle_graceful_shutdown(&self) {
        unimplemented!("{UNREACHABLE}")
    }
    fn support_archive(&self) -> impl tokio::io::AsyncRead + Send + Unpin + 'static {
        unimplemented!("{UNREACHABLE}");
        #[expect(
            unreachable_code,
            reason = "stub panics on use; the value only pins the RPIT type"
        )]
        return tokio::io::empty();
    }
    async fn sync_boot_environment(&self, _config: &BootloaderConfig) -> Result<(), Self::Error> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn control_service(&self, _service: &str, _actions: &[&str]) -> anyhow::Result<()> {
        unimplemented!("{UNREACHABLE}")
    }
    fn network_manager(&self) -> &dyn bmc_net::NetworkManager {
        unimplemented!("{UNREACHABLE}")
    }
}

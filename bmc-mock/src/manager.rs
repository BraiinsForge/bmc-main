// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
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
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use crate::pacing::UpgradePacing;
use crate::{MockSessionManager, mockfs::MockFs};
use bmc::bootloader_config::BootloaderConfig;
use bmc::manager::{UpgradeError, UpgradeMarker, consume_upgrade_marker};
use bmc_net::NetworkManager;
use bmc_net::mock::MockNetworkManager;
use bmc_nix::progress::{ActiveDownload, ProgressEvent};
use bmc_platform::{BosPlatform, BosVersion};
use bmc_shared_time::time::Timezone;
use bmc_support::{PlainZip, SupportArchive};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::signal;
use tracing::info;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Support(#[from] anyhow::Error),
}

#[derive(Debug)]
#[expect(
    clippy::struct_field_names,
    reason = "the *_manager fields name the subsystem they own; renaming them would be less clear"
)]
pub struct Manager {
    mockfs: MockFs,
    platform: BosPlatform,
    pub session_manager: MockSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    password: Arc<Mutex<Option<String>>>,
    network_manager: Arc<MockNetworkManager>,
    pacing: UpgradePacing,
    stop: Arc<tokio::sync::Notify>,
}

impl Manager {
    const DUMMY_SUPPORT_FILE_NAME: &'static str = "hello_deck.txt";
    const DUMMY_SUPPORT_FILE_CONTENT: &'static str = "wake up Neo";

    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the mock manager wires up independent test doubles; a wrapper \
                  struct just to satisfy the lint would not aid the test harness"
    )]
    pub fn new(
        mockfs: MockFs,
        session_manager: MockSessionManager,
        password: Arc<Mutex<Option<String>>>,
        platform: BosPlatform,
        pacing: UpgradePacing,
        stop: Arc<tokio::sync::Notify>,
        factory_default: bool,
        setup_pending: bool,
        port: u16,
    ) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(Timezone::default());
        Self {
            mockfs,
            platform,
            session_manager,
            timezone_sender,
            password,
            network_manager: Arc::new(
                MockNetworkManager::with_provisioning(factory_default, setup_pending)
                    // The mock serves the captive portal from its own HTTP
                    // listener, so the redirect must point back at it.
                    .with_captive_portal_host(format!("localhost:{port}")),
            ),
            pacing,
            stop,
        }
    }
}

#[async_trait::async_trait]
impl bmc::BmcManager for Manager {
    type Error = Error;
    type SessionManager = MockSessionManager;

    async fn version(&self) -> Option<BosVersion> {
        Some(BosVersion::new(&25, &7))
    }

    fn platform(&self) -> BosPlatform {
        self.platform
    }

    fn network_manager(&self) -> &dyn NetworkManager {
        self.network_manager.as_ref()
    }

    async fn upgrade(
        &self,
        keep_settings: bool,
        _upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        info!(
            "Performing system upgrade (keep_settings={})...",
            keep_settings
        );
        if let Some(progress) = progress {
            let total_bytes = 4_000_000;
            let mut lines = vec![
                ProgressEvent::Phase {
                    phase: "realizing".to_owned(),
                }
                .to_bmc_line(),
                ProgressEvent::RealizationStarted { total_paths: 3 }.to_bmc_line(),
            ];
            // Walk the download across chunks so the client sees real
            // progression toward total_bytes, not a single frozen frame.
            for downloaded_bytes in (1_000_000..=total_bytes).step_by(1_000_000) {
                lines.push(
                    ProgressEvent::Download {
                        downloaded_bytes,
                        total_bytes: Some(total_bytes),
                        remaining_bytes: Some(total_bytes - downloaded_bytes),
                        active: vec![ActiveDownload {
                            store_path: Some("/nix/store/mock-core".to_owned()),
                            source: Some("mock://packages/core".to_owned()),
                            downloaded_bytes,
                            total_bytes: Some(total_bytes),
                        }],
                    }
                    .to_bmc_line(),
                );
            }
            // The firmware-time package run stages next-boot activation, so it
            // stops at building; activation happens after the reboot.
            lines.extend([
                ProgressEvent::RealizationFinished.to_bmc_line(),
                ProgressEvent::Phase {
                    phase: "verifying".to_owned(),
                }
                .to_bmc_line(),
                ProgressEvent::Phase {
                    phase: "building".to_owned(),
                }
                .to_bmc_line(),
            ]);
            for line in lines {
                _ = progress.send(line);
                tokio::time::sleep(self.pacing.progress_step()).await;
            }
        }
        if crate::scenario::read(&self.mockfs.upgrade_scenario()).run
            == crate::scenario::RunScenario::ApplyFail
        {
            return Err(UpgradeError::Failed(
                "mock: firmware apply failed".to_owned(),
            ));
        }

        tokio::time::sleep(self.pacing.sysupgrade_duration()).await;

        crate::scenario::consume_pending_install(
            &self.mockfs.pending_install(),
            &self.mockfs.upgrade_scenario(),
        );

        let reboot_delay = self.pacing.shutdown_delay();
        tokio::spawn(async move {
            tokio::time::sleep(reboot_delay).await;
            info!("Mock sysupgrade: exiting to simulate the reboot");
            std::process::exit(0);
        });

        Ok(())
    }

    async fn consume_upgrade_marker(&self) -> UpgradeMarker {
        consume_upgrade_marker(&self.mockfs.upgrade_result()).await
    }

    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
        let Some(marker) = self.mockfs.service_upgrade_marker() else {
            return UpgradeMarker::Absent;
        };
        consume_upgrade_marker(&marker).await
    }

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error> {
        let current_password = self.password.lock().expect("BUG: cannot lock password");

        let matches = match (password, current_password.as_deref()) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(password), Some(current_password)) => password == current_password,
        };

        Ok(matches)
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        info!("Setting password to {:?}", password);

        let mut guard = self.password.lock().expect("BUG: cannot lock password");
        *guard = password;

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

    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error> {
        info!(hard, "Performing factory reset...");
        Ok(())
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        info!("Performing reboot...");
        Ok(())
    }

    async fn handle_graceful_shutdown(&self) {
        // The notifier models the point where bmc-openwrt receives procd's
        // SIGTERM from the external service orchestrator; the mock never
        // signals itself, it just runs the same graceful Axum shutdown path.
        tokio::select! {
            result = signal::ctrl_c() => {
                _ = result;
                info!("Shutdown signal received");
            }
            () = self.stop.notified() => {
                info!("Mock application stop requested");
            }
        }
    }

    async fn support_archive(&self) -> Result<Vec<u8>, Error> {
        info!("Support archive");
        let mut buf = Vec::new();
        let mut archive = SupportArchive::new(&mut buf, &PlainZip, false, &[]);
        archive.add_builtin(
            Self::DUMMY_SUPPORT_FILE_NAME,
            Self::DUMMY_SUPPORT_FILE_CONTENT,
        )?;
        archive.finish()?;
        Ok(buf)
    }

    async fn sync_boot_environment(&self, config: &BootloaderConfig) -> Result<(), Self::Error> {
        info!(?config, "Bootloader config sync (no-op in mock)");
        Ok(())
    }
}

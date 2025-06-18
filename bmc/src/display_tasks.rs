// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::system_upgrade::SystemUpgradeState;
use bmc_display::data::Screen;
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::interval;
use tracing::info;

#[derive(Debug)]
pub(crate) struct DisplayTasks {
    display_controller: DisplayController,
    system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
    timezone_receiver: watch::Receiver<Timezone>,
}

impl DisplayTasks {
    pub(crate) fn new(
        display_controller: DisplayController,
        system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
        timezone_receiver: watch::Receiver<Timezone>,
    ) -> Self {
        Self {
            display_controller,
            system_upgrade_receiver,
            timezone_receiver,
        }
    }

    pub(crate) fn spawn(self) {
        let Self {
            display_controller,
            system_upgrade_receiver,
            timezone_receiver,
        } = self;

        tokio::spawn(Self::run_system_upgrade_listener(
            display_controller.clone(),
            system_upgrade_receiver,
        ));

        tokio::spawn(Self::run_timezone_listener(timezone_receiver));

        tokio::spawn(Self::run_date_time_update(display_controller.clone()));
    }

    async fn run_system_upgrade_listener(
        display_controller: DisplayController,
        mut receiver: watch::Receiver<Option<SystemUpgradeState>>,
    ) {
        while let Ok(()) = receiver.changed().await {
            let Some(upgrade_state) = &*receiver.borrow_and_update() else {
                continue;
            };

            match upgrade_state {
                SystemUpgradeState::DownloadStarted { total_mb } => {
                    display_controller.update_download_firmware_progress(0.0, *total_mb);
                    display_controller.set_screen(Screen::DownloadFirmware);
                }
                SystemUpgradeState::DownloadProgress {
                    downloaded_mb,
                    total_mb,
                } => {
                    display_controller.update_download_firmware_progress(*downloaded_mb, *total_mb);
                }
                SystemUpgradeState::DownloadFinished { total_mb, .. } => {
                    display_controller.update_download_firmware_progress(*total_mb, *total_mb);
                }
                SystemUpgradeState::UpgradeStarted => {
                    display_controller.set_screen(Screen::Upgrade);
                }
                SystemUpgradeState::UpgradeFinished => {
                    display_controller.set_screen(Screen::UpgradeSuccess);
                }
                SystemUpgradeState::Failed => {
                    display_controller.set_screen(Screen::UpgradeFailed);
                }
            }
        }
    }

    async fn run_timezone_listener(mut receiver: watch::Receiver<Timezone>) {
        while let Ok(()) = receiver.changed().await {
            let timezone = receiver.borrow_and_update();
            info!(?timezone, "Timezone was changed");
        }
    }

    async fn run_date_time_update(display_controller: DisplayController) {
        let mut interval = interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            display_controller.update_datetime();
        }
    }
}

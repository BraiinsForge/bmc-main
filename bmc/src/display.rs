// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_display::display_driver::{
    DisplayBacklightDriver, DisplayDriver, DisplayEvent, DisplayHandle, DisplayHandler,
};
use data::DisplayDataProvider;
use tokio::{sync::watch::Receiver, task};

use crate::{
    system_upgrade::{StateService, SystemUpgradeState},
    time::Timezone,
};

pub(crate) mod data;

#[derive(Debug)]
pub(crate) struct DisplayTasks {
    _data_provider: DisplayDataProvider,
    display_handler: DisplayHandler,
    system_upgrade_receiver: Receiver<Option<SystemUpgradeState>>,
    timezone_watch: Receiver<Timezone>,
}

impl DisplayTasks {
    pub(crate) fn new<T: DisplayBacklightDriver>(
        display_driver: DisplayDriver<T>,
        state_service: StateService,
        timezone_watch: Receiver<Timezone>,
    ) -> Self {
        let system_upgrade_receiver = state_service.subscribe();
        let data_provider = DisplayDataProvider::new(state_service);
        let display_handler = DisplayHandler::new(display_driver, data_provider.clone());

        Self {
            _data_provider: data_provider,
            display_handler,
            system_upgrade_receiver,
            timezone_watch,
        }
    }

    pub(crate) fn spawn(&self) {
        let mut receiver = self.system_upgrade_receiver.clone();
        let display_handle = self.display_handler.clone();

        task::spawn(async move {
            loop {
                if receiver.changed().await.is_ok() {
                    let state = (*receiver.borrow()).clone();
                    if let Some(upgrade_status) = state {
                        match upgrade_status {
                            SystemUpgradeState::DownloadStarted => {
                                display_handle
                                    .emit_event(DisplayEvent::DownloadStarted)
                                    .await;
                            }
                            SystemUpgradeState::DownloadProgress { .. }
                            | SystemUpgradeState::DownloadFinished(_) => (),

                            SystemUpgradeState::UpgradeStarted => {
                                display_handle
                                    .emit_event(DisplayEvent::UpgradeStarted)
                                    .await;
                            }
                            SystemUpgradeState::UpgradeFinished => {
                                display_handle
                                    .emit_event(DisplayEvent::UpgradeFinishedSuccessfully)
                                    .await;
                            }
                            SystemUpgradeState::Failed => {
                                display_handle.emit_event(DisplayEvent::UpgradeFailed).await;
                            }
                        }
                    }
                }
            }
        });

        let display_handle = self.display_handler.clone();
        let mut timezone_watch = self.timezone_watch.clone();

        task::spawn(async move {
            loop {
                if timezone_watch.changed().await.is_ok() {
                    display_handle
                        .emit_event(DisplayEvent::TimezoneChanged)
                        .await;
                }
            }
        });
    }
}

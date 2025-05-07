// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_display::data_provider::{DataProvider, DownloadProgress};
use tokio::task;

use crate::system_upgrade::StateService;

use super::SystemUpgradeState;

#[derive(Clone, Debug)]
pub(crate) struct DisplayDataProvider {
    state_service: StateService,
}

impl DisplayDataProvider {
    pub(crate) fn new(state_service: StateService) -> Self {
        Self { state_service }
    }
}

impl DataProvider for DisplayDataProvider {
    fn get_download_firmware_screen_data(
        &self,
    ) -> bmc_display::data_provider::DownloadFirmwareScreenData {
        let mut receiver = self.state_service.subscribe();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        task::spawn(async move {
            loop {
                if receiver.changed().await.is_ok() {
                    let state = (*receiver.borrow_and_update()).clone();
                    match state {
                        Some(SystemUpgradeState::DownloadProgress {
                            downloaded_mb,
                            total_mb,
                        }) => {
                            _ = tx.send(DownloadProgress {
                                downloaded_mb,
                                total_mb,
                            });
                        }
                        _ => return,
                    }
                }
            }
        });

        bmc_display::data_provider::DownloadFirmwareScreenData {
            progress_receiver: rx,
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use tokio::sync::mpsc::Sender;
use tracing::error;

use crate::system_upgrade::{StateService, SystemUpgradeState};
use bmc_led::{
    data::{LedCommand, LedEvent},
    led_driver::LedEventHandler,
};
use tokio::{sync::watch::Receiver, task};

#[derive(Debug)]
pub(crate) struct LedController {
    led_event_handler: LedEventHandler,
    system_upgrade_receiver: Receiver<Option<SystemUpgradeState>>,
}

impl LedController {
    pub(crate) fn new(state_service: &StateService) -> Self {
        let system_upgrade_receiver = state_service.subscribe();

        Self {
            led_event_handler: LedEventHandler,
            system_upgrade_receiver,
        }
    }

    pub(crate) fn init(&self, led_cmd_tx: Sender<LedCommand>) {
        let led_event_tx = self.led_event_handler.init(led_cmd_tx);
        let mut receiver = self.system_upgrade_receiver.clone();

        task::spawn(async move {
            loop {
                if receiver.changed().await.is_ok() {
                    let state = (*receiver.borrow()).clone();
                    if let Some(upgrade_status) = state {
                        let led_event = match upgrade_status {
                            SystemUpgradeState::DownloadStarted { .. } => LedEvent::DownloadStarted,
                            SystemUpgradeState::DownloadProgress { .. } => {
                                LedEvent::DownloadProgress
                            }
                            SystemUpgradeState::DownloadFinished { .. } => {
                                LedEvent::DownloadFinished
                            }
                            SystemUpgradeState::UpgradeStarted => LedEvent::UpgradeStarted,
                            SystemUpgradeState::Failed => LedEvent::Failed,
                        };

                        // Ignore the result, since we don't care if the send fails
                        if let Err(e) = led_event_tx.send(led_event).await {
                            error!("Failed to send command: {}", e);
                        }
                    }
                }
            }
        });
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use tokio::sync::mpsc::Sender;

use crate::system_upgrade::{StateService, SystemUpgradeState};
use anyhow::Result;
use bmc_led::{
    data::{LedCommand, LedEvent},
    led_driver::{LedDriver, LedHandle, LedHandler},
};
use bmc_shared_time::time::Timezone;
use tokio::{sync::watch::Receiver, task};

#[derive(Debug)]
pub(crate) struct LedController {
    led_handler: LedHandler,
    system_upgrade_receiver: Receiver<Option<SystemUpgradeState>>,
    timezone_watch: tokio::sync::watch::Receiver<Timezone>,
}

impl LedController {
    pub(crate) fn new(
        _led_driver: LedDriver,
        led_cmd_tx: Sender<LedCommand>,
        state_service: &StateService,
        timezone_watch: tokio::sync::watch::Receiver<Timezone>,
    ) -> Self {
        let system_upgrade_receiver = state_service.subscribe();
        let led_handler = LedHandler::new(led_cmd_tx);

        Self {
            led_handler,
            system_upgrade_receiver,
            timezone_watch,
        }
    }

    pub(crate) fn init(&self) -> Result<()> {
        self.led_handler.init()?;

        let mut receiver = self.system_upgrade_receiver.clone();
        let led_handle = self.led_handler.clone();

        task::spawn(async move {
            loop {
                if receiver.changed().await.is_ok() {
                    let state = (*receiver.borrow()).clone();
                    if let Some(upgrade_status) = state {
                        match upgrade_status {
                            SystemUpgradeState::DownloadStarted { .. } => {
                                led_handle.emit_event(LedEvent::DownloadStarted).await;
                            }
                            SystemUpgradeState::DownloadProgress { .. } => {
                                led_handle.emit_event(LedEvent::DownloadProgress).await;
                            }
                            SystemUpgradeState::DownloadFinished { .. } => {
                                led_handle.emit_event(LedEvent::DownloadFinished).await;
                            }
                            SystemUpgradeState::UpgradeStarted => {
                                led_handle
                                    .emit_event(LedEvent::UpgradeFinishedSuccessfully)
                                    .await;
                            }
                            SystemUpgradeState::Failed => {
                                led_handle.emit_event(LedEvent::UpgradeFailed).await;
                            }
                        }
                    }
                }
            }
        });

        let led_handle: LedHandler = self.led_handler.clone();
        let mut timezone_watch = self.timezone_watch.clone();

        task::spawn(async move {
            loop {
                if timezone_watch.changed().await.is_ok() {
                    led_handle.emit_event(LedEvent::TimezoneChanged).await;
                }
            }
        });

        Ok(())
    }
}

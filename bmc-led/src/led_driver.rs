// Copyright (C) 2025  Braiins Systems s.r.o.

use super::data;
use crate::config;
use crate::config::LED_COLOR_CYAN;
use crate::config::LED_COLOR_MAGENTA;
use crate::config::LED_COLOR_RED;
use crate::config::LED_COLOR_WARM_WHITE;
use crate::data::LedCommand;
use std::fmt::Debug;
use tokio::sync::mpsc::Sender;
use tracing::error;

const EVENT_BUFFER_SIZE: usize = 4;

#[derive(Debug)]
pub struct LedDriver {
    pub command_sender: Sender<LedCommand>,
}

pub trait LedDriverFactory {
    #[must_use]
    fn new(device_path: &str) -> Self;
}

impl LedDriver {
    pub fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(()) // TODO: Implement once new BMC board config is available #BOS-3299
    }

    pub fn state(&self) -> anyhow::Result<bool> {
        Ok(true) // TODO: Implement once new BMC board config is available #BOS-3299
    }

    pub fn toggle_state(&mut self) -> anyhow::Result<()> {
        self.state().and_then(|state| self.change_state(!state))
    }

    pub fn turn_on(&self) -> anyhow::Result<()> {
        self.change_state(true)
    }

    pub fn turn_off(&self) -> anyhow::Result<()> {
        self.change_state(false)
    }

    pub fn brightness(&self) -> anyhow::Result<f32> {
        Ok(1.0) // TODO: Implement once new BMC board config is available #BOS-3299
    }

    #[must_use]
    pub fn max_brightness(&self) -> f32 {
        config::LED_MAX_BRIGHTNESS
    }

    pub fn set_brightness(&self, _value: f32) -> anyhow::Result<()> {
        Ok(()) // TODO: Implement once new BMC board config is available #BOS-3299
    }
}

#[derive(Debug, Default)]
pub struct LedEventHandler;

impl LedEventHandler {
    #[must_use]
    pub fn init(&self, command_sender: Sender<LedCommand>) -> Sender<data::LedEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let command = match event {
                    data::LedEvent::Idle | data::LedEvent::DownloadFinished => {
                        Some(LedCommand::SetPersistentEffect(
                            data::LedEffect::Fireflies,
                            LED_COLOR_MAGENTA,
                        ))
                    }
                    data::LedEvent::Alarm => Some(LedCommand::SetPersistentEffect(
                        data::LedEffect::Scan,
                        LED_COLOR_WARM_WHITE,
                    )),
                    data::LedEvent::DownloadStarted => Some(LedCommand::SetPersistentEffect(
                        data::LedEffect::Snake,
                        LED_COLOR_CYAN,
                    )),
                    data::LedEvent::UpgradeStarted => Some(LedCommand::SetPersistentEffect(
                        data::LedEffect::Chase,
                        LED_COLOR_CYAN,
                    )),
                    data::LedEvent::Failed => Some(LedCommand::SetPersistentEffect(
                        data::LedEffect::Fireflies,
                        LED_COLOR_RED,
                    )),
                    data::LedEvent::DownloadProgress => None,
                    data::LedEvent::Enable => Some(LedCommand::Enable),
                    data::LedEvent::Disable => Some(LedCommand::Disable),
                };

                if let Some(state) = command {
                    // Ignore the result, since we don't care if the send fails
                    if let Err(e) = command_sender.send(state).await {
                        error!("Failed to send command: {:?}, error {e} occured", state);
                    }
                }
            }
        });

        sender
    }
}

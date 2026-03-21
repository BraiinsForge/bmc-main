// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_led::{
    data::LedCommand,
    led_driver::{LedDriver, LedDriverFactory},
};
use tracing::info;

const COMMAND_BUFFER_SIZE: usize = 4;

#[derive(Debug)]
pub struct PlatformLedDriver(pub LedDriver);

impl LedDriverFactory for PlatformLedDriver {
    fn new(_device_path: &str) -> Self {
        let (command_sender, mut command_receiver) =
            tokio::sync::mpsc::channel::<LedCommand>(COMMAND_BUFFER_SIZE);

        tokio::spawn(async move {
            loop {
                if let Some(command) = command_receiver.recv().await {
                    info!("Received LED command: {:?}", command);
                }
            }
        });

        PlatformLedDriver(LedDriver { command_sender })
    }
}

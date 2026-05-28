// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::data::LedCommand;
use crate::led_driver::{LedDriver, LedDriverFactory};
use tokio::sync::mpsc::Receiver;

const COMMAND_BUFFER_SIZE: usize = 16;

/// Discards LED commands so stripless platforms don't fail widget LED requests.
#[derive(Debug)]
pub struct DisabledLedDriver(pub LedDriver);

impl LedDriverFactory for DisabledLedDriver {
    fn new(_device_path: &str) -> Self {
        let (command_sender, command_receiver) =
            tokio::sync::mpsc::channel::<LedCommand>(COMMAND_BUFFER_SIZE);
        tokio::spawn(disabled_worker(command_receiver));
        DisabledLedDriver(LedDriver { command_sender })
    }
}

async fn disabled_worker(mut led_cmd_rx: Receiver<LedCommand>) {
    while led_cmd_rx.recv().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use crate::data::{LedCommand, LedEffect, LedScene};
    use crate::led_driver::LedDriverFactory;

    #[tokio::test]
    async fn disabled_loop_accepts_commands_without_failing() {
        let driver = super::DisabledLedDriver::new("/dev/null");
        for _ in 0..32 {
            driver
                .0
                .command_sender
                .send(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::None,
                    period: None,
                    duration: None,
                }))
                .await
                .expect("BUG: disabled LED loop must keep accepting commands");
        }
    }
}

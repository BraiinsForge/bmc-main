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

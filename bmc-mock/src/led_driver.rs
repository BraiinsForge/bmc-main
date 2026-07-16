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

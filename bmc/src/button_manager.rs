// Copyright (C) 2025 Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

// Button manager taken from BOS

use crate::manager::BmcManager;
use bmc_button::{ButtonEvent, ButtonId, Buttons};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_stream::StreamExt;
use tracing::info;
use tracing::log::warn;

/// Maximum number of seconds to hold the button for to perform reboot
const REBOOT_MAX_HOLD_DURATION: Duration = Duration::from_secs(5);
/// Maximum number of seconds to hold the button for to perform factory reset
const FRESET_MAX_HOLD_DURATION: Duration = Duration::from_secs(10);
/// Maximum number of seconds to hold the button for to perform hard factory reset
const HARD_FRESET_MAX_HOLD_DURATION: Duration = Duration::from_secs(15);

/// Button current state enum up/down, with holding time
#[derive(Clone, Debug)]
pub enum ButtonState {
    Up { released: Instant },
    Down { pressed: Instant },
}

pub struct ButtonManager<T>
where
    T: BmcManager,
{
    pub buttons: Arc<Box<dyn Buttons + Send + Sync>>,
    pub state: HashMap<ButtonId, ButtonState>,
    pub bmc_manager: Arc<T>,
}

impl<T> std::fmt::Debug for ButtonManager<T>
where
    T: BmcManager + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ButtonManager")
            .field("buttons", &self.buttons)
            .field("state", &self.state)
            .field("bmc_manager", &self.bmc_manager)
            .finish()
    }
}

impl<T> ButtonManager<T>
where
    T: BmcManager,
{
    /// Creates a new `ButtonManager` with the given buttons trait
    pub fn new(buttons: Arc<Box<dyn Buttons + Send + Sync>>, bmc_manager: Arc<T>) -> Self {
        Self {
            buttons,
            state: HashMap::new(),
            bmc_manager,
        }
    }

    pub async fn run(mut self) {
        self.manage_buttons().await;
    }

    /// Main function to poll the button events and make actions
    pub async fn manage_buttons(&mut self) {
        // Initialize the state of all buttons to `Up`
        self.state.insert(
            ButtonId::Reset,
            ButtonState::Up {
                released: Instant::now(),
            },
        );

        let mut stream = self
            .buttons
            .to_stream()
            .expect("BUG: Can't create button stream");

        while let Some(event) = stream.next().await {
            info!("New button event: {:?}", event);
            match event {
                Ok(ButtonEvent::Pressed(button)) => {
                    if let ButtonState::Up { released } = self.state[&button] {
                        released
                    } else {
                        warn!("Button pressed without being released: {button:?}");
                        Instant::now()
                    };
                    self.state.insert(
                        button.clone(),
                        ButtonState::Down {
                            pressed: Instant::now(),
                        },
                    );
                }
                Ok(ButtonEvent::Released(button)) => {
                    if let ButtonState::Down { pressed } = self.state[&button] {
                        match &button {
                            ButtonId::Reset => {
                                self.handle_reset_button(pressed).await;
                            }
                        }
                    } else {
                        warn!("Button released without being pressed: {button:?}");
                    }
                    self.state.insert(
                        button.clone(),
                        ButtonState::Up {
                            released: Instant::now(),
                        },
                    );
                }
                Err(error) => warn!("Error while reading button event: {error}"),
            }
        }
    }

    /// Function to handle reset button
    pub async fn handle_reset_button(&self, pressed_at: Instant) {
        if pressed_at.elapsed() < REBOOT_MAX_HOLD_DURATION {
            info!("Rebooting the system");
            if let Err(e) = self.bmc_manager.reboot().await {
                warn!("Error while rebooting: {e}");
            }
        }
        if pressed_at.elapsed() >= REBOOT_MAX_HOLD_DURATION
            && pressed_at.elapsed() < FRESET_MAX_HOLD_DURATION
        {
            // Factory reset
            info!("Performing factory reset");
            if let Err(e) = self.bmc_manager.factory_reset(false).await {
                warn!("Error while factory reset: {e}");
            }
        }
        if pressed_at.elapsed() >= REBOOT_MAX_HOLD_DURATION
            && pressed_at.elapsed() < HARD_FRESET_MAX_HOLD_DURATION
        {
            // Hard factory reset
            info!("Performing hard factory reset");
            if let Err(e) = self.bmc_manager.factory_reset(true).await {
                warn!("Error while hard factory reset: {e}");
            }
        }
        if pressed_at.elapsed() >= HARD_FRESET_MAX_HOLD_DURATION {
            warn!(
                "Reset button pressed for more than {} seconds, ignoring",
                HARD_FRESET_MAX_HOLD_DURATION.as_secs()
            );
        }
    }
}

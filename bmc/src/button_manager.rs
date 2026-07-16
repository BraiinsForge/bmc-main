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

// Button manager taken from BOS

use crate::manager::BmcManager;
use bmc_button::{ButtonEvent, ButtonId, Buttons};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_stream::StreamExt;
use tracing::info;
use tracing::log::warn;

/// Maximum hold duration to trigger a reboot (0-2 seconds)
const REBOOT_MAX_HOLD_DURATION: Duration = Duration::from_secs(2);
/// Minimum hold duration to trigger a factory reset (5+ seconds)
const FACTORY_RESET_MIN_HOLD_DURATION: Duration = Duration::from_secs(5);

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
    pub screen_activity: Arc<tokio::sync::Notify>,
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
            .finish_non_exhaustive()
    }
}

impl<T> ButtonManager<T>
where
    T: BmcManager,
{
    /// Creates a new `ButtonManager` with the given buttons trait
    pub fn new(
        buttons: Arc<Box<dyn Buttons + Send + Sync>>,
        bmc_manager: Arc<T>,
        screen_activity: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            buttons,
            state: HashMap::new(),
            bmc_manager,
            screen_activity,
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
            let inner = match event {
                Ok(inner) => inner,
                Err(error) => {
                    warn!("Error while reading button event: {error}");
                    continue;
                }
            };
            self.screen_activity.notify_waiters();
            match inner {
                ButtonEvent::Pressed(button) => {
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
                ButtonEvent::Released(button) => {
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
            }
        }
    }

    /// Reset button has 2 roles: this function handles reboot and factory reset
    /// based on how long the button is held down.
    pub async fn handle_reset_button(&self, pressed_at: Instant) {
        let elapsed = pressed_at.elapsed();

        if elapsed <= REBOOT_MAX_HOLD_DURATION {
            info!("Rebooting the system");
            if let Err(e) = self.bmc_manager.reboot().await {
                warn!("Error while rebooting: {e}");
            }
        } else if elapsed >= FACTORY_RESET_MIN_HOLD_DURATION {
            info!("Performing factory reset");
            if let Err(e) = self.bmc_manager.factory_reset(false).await {
                warn!("Error while performing factory reset: {e}");
            }
        } else {
            info!(
                "Reset button pressed for {} seconds (between {}-{}s), ignoring",
                elapsed.as_secs(),
                REBOOT_MAX_HOLD_DURATION.as_secs(),
                FACTORY_RESET_MIN_HOLD_DURATION.as_secs()
            );
        }
    }
}

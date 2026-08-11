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

use std::{sync::Arc, time::Duration};

use bmc_shared_ii_net::wifi::SignalStrength;
use tokio::{
    sync::{self, mpsc::Sender, watch},
    time::{Instant, interval},
};
use tracing::{debug, error, warn};

use crate::{
    BmcManager,
    alarm::{AlarmBus, AlarmEvent},
    led_coordinator::{Layer, LedCoordinatorHandle},
    manager::WifiEvent,
    system_upgrade::{StateService, SystemUpgradeState},
};
use bmc_led::{
    data::{LedCommand, LedEvent},
    led_driver::LedIndicatorsState,
};
use tokio::task;

const EVENT_BUFFER_SIZE: usize = 4;

/// Far-future deadline used when no temp scene is active. `tokio::time::Sleep`
/// has no "never" state, so the select arm parks on a sleep this long instead.
const IDLE_EXPIRY: Duration = Duration::from_hours(8760);

/// Next instant the event loop must wake to re-resolve the system layer:
/// the active temp's deadline, or the idle far-future when no temp is set.
/// Reading the deadline from the state (rather than the resolved scene's
/// duration) keeps an unrelated event from re-arming past the temp window.
fn next_wakeup(state: &LedIndicatorsState) -> Instant {
    state
        .temp_deadline()
        .map_or_else(|| Instant::now() + IDLE_EXPIRY, Instant::from_std)
}

#[derive(Clone, Debug)]
pub enum LedState {
    Enabled,
    Disabled,
}

impl From<bool> for LedState {
    fn from(value: bool) -> Self {
        if value {
            LedState::Enabled
        } else {
            LedState::Disabled
        }
    }
}

#[derive(Debug)]
pub(crate) struct LedController<T>
where
    T: BmcManager,
{
    event_sender: Option<Sender<LedEvent>>,
    command_sender: Option<Sender<LedCommand>>,
    system_upgrade_receiver: sync::watch::Receiver<Option<SystemUpgradeState>>,
    manager: Arc<T>,
    last_price_change_24h_receiver: watch::Receiver<f32>,
    alarm_bus: AlarmBus,
}

impl<T: BmcManager> Clone for LedController<T> {
    fn clone(&self) -> Self {
        Self {
            event_sender: self.event_sender.clone(),
            command_sender: self.command_sender.clone(),
            system_upgrade_receiver: self.system_upgrade_receiver.clone(),
            manager: self.manager.clone(),
            last_price_change_24h_receiver: self.last_price_change_24h_receiver.clone(),
            alarm_bus: self.alarm_bus.clone(),
        }
    }
}

impl<T> LedController<T>
where
    T: BmcManager,
{
    pub(crate) fn new(
        state_service: &StateService,
        manager: Arc<T>,
        last_price_change_24h: watch::Receiver<f32>,
        led_enabled: bool,
        alarm_bus: AlarmBus,
    ) -> (Self, watch::Sender<LedState>, watch::Receiver<LedState>) {
        let system_upgrade_receiver = state_service.subscribe();

        let (state_sender, state_receiver) = watch::channel(led_enabled.into());

        let this = Self {
            event_sender: None,
            command_sender: None,
            system_upgrade_receiver,
            manager,
            last_price_change_24h_receiver: last_price_change_24h,
            alarm_bus,
        };

        (this, state_sender, state_receiver)
    }

    fn run_wifi_task(&self, led_event_tx: Sender<LedEvent>) {
        let manager = self.manager.clone();
        task::spawn(async move {
            let mut interval = interval(Duration::from_secs(2));
            let mut last_event = LedEvent::WifiNone;

            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                let mut new_event = last_event;

                let wifi_status = manager.wifi_status().await;
                if let Err(_e) = wifi_status {
                    new_event = LedEvent::WifiError;
                } else if let Ok(wifi_status) = wifi_status {
                    debug!("Current WiFi status: {:?}", wifi_status.status);
                    if let Some(link_state) = wifi_status.status.sta_link_state {
                        debug!("Current WiFi link state: {:?}", link_state);

                        match link_state.signal_strength() {
                            SignalStrength::Excellent
                            | SignalStrength::Fair
                            | SignalStrength::Low => {
                                new_event = LedEvent::WifiConnected;
                            }
                            SignalStrength::Offline => {
                                new_event = LedEvent::WifiError;
                            }
                        }
                    } else if let Some(_configuration) = wifi_status.status.configuration {
                        new_event = LedEvent::WifiConnecting;
                    } else {
                        new_event = LedEvent::WifiError;
                    }
                }

                if new_event != last_event {
                    debug!(
                        "WiFi LED event changed from {:?} to {:?}",
                        last_event, new_event
                    );
                    last_event = new_event;

                    if let Err(e) = led_event_tx.send(new_event).await {
                        error!("Failed to send LED event: {}", e);
                    }
                }

                interval.tick().await;
            }
        });
    }

    fn run_sysupgrade_task(&self, led_event_tx: Sender<LedEvent>) {
        let mut receiver = self.system_upgrade_receiver.clone();

        task::spawn(async move {
            loop {
                if receiver.changed().await.is_ok() {
                    let state = (*receiver.borrow()).clone();
                    if let Some(upgrade_status) = state
                        && let Some(led_event) = upgrade_state_to_led(&upgrade_status)
                    {
                        // Ignore the result, since we don't care if the send fails
                        if let Err(e) = led_event_tx.send(led_event).await {
                            error!("Failed to send command: {}", e);
                        }
                    }
                }
            }
        });
    }

    // TODO: Price alerts
    #[expect(dead_code)]
    fn run_price_task(&mut self, led_event_tx: Sender<LedEvent>) {
        let mut receiver = self.last_price_change_24h_receiver.clone();
        task::spawn(async move {
            let mut last_led_event = LedEvent::PriceUpEnded;

            loop {
                if receiver.changed().await.is_ok() {
                    let value = *receiver.borrow();

                    let led_event = if value > 0.0 {
                        if last_led_event == LedEvent::PriceDown {
                            LedEvent::PriceDownEnded
                        } else {
                            LedEvent::PriceUp
                        }
                    } else if value < 0.0 {
                        if last_led_event == LedEvent::PriceUp {
                            LedEvent::PriceUpEnded
                        } else {
                            LedEvent::PriceDown
                        }
                    } else if last_led_event == LedEvent::PriceUp {
                        LedEvent::PriceUpEnded
                    } else if last_led_event == LedEvent::PriceDown {
                        LedEvent::PriceDownEnded
                    } else {
                        LedEvent::PriceUpEnded
                    };

                    if led_event != last_led_event {
                        last_led_event = led_event;

                        if let Err(e) = led_event_tx.try_send(led_event) {
                            error!("Failed to send command: {}", e);
                        }
                    }
                }
            }
        });
    }

    fn run_alarm_event_listener(&self, led_event_tx: Sender<LedEvent>) {
        let mut rx_events = self.alarm_bus.subscribe_events();
        tokio::spawn({
            async move {
                loop {
                    let event = match rx_events.recv().await {
                        Ok(event) => event,
                        // Skipping stale events must not kill the listener;
                        // only a closed bus ends it.
                        Err(sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(skipped = n, "alarm event receiver lagged");
                            continue;
                        }
                        Err(sync::broadcast::error::RecvError::Closed) => break,
                    };
                    match event {
                        AlarmEvent::Stopped { .. } | AlarmEvent::Snoozed => {
                            if let Err(e) = led_event_tx.try_send(LedEvent::ClockAlarmEnded) {
                                error!("Failed to send command: {}", e);
                            }
                        }

                        AlarmEvent::Started { .. } => {
                            if let Err(e) = led_event_tx.try_send(LedEvent::ClockAlarm) {
                                error!("Failed to send command: {}", e);
                            }
                        }
                    }
                }
            }
        });
    }

    fn run_wifi_scan_listener(&self, led_event_tx: Sender<LedEvent>) {
        let mut rx_events = self.manager.subscribe_wifi_events();
        tokio::spawn({
            async move {
                loop {
                    let event = match rx_events.recv().await {
                        Ok(event) => event,
                        // Skipping stale events must not kill the listener;
                        // only a closed bus ends it.
                        Err(sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(skipped = n, "wifi event receiver lagged");
                            continue;
                        }
                        Err(sync::broadcast::error::RecvError::Closed) => break,
                    };
                    let led_event = match event {
                        WifiEvent::ScanStarted => LedEvent::WifiScan,
                        WifiEvent::ScanEnded => LedEvent::WifiScanEnded,
                    };

                    if let Err(e) = led_event_tx.try_send(led_event) {
                        error!("Failed to send command: {}", e);
                    }
                }
            }
        });
    }

    pub(crate) fn init(
        &mut self,
        led_cmd_tx: Sender<LedCommand>,
        coordinator: LedCoordinatorHandle,
    ) {
        self.command_sender = Some(led_cmd_tx);
        let led_event_tx = Self::spawn_event_loop(coordinator);
        self.event_sender = Some(led_event_tx.clone());

        self.run_wifi_task(led_event_tx.clone());
        self.run_sysupgrade_task(led_event_tx.clone());
        self.run_alarm_event_listener(led_event_tx.clone());
        self.run_wifi_scan_listener(led_event_tx);

        // TODO: Price alerts
        // self.run_price_task(led_event_tx);
    }

    fn spawn_event_loop(coordinator: LedCoordinatorHandle) -> Sender<LedEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        task::spawn(async move {
            let mut state = LedIndicatorsState::new();
            // Idle when no temp is active; reset to the temp's deadline when a
            // duration-bearing scene is published, so the system layer clears
            // back to the persistent (or `None`) when the temp expires.
            let pick_deadline = Instant::now() + IDLE_EXPIRY;
            let temp_expiry = tokio::time::sleep_until(pick_deadline);
            tokio::pin!(temp_expiry);

            loop {
                tokio::select! {
                    event = receiver.recv() => {
                        let Some(event) = event else { break };
                        debug!("Received LED event: {:?}", event);
                        state.apply_event(event);
                        let scene = state.current_scene();
                        temp_expiry.as_mut().reset(next_wakeup(&state));
                        coordinator.publish(Layer::System, scene);
                    }
                    () = &mut temp_expiry => {
                        let scene = state.current_scene();
                        temp_expiry.as_mut().reset(next_wakeup(&state));
                        coordinator.publish(Layer::System, scene);
                    }
                }
            }
        });

        sender
    }

    pub fn push_event(&self, event: LedEvent) {
        if let Some(sender) = &self.event_sender {
            let _ = sender.try_send(event);
        }
    }

    pub fn send_command(&self, command: LedCommand) {
        if let Some(sender) = &self.command_sender {
            let _ = sender.try_send(command);
        }
    }
}

pub(crate) async fn run_led_state_task(
    mut state_rx: watch::Receiver<LedState>,
    coordinator: LedCoordinatorHandle,
) {
    loop {
        let enabled = matches!(*state_rx.borrow_and_update(), LedState::Enabled);
        debug!("Setting led enabled: {}", enabled);
        coordinator.set_enabled(enabled);
        if state_rx.changed().await.is_err() {
            break;
        }
    }
}

fn upgrade_state_to_led(state: &SystemUpgradeState) -> Option<LedEvent> {
    match state {
        SystemUpgradeState::DownloadStarted { .. } | SystemUpgradeState::UpgradeStarted => {
            Some(LedEvent::DownloadOrUpgradeStarted)
        }
        // A finished download is not a finished upgrade: the build and
        // activation still run afterwards. Keep the in-progress animation
        // going instead of flashing a premature success.
        SystemUpgradeState::DownloadProgress { .. }
        | SystemUpgradeState::DownloadFinished { .. } => None,
        // Only package upgrades reach here: a firmware upgrade ends by rebooting
        // into the new image, so it never emits `Finished` and its success is
        // the reboot itself, not a terminal green. The applying phase keeps the
        // in-progress animation running until the process dies.
        SystemUpgradeState::Finished => Some(LedEvent::DownloadOrUpgradeSuccess),
        SystemUpgradeState::Failed => Some(LedEvent::DownloadOrUpgradeError),
    }
}

#[cfg(test)]
mod tests {
    use super::upgrade_state_to_led;
    use crate::system_upgrade::SystemUpgradeState;
    use bmc_led::data::LedEvent;

    #[test]
    fn download_finished_keeps_the_in_progress_animation() {
        assert_eq!(
            upgrade_state_to_led(&SystemUpgradeState::DownloadFinished {
                hash: None,
                total_mb: None,
            }),
            None,
            "a finished download must not flash success before build and activate run"
        );
    }

    #[test]
    fn only_finished_signals_success() {
        assert_eq!(
            upgrade_state_to_led(&SystemUpgradeState::Finished),
            Some(LedEvent::DownloadOrUpgradeSuccess)
        );
    }

    #[test]
    fn failure_signals_error() {
        assert_eq!(
            upgrade_state_to_led(&SystemUpgradeState::Failed),
            Some(LedEvent::DownloadOrUpgradeError)
        );
    }
}

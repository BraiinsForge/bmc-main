// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{sync::Arc, time::Duration};

use bmc_shared_ii_net::wifi::SignalStrength;
use tokio::{
    sync::{self, mpsc::Sender, watch},
    time::interval,
};
use tracing::{debug, error};

use crate::{
    BmcManager,
    alarm::{AlarmBus, AlarmEvent},
    manager::WifiEvent,
    system_upgrade::{StateService, SystemUpgradeState},
};
use bmc_led::{
    data::{LedCommand, LedEvent},
    led_driver::LedEventHandler,
};
use tokio::task;

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

impl From<LedState> for LedEvent {
    fn from(value: LedState) -> Self {
        match value {
            LedState::Enabled => LedEvent::Enable,
            LedState::Disabled => LedEvent::Disable,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LedController<T>
where
    T: BmcManager,
{
    led_event_handler: LedEventHandler,
    command_sender: Option<Sender<LedCommand>>,
    system_upgrade_receiver: sync::watch::Receiver<Option<SystemUpgradeState>>,
    manager: Arc<T>,
    last_price_change_24h_receiver: watch::Receiver<f32>,
    state_receiver: watch::Receiver<LedState>,
    alarm_bus: AlarmBus,
}

impl<T: BmcManager> Clone for LedController<T> {
    fn clone(&self) -> Self {
        Self {
            led_event_handler: self.led_event_handler.clone(),
            command_sender: self.command_sender.clone(),
            system_upgrade_receiver: self.system_upgrade_receiver.clone(),
            manager: self.manager.clone(),
            last_price_change_24h_receiver: self.last_price_change_24h_receiver.clone(),
            state_receiver: self.state_receiver.clone(),
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
    ) -> (Self, watch::Sender<LedState>) {
        let system_upgrade_receiver = state_service.subscribe();

        let (state_sender, state_receiver) = watch::channel(led_enabled.into());

        let this = Self {
            led_event_handler: LedEventHandler::default(),
            command_sender: None,
            system_upgrade_receiver,
            manager,
            last_price_change_24h_receiver: last_price_change_24h,
            state_receiver,
            alarm_bus,
        };

        (this, state_sender)
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
                    if let Some(upgrade_status) = state {
                        let led_event = match upgrade_status {
                            SystemUpgradeState::DownloadStarted { .. }
                            | SystemUpgradeState::UpgradeStarted => {
                                Some(LedEvent::DownloadOrUpgradeStarted)
                            }
                            SystemUpgradeState::DownloadProgress { .. } => None,
                            SystemUpgradeState::DownloadFinished { .. } => {
                                Some(LedEvent::DownloadOrUpgradeSuccess)
                            }
                            SystemUpgradeState::Failed => Some(LedEvent::DownloadOrUpgradeError),
                        };

                        if let Some(led_event) = led_event {
                            // Ignore the result, since we don't care if the send fails
                            if let Err(e) = led_event_tx.send(led_event).await {
                                error!("Failed to send command: {}", e);
                            }
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

    fn run_led_state_task(&self, led_event_tx: Sender<LedEvent>) {
        let mut receiver = self.state_receiver.clone();

        task::spawn(async move {
            while let Ok(()) = receiver.changed().await {
                let led_state = (*receiver.borrow_and_update()).clone();

                debug!("Setting led state: {:?}", led_state);

                if let Err(e) = led_event_tx.send(led_state.into()).await {
                    error!("Failed to send led command: {}", e);
                }
            }
        });
    }

    fn run_alarm_event_listener(&self, led_event_tx: Sender<LedEvent>) {
        let mut rx_events = self.alarm_bus.subscribe_events();
        tokio::spawn({
            async move {
                while let Ok(event) = rx_events.recv().await {
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
                while let Ok(event) = rx_events.recv().await {
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

    pub(crate) fn init(&mut self, led_cmd_tx: Sender<LedCommand>) {
        self.command_sender = Some(led_cmd_tx.clone());
        let led_event_tx = self.led_event_handler.init(led_cmd_tx);

        // NOTE: Enable/Disable led events on start
        let led_state = self.state_receiver.borrow().clone().into();
        self.push_event(led_state);

        self.run_led_state_task(led_event_tx.clone());
        self.run_wifi_task(led_event_tx.clone());
        self.run_sysupgrade_task(led_event_tx.clone());
        self.run_alarm_event_listener(led_event_tx.clone());
        self.run_wifi_scan_listener(led_event_tx);

        // TODO: Price alerts
        // self.run_price_task(led_event_tx);
    }

    pub fn push_event(&self, event: LedEvent) {
        self.led_event_handler.push_event(event);
    }

    pub fn send_command(&self, command: LedCommand) {
        if let Some(sender) = &self.command_sender {
            let _ = sender.try_send(command);
        }
    }
}

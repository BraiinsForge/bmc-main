// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::Duration;

use super::data;
use crate::config::{
    self, BREATHE_PERIOD, ERROR_DURATION, KNIGHT_RIDER_PERIOD, RGB_GREEN, RGB_ORANGE, RGB_RED,
    RGB_VIOLET60, SOLID_PERIOD, SUCCESS_DURATION,
};
use crate::data::{LedCommand, LedEffect, LedEvent, LedEventPersistence};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error};

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
        Ok(())
    }

    pub fn state(&self) -> anyhow::Result<bool> {
        Ok(true)
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
        Ok(1.0)
    }

    #[must_use]
    pub fn max_brightness(&self) -> f32 {
        config::LED_MAX_BRIGHTNESS
    }

    pub fn set_brightness(&self, _value: f32) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct LedEventHandler {
    pub event_sender: Option<Sender<data::LedEvent>>,
}

#[derive(Debug, Default, Clone)]
struct LedIndicatorsState {
    wifi_persist: Option<LedCommand>,
    wifi_temp: Option<LedCommand>,
    wifi_scan_persist: Option<LedCommand>,
    price_persist: Option<LedCommand>,
    clock_persist: Option<LedCommand>,
    device_persist: Option<LedCommand>,
    sys_persist: Option<LedCommand>,
    scene_persist: Option<LedCommand>,
}

impl LedIndicatorsState {
    #[expect(clippy::too_many_lines)]
    fn apply_event(
        &mut self,
        event: LedEvent,
    ) -> (Option<LedCommand>, Option<LedCommand>, Option<LedCommand>) {
        let mut control: Option<LedCommand> = None;

        match event {
            // Device lifecycle
            LedEvent::DeviceInitializing => {
                self.device_persist = Some(LedCommand::SetEffect(
                    LedEffect::KnightRider(RGB_VIOLET60),
                    LedEventPersistence::Persistent,
                    KNIGHT_RIDER_PERIOD,
                ));
            }
            LedEvent::DeviceReady => {
                self.device_persist = None;
            }

            // Wi-Fi
            LedEvent::WifiConnecting => {
                self.wifi_persist = Some(LedCommand::SetEffect(
                    LedEffect::KnightRider(RGB_VIOLET60),
                    LedEventPersistence::Persistent,
                    KNIGHT_RIDER_PERIOD,
                ));
                self.wifi_temp = None;
            }
            LedEvent::WifiConnected => {
                self.wifi_persist = None;
                self.wifi_temp = Some(LedCommand::SetEffect(
                    LedEffect::Solid(RGB_GREEN),
                    LedEventPersistence::Temporary(SUCCESS_DURATION),
                    SOLID_PERIOD,
                ));
            }
            LedEvent::WifiNone => {
                self.wifi_persist = Some(LedCommand::SetEffect(
                    LedEffect::None,
                    LedEventPersistence::Persistent,
                    Duration::from_secs(0),
                ));
                self.wifi_temp = None;
            }
            LedEvent::WifiError => {
                self.wifi_persist = Some(LedCommand::SetEffect(
                    LedEffect::Solid(RGB_RED),
                    LedEventPersistence::Persistent,
                    SOLID_PERIOD,
                ));
                self.wifi_temp = None;
            }

            LedEvent::WifiScan => {
                self.wifi_scan_persist = Some(LedCommand::SetEffect(
                    LedEffect::KnightRider(RGB_VIOLET60),
                    LedEventPersistence::Persistent,
                    KNIGHT_RIDER_PERIOD,
                ));
                self.wifi_temp = None;
            }

            LedEvent::WifiScanEnded => {
                self.wifi_scan_persist = None;
            }

            // Preview of the scene
            LedEvent::PreviewScene => {
                self.scene_persist = Some(LedCommand::SetEffect(
                    LedEffect::KnightRider(RGB_VIOLET60),
                    LedEventPersistence::Persistent,
                    KNIGHT_RIDER_PERIOD,
                ));
            }
            LedEvent::PreviewSceneEnded => {
                self.scene_persist = None;
            }

            // Price
            LedEvent::PriceUp => {
                self.price_persist = Some(LedCommand::SetEffect(
                    LedEffect::Breathe(RGB_GREEN),
                    LedEventPersistence::Persistent,
                    BREATHE_PERIOD,
                ));
            }
            LedEvent::PriceDown => {
                self.price_persist = Some(LedCommand::SetEffect(
                    LedEffect::Breathe(RGB_RED),
                    LedEventPersistence::Persistent,
                    BREATHE_PERIOD,
                ));
            }
            LedEvent::PriceUpEnded | LedEvent::PriceDownEnded => {
                self.price_persist = None;
            }

            // Clock
            LedEvent::ClockAlarm => {
                self.clock_persist = Some(LedCommand::SetEffect(
                    LedEffect::Breathe(RGB_ORANGE),
                    LedEventPersistence::Persistent,
                    BREATHE_PERIOD,
                ));
            }
            LedEvent::ClockAlarmEnded => {
                self.clock_persist = None;
            }

            // System updates
            LedEvent::DownloadOrUpgradeStarted => {
                self.sys_persist = Some(LedCommand::SetEffect(
                    LedEffect::KnightRider(RGB_ORANGE),
                    LedEventPersistence::Persistent,
                    KNIGHT_RIDER_PERIOD,
                ));
            }
            LedEvent::DownloadOrUpgradeSuccess => {
                self.sys_persist = Some(LedCommand::SetEffect(
                    LedEffect::Solid(RGB_GREEN),
                    LedEventPersistence::Temporary(SUCCESS_DURATION),
                    SOLID_PERIOD,
                ));
            }
            LedEvent::DownloadOrUpgradeError => {
                self.sys_persist = Some(LedCommand::SetEffect(
                    LedEffect::Solid(RGB_RED),
                    LedEventPersistence::Temporary(ERROR_DURATION),
                    SOLID_PERIOD,
                ));
            }

            // Global control
            LedEvent::Enable => control = Some(LedCommand::Enable),
            LedEvent::Disable => control = Some(LedCommand::Disable),
        }

        let temp = self.wifi_temp.take();
        let persistent = self.select_persistent(temp.is_some());

        (control, temp, persistent)
    }

    fn any_persist_active(&self) -> bool {
        self.device_persist.is_some()
            || self.clock_persist.is_some()
            || self.wifi_persist.is_some()
            || self.sys_persist.is_some()
            || self.price_persist.is_some()
            || self.scene_persist.is_some()
            || self.wifi_scan_persist.is_some()
    }

    fn select_persistent(&self, temp_present: bool) -> Option<LedCommand> {
        if temp_present {
            self.device_persist.or(self.clock_persist).or_else(|| {
                if self.any_persist_active() {
                    None
                } else {
                    Some(LedCommand::SetEffect(
                        LedEffect::None,
                        LedEventPersistence::Persistent,
                        Duration::from_secs(0),
                    ))
                }
            })
        } else {
            self.device_persist
                .or(self.clock_persist)
                .or(self.wifi_persist)
                .or(self.sys_persist)
                .or(self.scene_persist)
                .or(self.wifi_scan_persist)
                .or(self.price_persist)
                .or(Some(LedCommand::SetEffect(
                    LedEffect::None,
                    LedEventPersistence::Persistent,
                    Duration::from_secs(0),
                )))
        }
    }
}

impl LedEventHandler {
    #[must_use]
    pub fn init(&mut self, command_sender: Sender<LedCommand>) -> Sender<data::LedEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);
        self.event_sender = Some(sender.clone());

        tokio::spawn(async move {
            let mut state = LedIndicatorsState::default();
            let sender = command_sender.clone();

            while let Some(event) = receiver.recv().await {
                debug!("Received LED event: {:?}", event);

                let (control, temp, persistent) = state.apply_event(event);

                for cmd in [control, temp, persistent].into_iter().flatten() {
                    if let Err(e) = sender.send(cmd).await {
                        error!("Failed to send LED command {:?}: {e}", cmd);
                    }
                }
            }
        });

        sender
    }

    pub fn push_event(&self, event: data::LedEvent) {
        if let Some(sender) = &self.event_sender {
            let _ = sender.try_send(event);
        }
    }
}

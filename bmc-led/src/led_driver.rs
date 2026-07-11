// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::{
    self, BREATHE_PERIOD, ERROR_DURATION, KNIGHT_RIDER_PERIOD, RGB_GREEN, RGB_ORANGE, RGB_RED,
    RGB_VIOLET60, SUCCESS_DURATION,
};
use crate::data::{LedCommand, LedEffect, LedEvent, LedScene};
use std::time::Instant;
use tokio::sync::mpsc::Sender;

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

#[derive(Debug, Default, Clone)]
pub struct LedIndicatorsState {
    wifi_persist: Option<LedCommand>,
    temp: Option<LedCommand>,
    /// Wall-clock instant at which `temp` stops being active. Tracked
    /// separately from `temp` so a later unrelated event re-resolving the
    /// scene cannot extend or forget the original temp window.
    temp_deadline: Option<Instant>,
    wifi_scan_persist: Option<LedCommand>,
    price_persist: Option<LedCommand>,
    clock_persist: Option<LedCommand>,
    device_persist: Option<LedCommand>,
    sys_persist: Option<LedCommand>,
}

impl LedIndicatorsState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm a one-shot temp scene, deriving its deadline from the scene's
    /// own `duration`. A duration-less scene stays active until cleared.
    fn set_temp(&mut self, scene: LedScene) {
        self.temp_deadline = scene.duration.map(|d| Instant::now() + d);
        self.temp = Some(LedCommand::SetEffect(scene));
    }

    fn clear_temp(&mut self) {
        self.temp = None;
        self.temp_deadline = None;
    }

    /// Instant at which the active temp expires, if any. The event loop
    /// arms its wake-up timer from this so the temp window is owned here
    /// rather than re-derived from the resolved scene on every event.
    #[must_use]
    pub fn temp_deadline(&self) -> Option<Instant> {
        self.temp_deadline
    }

    pub fn apply_event(&mut self, event: LedEvent) {
        match event {
            // Device lifecycle
            LedEvent::DeviceInitializing => {
                self.device_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::KnightRider(RGB_VIOLET60),
                    period: Some(KNIGHT_RIDER_PERIOD),
                    duration: None,
                }));
            }
            LedEvent::DeviceReady => {
                self.device_persist = None;
            }

            // Wi-Fi
            LedEvent::WifiConnecting => {
                self.wifi_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::KnightRider(RGB_VIOLET60),
                    period: Some(KNIGHT_RIDER_PERIOD),
                    duration: None,
                }));
                self.clear_temp();
            }
            LedEvent::WifiConnected => {
                self.wifi_persist = None;
                self.set_temp(LedScene {
                    effect: LedEffect::Solid(RGB_GREEN),
                    period: None,
                    duration: Some(SUCCESS_DURATION),
                });
            }
            LedEvent::WifiNone | LedEvent::WifiError => {
                self.wifi_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::None,
                    period: None,
                    duration: None,
                }));
                self.clear_temp();
            }

            LedEvent::WifiScan => {
                self.wifi_scan_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::KnightRider(RGB_VIOLET60),
                    period: Some(KNIGHT_RIDER_PERIOD),
                    duration: None,
                }));
                self.clear_temp();
            }

            LedEvent::WifiScanEnded => {
                self.wifi_scan_persist = None;
            }

            // Price
            LedEvent::PriceUp => {
                self.price_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::Breathe(RGB_GREEN),
                    period: Some(BREATHE_PERIOD),
                    duration: None,
                }));
            }
            LedEvent::PriceDown => {
                self.price_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::Breathe(RGB_RED),
                    period: Some(BREATHE_PERIOD),
                    duration: None,
                }));
            }
            LedEvent::PriceUpEnded | LedEvent::PriceDownEnded => {
                self.price_persist = None;
            }

            // Clock
            LedEvent::ClockAlarm => {
                self.clock_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::Breathe(RGB_ORANGE),
                    period: Some(BREATHE_PERIOD),
                    duration: None,
                }));
            }
            LedEvent::ClockAlarmEnded => {
                self.clock_persist = None;
            }

            // System updates
            LedEvent::DownloadOrUpgradeStarted => {
                self.sys_persist = Some(LedCommand::SetEffect(LedScene {
                    effect: LedEffect::KnightRider(RGB_ORANGE),
                    period: Some(KNIGHT_RIDER_PERIOD),
                    duration: None,
                }));
            }
            LedEvent::DownloadOrUpgradeSuccess => {
                self.sys_persist = None;
                self.set_temp(LedScene {
                    effect: LedEffect::Solid(RGB_GREEN),
                    period: None,
                    duration: Some(SUCCESS_DURATION),
                });
            }
            LedEvent::DownloadOrUpgradeError => {
                self.sys_persist = None;
                self.set_temp(LedScene {
                    effect: LedEffect::Solid(RGB_RED),
                    period: None,
                    duration: Some(ERROR_DURATION),
                });
            }
        }
    }

    fn any_persist_active(&self) -> bool {
        self.device_persist.is_some()
            || self.clock_persist.is_some()
            || self.wifi_persist.is_some()
            || self.sys_persist.is_some()
            || self.price_persist.is_some()
            || self.wifi_scan_persist.is_some()
    }

    const NONE_SCENE: LedCommand = LedCommand::SetEffect(LedScene {
        effect: LedEffect::None,
        period: None,
        duration: None,
    });

    fn select_persistent(&self, temp_present: bool) -> Option<LedCommand> {
        if temp_present {
            self.device_persist.or(self.clock_persist).or_else(|| {
                if self.any_persist_active() {
                    None
                } else {
                    Some(Self::NONE_SCENE)
                }
            })
        } else {
            self.device_persist
                .or(self.clock_persist)
                .or(self.wifi_persist)
                .or(self.sys_persist)
                .or(self.wifi_scan_persist)
                .or(self.price_persist)
                .or(Some(Self::NONE_SCENE))
        }
    }

    /// Return the currently active scene, resolved across the priority
    /// stack and the temp slot. The temp slot, when set, wins and stays
    /// active until its `temp_deadline` passes — it is dropped here once
    /// expired, not consumed on first read, so polling it repeatedly within
    /// its window keeps reporting it.
    ///
    /// `None` means "this layer has nothing to draw" — the coordinator
    /// falls through to lower layers. A resolved scene whose effect is
    /// `LedEffect::None` is also reported as `None`; pinning `Layer::System`
    /// to a None-effect scene would mask every lower layer.
    #[must_use]
    pub fn current_scene(&mut self) -> Option<LedScene> {
        self.current_scene_at(Instant::now())
    }

    fn current_scene_at(&mut self, now: Instant) -> Option<LedScene> {
        if self.temp_deadline.is_some_and(|deadline| now >= deadline) {
            self.clear_temp();
        }
        let resolved = match &self.temp {
            Some(LedCommand::SetEffect(scene)) => Some(*scene),
            _ => match self.select_persistent(false) {
                Some(LedCommand::SetEffect(scene)) => Some(scene),
                _ => None,
            },
        };
        resolved.filter(|s| !matches!(s.effect, LedEffect::None))
    }
}

#[cfg(test)]
mod tests {
    use super::LedIndicatorsState;
    use crate::data::{LedEffect, LedEvent};

    #[test]
    fn current_scene_is_none_when_nothing_set() {
        let mut state = LedIndicatorsState::new();
        assert!(state.current_scene().is_none());
    }

    #[test]
    fn current_scene_reports_persistent_repeatably() {
        let mut state = LedIndicatorsState::new();
        state.apply_event(LedEvent::WifiConnecting);

        // A persistent indicator is not consumed: it reports the same scene
        // on every poll until something clears it.
        for _ in 0..2 {
            let scene = state
                .current_scene()
                .expect("BUG: wifi-connecting must drive the system layer");
            assert!(matches!(scene.effect, LedEffect::KnightRider(_)));
            assert!(scene.duration.is_none());
        }
    }

    #[test]
    fn current_scene_temp_stays_active_until_deadline_then_persistent() {
        let mut state = LedIndicatorsState::new();
        // A persistent system indicator underneath, plus a success temp.
        state.apply_event(LedEvent::DownloadOrUpgradeStarted);
        state.apply_event(LedEvent::WifiConnected);
        let deadline = state
            .temp_deadline()
            .expect("BUG: a duration-bearing temp must arm a deadline");

        // The temp is not consumed on read: repeated polls within its window
        // keep reporting it, so an idle re-poll cannot drop the flash early.
        for _ in 0..3 {
            let temp = state.current_scene().expect("BUG: temp must stay active");
            assert!(matches!(temp.effect, LedEffect::Solid(_)));
        }

        // Once the deadline passes, the temp is dropped and the persistent
        // underneath shows through.
        let persistent = state
            .current_scene_at(deadline)
            .expect("BUG: persistent must remain once the temp expires");
        assert!(matches!(persistent.effect, LedEffect::KnightRider(_)));
        assert!(persistent.duration.is_none());
        assert!(state.temp_deadline().is_none());
    }

    #[test]
    fn unrelated_event_does_not_disturb_the_temp_window() {
        let mut state = LedIndicatorsState::new();
        state.apply_event(LedEvent::WifiConnected);
        let deadline = state
            .temp_deadline()
            .expect("BUG: temp must arm a deadline");

        // An event that does not touch the temp slot (a system indicator
        // underneath) must neither end the flash early nor move its deadline.
        state.apply_event(LedEvent::DownloadOrUpgradeStarted);
        let scene = state
            .current_scene()
            .expect("BUG: the success flash must outlive an unrelated event");
        assert!(matches!(scene.effect, LedEffect::Solid(_)));
        assert_eq!(state.temp_deadline(), Some(deadline));
    }

    #[test]
    fn current_scene_filters_none_effect_to_none() {
        let mut state = LedIndicatorsState::new();
        // `WifiNone` parks a `None`-effect persistent; the system layer must
        // report `None` so lower layers show through instead of being masked.
        state.apply_event(LedEvent::WifiNone);
        assert!(state.current_scene().is_none());
    }
}

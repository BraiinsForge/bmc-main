// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub enum LedEffect {
    #[default]
    None,
    Chase(Rgb),
    KnightRider(Rgb),
    Scan(Rgb),
    Snake(Rgb),
    Breathe(Rgb),
    Solid(Rgb),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedEventPersistence {
    Temporary(Duration),
    Persistent,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LedEvent {
    DeviceInitializing, // Knight Rider
    DeviceReady,
    WifiConnecting, // Knight Rider
    WifiConnected,  // Success
    WifiNone,       // None
    WifiError,      // Error
    WifiScan,       // Knight Rider
    WifiScanEnded,
    PriceUp, // Breathe
    PriceUpEnded,
    PriceDown, // Breathe
    PriceDownEnded,
    ClockAlarm, // Breathe
    ClockAlarmEnded,
    DownloadOrUpgradeStarted, // Knight Rider
    DownloadOrUpgradeSuccess, // Success
    DownloadOrUpgradeError,   // Error
    Disable,
    Enable,
    PreviewScene, // Knight Rider
    PreviewSceneEnded,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LedCommand {
    Disable,
    Enable,
    SetBrightness(f32),
    SetEffect(LedEffect, LedEventPersistence, Duration),
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedEffectKind {
    #[default]
    None,
    Solid,
    Breathe,
    Chase,
    KnightRider,
    Scan,
    Snake,
}

impl LedEffectKind {
    #[must_use]
    pub fn with_color(self, rgb: Rgb) -> LedEffect {
        match self {
            Self::None => LedEffect::None,
            Self::Solid => LedEffect::Solid(rgb),
            Self::Breathe => LedEffect::Breathe(rgb),
            Self::Chase => LedEffect::Chase(rgb),
            Self::KnightRider => LedEffect::KnightRider(rgb),
            Self::Scan => LedEffect::Scan(rgb),
            Self::Snake => LedEffect::Snake(rgb),
        }
    }
}

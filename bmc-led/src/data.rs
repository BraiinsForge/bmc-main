// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::Duration;

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
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

/// An LED effect bundled with its timing metadata.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct LedScene {
    pub effect: LedEffect,
    /// Animation cycle period.  `None` for static effects (Solid, None).
    pub period: Option<Duration>,
    /// How long this scene lasts.  `None` = persistent (until replaced).
    pub duration: Option<Duration>,
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

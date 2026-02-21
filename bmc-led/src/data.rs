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

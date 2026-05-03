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

/// Bare-discriminant view of [`LedEffect`] used as the wire-format kind byte
/// between the wasm SDK guest and the host.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum LedEffectKind {
    None = 0,
    Chase = 1,
    KnightRider = 2,
    Scan = 3,
    Snake = 4,
    Breathe = 5,
    Solid = 6,
}

impl TryFrom<u8> for LedEffectKind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Chase),
            2 => Ok(Self::KnightRider),
            3 => Ok(Self::Scan),
            4 => Ok(Self::Snake),
            5 => Ok(Self::Breathe),
            6 => Ok(Self::Solid),
            other => Err(other),
        }
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
    SetEffect(LedScene),
}

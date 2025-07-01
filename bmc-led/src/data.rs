// Copyright (C) 2025  Braiins Systems s.r.o.

#[derive(Debug, Copy, Clone)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    #[must_use]
    pub const fn from(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum LedEffect {
    Snake,
    Chase,
    Scan,
    Fireflies,
    KnightRider,
}

#[derive(Debug, Clone, Copy)]
pub enum LedCommand {
    NoChange,
    // SetTemporaryEffect(LedEffect, Rgb),
    SetPersistentEffect(LedEffect, Rgb),
    SetColor(Rgb),
    SetBrightness(f32),
}

#[derive(Debug)]
pub enum LedEvent {
    Idle,
    Alarm,
    DownloadStarted,
    DownloadProgress,
    DownloadFinished,
    UpgradeStarted,
    UpgradeFailed,
    UpgradeFinishedSuccessfully,
    TimezoneChanged,
}

// Copyright (C) 2025  Braiins Systems s.r.o.

#[derive(Debug, Copy, Clone, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub enum LedEffect {
    None,
    Chase,
    #[default]
    Fireflies,
    KnightRider,
    Scan,
    Snake,
}

#[derive(Debug, Clone, Copy)]
pub enum LedCommand {
    Disable,
    Enable,
    SetBrightness(f32),
    SetColor(Rgb),
    SetPersistentEffect(LedEffect, Rgb),
    // SetTemporaryEffect(LedEffect, Rgb), // TODO: Implement temporary effects once we decide how to handle temporary states #BOS-3299
}

#[derive(Debug)]
pub enum LedEvent {
    Alarm,
    Disable,
    DownloadFinished,
    DownloadProgress,
    DownloadStarted,
    Enable,
    Failed,
    Idle,
    UpgradeStarted,
}

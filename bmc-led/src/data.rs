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
    Chase,
    #[default]
    Fireflies,
    KnightRider,
    Scan,
    Snake,
}

#[derive(Debug, Clone, Copy)]
pub enum LedCommand {
    SetBrightness(f32),
    SetColor(Rgb),
    SetPersistentEffect(LedEffect, Rgb),
    // SetTemporaryEffect(LedEffect, Rgb), // TODO: Implement temporary effects once we decide how to handle temporary states
}

#[derive(Debug)]
pub enum LedEvent {
    Alarm,
    DownloadFinished,
    DownloadProgress,
    DownloadStarted,
    Failed,
    Idle,
    UpgradeStarted,
}

pub const APA102_MAX_BRIGHTNESS: u8 = 31; // APA102 max brightness value (5 bits)
pub const LED_MAX_BRIGHTNESS: f32 = 1.0;
pub const RGB_MAX: u8 = 255;
pub const LED_FRACTION_MAX: f32 = 1.0;
pub const LED_MIN_FACTOR: f32 = 0.1;
pub const LED_PHASE_MULTIPLIER: f32 = 2.0;

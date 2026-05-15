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

/// Bare-discriminant view of [`LedEffect`] used as the wire-format kind byte
/// between the wasm SDK guest and the host. Pinned with `repr(u8)` so the
/// discriminants match `deck_widget_v1.led_effect`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum LedEffectKind {
    Chase = 0,
    KnightRider = 1,
    Scan = 2,
    Snake = 3,
    Breathe = 4,
    Solid = 5,
}

impl TryFrom<u8> for LedEffectKind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Chase),
            1 => Ok(Self::KnightRider),
            2 => Ok(Self::Scan),
            3 => Ok(Self::Snake),
            4 => Ok(Self::Breathe),
            5 => Ok(Self::Solid),
            other => Err(other),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_kind_discriminants_match_protocol() {
        assert_eq!(LedEffectKind::Chase as u8, 0);
        assert_eq!(LedEffectKind::KnightRider as u8, 1);
        assert_eq!(LedEffectKind::Scan as u8, 2);
        assert_eq!(LedEffectKind::Snake as u8, 3);
        assert_eq!(LedEffectKind::Breathe as u8, 4);
        assert_eq!(LedEffectKind::Solid as u8, 5);
    }

    #[test]
    fn effect_kind_try_from_round_trips_known_values() {
        for v in 0_u8..=5 {
            assert_eq!(
                LedEffectKind::try_from(v).expect("BUG: 0..=5 must be valid") as u8,
                v
            );
        }
        assert_eq!(LedEffectKind::try_from(6_u8), Err(6));
    }
}

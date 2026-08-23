// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
/// discriminants match `deck_widget.led_effect`.
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

/// LED request scope — which arbitration tier a request lands on.
/// Pinned with `repr(u8)` to match `deck_widget.led_scope`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum LedScope {
    Local = 0,
    Global = 1,
}

impl TryFrom<u8> for LedScope {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Local),
            1 => Ok(Self::Global),
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

    // These bytes are the guest↔host wasm wire format. They are also kept
    // equal to `deck_widget.led_effect` by hand — bmc-led can't import the
    // protocol crate to assert that here, so this only pins the values stable.
    #[test]
    fn effect_kind_wire_bytes_are_stable() {
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

    #[test]
    fn scope_discriminants_match_protocol() {
        assert_eq!(LedScope::Local as u8, 0);
        assert_eq!(LedScope::Global as u8, 1);
    }

    #[test]
    fn scope_try_from_round_trips_known_values() {
        for v in 0_u8..=1 {
            assert_eq!(
                LedScope::try_from(v).expect("BUG: 0..=1 must be valid") as u8,
                v
            );
        }
        assert_eq!(LedScope::try_from(2_u8), Err(2));
    }
}

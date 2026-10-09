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

//! Animation types shared between SDK (WASM) and host runtime.

/// Animatable property identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AnimProperty {
    Rotate = 0,
    Scale = 1,
    Alpha = 2,
    TranslateX = 3,
    TranslateY = 4,
    OrbitAngle = 5,
    Color = 6,
}

/// Easing function identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Easing {
    Linear = 0,
    EaseIn = 1,
    EaseOut = 2,
    EaseInOut = 3,
    EaseInCubic = 4,
    EaseOutCubic = 5,
    EaseInOutCubic = 6,
    /// Overshoot then settle — good for snappy UI transitions.
    EaseOutBack = 7,
    /// Anticipation + overshoot — dramatic entrance/exit.
    EaseInOutBack = 8,
    /// Natural settle with multiple bounces — dice landing, ball drop.
    EaseOutBounce = 9,
    /// Spring-like damped oscillation — wobbly settle.
    EaseOutElastic = 10,
}

/// Loop mode for repeating animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LoopMode {
    Once = 0,
    Forever = 1,
    PingPong = 2,
}

/// Color interpolation space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ColorSpace {
    #[default]
    Oklab = 0,
    Oklch = 1,
    LinearRgb = 2,
    Srgb = 3,
}

impl AnimProperty {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rotate),
            1 => Some(Self::Scale),
            2 => Some(Self::Alpha),
            3 => Some(Self::TranslateX),
            4 => Some(Self::TranslateY),
            5 => Some(Self::OrbitAngle),
            6 => Some(Self::Color),
            _ => None,
        }
    }
}

impl Easing {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Linear),
            1 => Some(Self::EaseIn),
            2 => Some(Self::EaseOut),
            3 => Some(Self::EaseInOut),
            4 => Some(Self::EaseInCubic),
            5 => Some(Self::EaseOutCubic),
            6 => Some(Self::EaseInOutCubic),
            7 => Some(Self::EaseOutBack),
            8 => Some(Self::EaseInOutBack),
            9 => Some(Self::EaseOutBounce),
            10 => Some(Self::EaseOutElastic),
            _ => None,
        }
    }
}

impl LoopMode {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Once),
            1 => Some(Self::Forever),
            2 => Some(Self::PingPong),
            _ => None,
        }
    }
}

impl ColorSpace {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Oklab),
            1 => Some(Self::Oklch),
            2 => Some(Self::LinearRgb),
            3 => Some(Self::Srgb),
            _ => None,
        }
    }
}

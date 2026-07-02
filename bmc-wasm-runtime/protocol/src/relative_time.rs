// Copyright (C) 2026  Braiins Systems s.r.o.

//! Wire options for the `RelativeTimeLive` node; the host formats against its own clock.

use core::fmt;

/// How a relative-time duration is spelled out. No `Default` — always explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RelTimeFormat {
    /// Abbreviated single unit with a direction affix: `7m ago` / `in 7m`.
    Short = 0,
}

/// Invalid [`RelTimeFormat`] wire discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRelTimeFormat(pub u8);

impl fmt::Display for InvalidRelTimeFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid relative-time format wire value {}", self.0)
    }
}

impl std::error::Error for InvalidRelTimeFormat {}

impl From<RelTimeFormat> for u8 {
    fn from(format: RelTimeFormat) -> Self {
        format as Self
    }
}

impl TryFrom<u8> for RelTimeFormat {
    type Error = InvalidRelTimeFormat;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Short),
            _ => Err(InvalidRelTimeFormat(value)),
        }
    }
}

/// Direction handling for a relative-time label.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RelTimeClamp {
    /// Sign of `now - anchor` picks direction; flips through `now`.
    #[default]
    Auto = 0,
    /// Always "ago"; reads `now` before the anchor.
    ElapsedOnly = 1,
    /// Always "in"; reads `now` once the anchor passes.
    RemainingOnly = 2,
}

/// Invalid [`RelTimeClamp`] wire discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRelTimeClamp(pub u8);

impl fmt::Display for InvalidRelTimeClamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid relative-time clamp wire value {}", self.0)
    }
}

impl std::error::Error for InvalidRelTimeClamp {}

impl From<RelTimeClamp> for u8 {
    fn from(clamp: RelTimeClamp) -> Self {
        clamp as Self
    }
}

impl TryFrom<u8> for RelTimeClamp {
    type Error = InvalidRelTimeClamp;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Auto),
            1 => Ok(Self::ElapsedOnly),
            2 => Ok(Self::RemainingOnly),
            _ => Err(InvalidRelTimeClamp(value)),
        }
    }
}

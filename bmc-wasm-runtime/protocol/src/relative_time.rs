// Copyright (C) 2026  Braiins Systems s.r.o.

//! Wire options for the `RelativeTimeLive` node; the host formats against its own clock.

use core::fmt;

/// Whether the unit label is abbreviated or spelled out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RelTimeLength {
    /// Abbreviated: `7m`, `2h`, `3d`.
    Short = 0,
    /// Full words, pluralized: `7 minutes`, `2 hours`, `1 day`.
    Long = 1,
}

/// How many whole units the label carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RelTimeSegments {
    /// The largest unit only (`7m`); ticks at that unit's boundary.
    Single = 0,
    /// The two largest units (`7m 30s`, smaller dropped when zero); ticks at
    /// the smaller unit.
    Double = 1,
}

/// How a `RelativeTimeLive` label is spelled — label width and unit count picked
/// independently. No `Default` — always explicit. `{ Short, Single }` → `7m`;
/// `{ Long, Double }` → `7 minutes 30 seconds`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelTimeFormat {
    pub length: RelTimeLength,
    pub segments: RelTimeSegments,
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
    /// Packed: bit 0 = length, bit 1 = segments.
    fn from(format: RelTimeFormat) -> Self {
        (format.length as Self) | ((format.segments as Self) << 1)
    }
}

impl TryFrom<u8> for RelTimeFormat {
    type Error = InvalidRelTimeFormat;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value & !0b11 != 0 {
            return Err(InvalidRelTimeFormat(value));
        }
        let length = if value & 0b01 == 0 {
            RelTimeLength::Short
        } else {
            RelTimeLength::Long
        };
        let segments = if value & 0b10 == 0 {
            RelTimeSegments::Single
        } else {
            RelTimeSegments::Double
        };
        Ok(Self { length, segments })
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

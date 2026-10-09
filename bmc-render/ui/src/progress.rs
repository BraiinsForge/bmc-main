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

//! Progress-bar wire types shared between SDK and host.

use core::fmt;

/// What a progress-bar node is, spelled on the wire as one byte. The SDK's
/// `ProgressMode` carries the fraction payload; this is its discriminant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ProgressKind {
    /// A draggable slider: fill plus drag thumb.
    #[default]
    Slider = 0,
    /// Unknown duration — animated indicator across full width.
    Indeterminate = 1,
    /// A passive meter: plain rounded bar, no drag thumb.
    Meter = 2,
}

/// Invalid [`ProgressKind`] wire discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidProgressKind(pub u8);

impl fmt::Display for InvalidProgressKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid progress kind wire value {}", self.0)
    }
}

impl std::error::Error for InvalidProgressKind {}

impl From<ProgressKind> for u8 {
    fn from(kind: ProgressKind) -> Self {
        kind as Self
    }
}

impl TryFrom<u8> for ProgressKind {
    type Error = InvalidProgressKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Slider),
            1 => Ok(Self::Indeterminate),
            2 => Ok(Self::Meter),
            _ => Err(InvalidProgressKind(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips_through_its_wire_byte() {
        for kind in [
            ProgressKind::Slider,
            ProgressKind::Indeterminate,
            ProgressKind::Meter,
        ] {
            assert_eq!(ProgressKind::try_from(u8::from(kind)), Ok(kind));
        }
        assert!(ProgressKind::try_from(3).is_err());
    }
}

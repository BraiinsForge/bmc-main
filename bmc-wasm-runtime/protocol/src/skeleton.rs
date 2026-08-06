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

//! Loading-placeholder wire vocabulary, shared by the SDK's builders and
//! the host's skeleton component.

use core::fmt;

/// Which Carbon skeleton a `NODE_SKELETON` stands for (the wire value
/// after the node byte). The host owns each one's geometry; the guest
/// names the role and the metrics of the text its slot would have held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum SkeletonKind {
    /// `.cds--skeleton__text`: a body line's bar, sized to a glyph count.
    #[default]
    Text = 0,
    /// `.cds--skeleton__heading`: a heading line's taller bar.
    Heading = 1,
    /// `.cds--skeleton__placeholder`: an explicit pixel box.
    Placeholder = 2,
    /// A bar of an explicit height that grows to span the row holding it.
    Fill = 3,
}

/// Invalid [`SkeletonKind`] wire discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidSkeletonKind(pub u8);

impl fmt::Display for InvalidSkeletonKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid skeleton kind wire value {}", self.0)
    }
}

impl std::error::Error for InvalidSkeletonKind {}

impl From<SkeletonKind> for u8 {
    fn from(kind: SkeletonKind) -> Self {
        kind as Self
    }
}

impl TryFrom<u8> for SkeletonKind {
    type Error = InvalidSkeletonKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Text),
            1 => Ok(Self::Heading),
            2 => Ok(Self::Placeholder),
            3 => Ok(Self::Fill),
            _ => Err(InvalidSkeletonKind(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_wire_byte() {
        for kind in [
            SkeletonKind::Text,
            SkeletonKind::Heading,
            SkeletonKind::Placeholder,
            SkeletonKind::Fill,
        ] {
            assert_eq!(SkeletonKind::try_from(u8::from(kind)), Ok(kind));
        }
    }

    #[test]
    fn unknown_bytes_are_rejected() {
        assert_eq!(SkeletonKind::try_from(9), Err(InvalidSkeletonKind(9)));
    }
}

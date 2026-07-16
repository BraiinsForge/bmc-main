// Copyright (C) 2023  Braiins Systems s.r.o.
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

use crate::impl_serde_as_string;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::num::ParseIntError;
use std::str::FromStr;

/// First 4 bytes (8 chars hex) of a commit hash.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct CommitHashShort(u32);

impl_serde_as_string!(CommitHashShort);

impl CommitHashShort {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

impl Display for CommitHashShort {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

impl FromStr for CommitHashShort {
    type Err = ParseIntError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        u32::from_str_radix(input, 16).map(Self)
    }
}

impl Debug for CommitHashShort {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl From<u32> for CommitHashShort {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

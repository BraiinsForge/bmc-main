// Copyright (C) 2023  Braiins Systems s.r.o.

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

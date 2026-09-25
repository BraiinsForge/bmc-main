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

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher as _};

use crate::ExecutionId;

/// One claimable offer at a time.
/// Storing the same upgrade again keeps its ID, so clients that checked it share one;
/// only the start that names the ID takes the offer out, so an offer runs at most once.
#[derive(Debug)]
pub struct OfferSlot<T> {
    current: Option<(ExecutionId, T)>,
}

impl<T> Default for OfferSlot<T> {
    fn default() -> Self {
        Self { current: None }
    }
}

impl<T: Fingerprint> OfferSlot<T> {
    /// Returns the current ID when `offer` has the stored offer's fingerprint, a new one otherwise;
    /// either way `offer` replaces the stored value.
    pub fn store(&mut self, offer: T) -> ExecutionId {
        let id = if let Some((id, stored)) = &self.current
            && stored.fingerprint() == offer.fingerprint()
        {
            *id
        } else {
            ExecutionId::new()
        };
        self.current = Some((id, offer));
        id
    }
}

impl<T> OfferSlot<T> {
    /// Takes the offer out when `id` names it; any other ID leaves the stored offer in place.
    pub fn claim(&mut self, id: ExecutionId) -> Option<T> {
        match self.current.take() {
            Some((current, offer)) if current == id => Some(offer),
            other => {
                self.current = other;
                None
            }
        }
    }

    pub fn invalidate(&mut self) {
        self.current = None;
    }
}

/// What an offer would do to the device; offers with equal fingerprints share one ID.
pub trait Fingerprint {
    fn fingerprint(&self) -> OfferFingerprint<'_>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum OfferFingerprint<'a> {
    /// `install` is what the firmware run installs after the flash, `None` when it skips packages.
    Firmware {
        hash: &'a str,
        install: Option<BTreeSet<&'a str>>,
    },
    /// `plan` digests what a package run re-plans `install` against: the package index and the
    /// installed profile on a device, whatever stands in for them in a mock.
    Packages {
        plan: u64,
        install: BTreeSet<&'a str>,
    },
}

/// Equal values digest equally; the digest is only comparable within one build.
#[must_use]
pub fn digest(value: &impl Hash) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// The requested installs as the set they are, so their order and repeats do not split offers.
#[must_use]
pub fn install_set(install: &[String]) -> BTreeSet<&str> {
    install.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests;

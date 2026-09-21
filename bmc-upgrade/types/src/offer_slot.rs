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

use crate::ExecutionId;

/// One claimable offer at a time: a check stores the freshly prepared offer under a new ID,
/// and only the start that names that ID takes it out, so an offer runs at most once.
#[derive(Debug)]
pub struct OfferSlot<T> {
    current: Option<(ExecutionId, T)>,
}

impl<T> Default for OfferSlot<T> {
    fn default() -> Self {
        Self { current: None }
    }
}

impl<T> OfferSlot<T> {
    /// Replaces whatever was stored and returns the ID the new offer answers to.
    pub fn store(&mut self, offer: T) -> ExecutionId {
        let id = ExecutionId::new();
        self.current = Some((id, offer));
        id
    }

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

#[cfg(test)]
mod tests;

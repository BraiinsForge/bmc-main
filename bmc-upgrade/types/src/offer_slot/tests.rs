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

use super::{Fingerprint, OfferFingerprint, OfferSlot};
use crate::ExecutionId;

impl Fingerprint for &str {
    fn fingerprint(&self) -> OfferFingerprint<'_> {
        OfferFingerprint::Firmware {
            hash: self,
            install: None,
        }
    }
}

/// The number is the value a repeated check refreshes; only the name is fingerprinted.
impl Fingerprint for (&str, u8) {
    fn fingerprint(&self) -> OfferFingerprint<'_> {
        self.0.fingerprint()
    }
}

#[test]
fn the_stored_offer_is_claimed_once() {
    let mut slot = OfferSlot::default();
    let id = slot.store("offer");
    assert_eq!(slot.claim(id), Some("offer"));
    assert_eq!(slot.claim(id), None);
}

#[test]
fn a_foreign_id_leaves_the_offer_claimable() {
    let mut slot = OfferSlot::default();
    let id = slot.store("offer");
    assert_eq!(slot.claim(ExecutionId::new()), None);
    assert_eq!(slot.claim(id), Some("offer"));
}

#[test]
fn a_different_offer_retires_the_previous_id() {
    let mut slot = OfferSlot::default();
    let old = slot.store("old");
    let new = slot.store("new");
    assert_ne!(old, new);
    assert_eq!(slot.claim(old), None);
    assert_eq!(slot.claim(new), Some("new"));
}

#[test]
fn the_same_offer_keeps_its_id_and_takes_the_new_value() {
    let mut slot = OfferSlot::default();
    let first = slot.store(("offer", 1));
    let second = slot.store(("offer", 2));
    assert_eq!(first, second);
    assert_eq!(slot.claim(first), Some(("offer", 2)));
}

#[test]
fn invalidating_drops_the_offer() {
    let mut slot = OfferSlot::default();
    let id = slot.store("offer");
    slot.invalidate();
    assert_eq!(slot.claim(id), None);
}

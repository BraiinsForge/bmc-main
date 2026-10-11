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

//! The tag reservations every renderer asset registry keeps.
//!
//! A tag holds its id from reservation until eviction.
//! It is resident while a payload is stored under that id and suspended otherwise,
//! so restoring a suspended tag hands back the id its widget already holds.
//! Payloads leave only through return values, for their owner to free.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

use bmc_wasm_protocol::{IdPool, ReusableId, tag_matches_prefix};

use crate::renderer::{AssetSuspendResult, AssetTagState};

/// Why [`TagReservations::register_with`] stored nothing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RegisterError<E> {
    /// The tag was unknown and every id below the pool's cap is taken.
    Exhausted,
    Load(E),
}

/// What [`TagReservations::suspend`] did, with the payload it took for its owner to free.
#[derive(Debug)]
pub(crate) enum Suspension<Id, P> {
    Suspended { id: Id, payload: P },
    AlreadySuspended(Id),
    Unknown,
}

impl<Id: Copy, P> Suspension<Id, P> {
    pub(crate) fn result(&self) -> AssetSuspendResult<Id> {
        match *self {
            Self::Suspended { id, .. } => AssetSuspendResult::Suspended(id),
            Self::AlreadySuspended(id) => AssetSuspendResult::AlreadySuspended(id),
            Self::Unknown => AssetSuspendResult::Unknown,
        }
    }
}

#[derive(Clone, Copy)]
struct Reservation<Id, M> {
    id: Id,
    meta: M,
}

/// One asset kind's tag reservations, with the payloads of the resident ones.
///
/// `M` is metadata a reservation keeps across suspension, such as bitmap sampling.
pub(crate) struct TagReservations<Id: ReusableId, P, M = ()> {
    by_tag: HashMap<String, Reservation<Id, M>>,
    payloads: HashMap<Id, P>,
    ids: IdPool<Id>,
    exclusive_cap: u16,
}

impl<Id: ReusableId, P, M> fmt::Debug for TagReservations<Id, P, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TagReservations")
            .field("tags", &self.by_tag.len())
            .field("resident", &self.payloads.len())
            .field("ids", &self.ids)
            .finish_non_exhaustive()
    }
}

impl<Id, P, M> TagReservations<Id, P, M>
where
    Id: ReusableId + Eq + Hash,
    M: Copy,
{
    pub(crate) fn new(exclusive_cap: u16) -> Self {
        Self {
            by_tag: HashMap::new(),
            payloads: HashMap::new(),
            ids: IdPool::new(exclusive_cap),
            exclusive_cap,
        }
    }

    pub(crate) fn tag_state(&self, tag: &str) -> AssetTagState<Id> {
        let Some(reservation) = self.by_tag.get(tag) else {
            return AssetTagState::Unknown;
        };
        if self.payloads.contains_key(&reservation.id) {
            AssetTagState::Resident(reservation.id)
        } else {
            AssetTagState::Suspended(reservation.id)
        }
    }

    pub(crate) fn meta(&self, tag: &str) -> Option<M> {
        self.by_tag.get(tag).map(|reservation| reservation.meta)
    }

    /// The id `tag` holds, reserving one under `meta` if the tag is unknown.
    /// `None` when the tag is unknown and the pool is exhausted.
    pub(crate) fn reserve(&mut self, tag: &str, meta: M) -> Option<Id> {
        if let Some(reservation) = self.by_tag.get(tag) {
            return Some(reservation.id);
        }
        let id = self.ids.alloc()?;
        self.by_tag.insert(tag.to_owned(), Reservation { id, meta });
        Some(id)
    }

    /// Store a payload for `tag` unless one is already resident.
    ///
    /// `load` gets the reservation's metadata:
    /// the stored one for a suspended tag, `meta` for an unknown one.
    /// A failed load keeps a suspended reservation
    /// but releases one made by this call, so a broken payload burns no id.
    pub(crate) fn register_with<E>(
        &mut self,
        tag: &str,
        meta: M,
        load: impl FnOnce(M) -> Result<P, E>,
    ) -> Result<Id, RegisterError<E>> {
        let stored = self.meta(tag);
        let id = self.reserve(tag, meta).ok_or(RegisterError::Exhausted)?;
        if self.payloads.contains_key(&id) {
            return Ok(id);
        }
        match load(stored.unwrap_or(meta)) {
            Ok(payload) => {
                self.payloads.insert(id, payload);
                Ok(id)
            }
            Err(error) => {
                if stored.is_none() {
                    self.by_tag.remove(tag);
                    self.ids.release(id);
                }
                Err(RegisterError::Load(error))
            }
        }
    }

    /// Store `payload` for `tag` under `meta`, reserving the tag if it is unknown,
    /// and return the payload it displaced.
    /// When the tag is unknown and no id is free, `payload` comes back as the error.
    pub(crate) fn replace(&mut self, tag: &str, meta: M, payload: P) -> Result<(Id, Option<P>), P> {
        let id = if let Some(reservation) = self.by_tag.get_mut(tag) {
            reservation.meta = meta;
            reservation.id
        } else {
            let Some(id) = self.ids.alloc() else {
                return Err(payload);
            };
            self.by_tag.insert(tag.to_owned(), Reservation { id, meta });
            id
        };
        Ok((id, self.payloads.insert(id, payload)))
    }

    /// Take `tag`'s payload while it keeps its id.
    pub(crate) fn suspend(&mut self, tag: &str) -> Suspension<Id, P> {
        let Some(reservation) = self.by_tag.get(tag) else {
            return Suspension::Unknown;
        };
        let id = reservation.id;
        match self.payloads.remove(&id) {
            Some(payload) => Suspension::Suspended { id, payload },
            None => Suspension::AlreadySuspended(id),
        }
    }

    /// Forget `tag` and release its id, returning the id and any payload it held.
    pub(crate) fn evict(&mut self, tag: &str) -> Option<(Id, Option<P>)> {
        let reservation = self.by_tag.remove(tag)?;
        let payload = self.payloads.remove(&reservation.id);
        self.ids.release(reservation.id);
        Some((reservation.id, payload))
    }

    /// Evict every tag that is `prefix` or a segment below it.
    pub(crate) fn evict_prefix(&mut self, prefix: &str) -> Vec<(Id, Option<P>)> {
        let tags: Vec<String> = self
            .by_tag
            .keys()
            .filter(|tag| tag_matches_prefix(tag, prefix))
            .cloned()
            .collect();
        tags.iter().filter_map(|tag| self.evict(tag)).collect()
    }

    /// Forget every tag and restart the ids, returning all payloads.
    pub(crate) fn clear(&mut self) -> Vec<P> {
        self.by_tag.clear();
        self.ids = IdPool::new(self.exclusive_cap);
        self.payloads.drain().map(|(_, payload)| payload).collect()
    }

    pub(crate) fn payload(&self, id: Id) -> Option<&P> {
        self.payloads.get(&id)
    }

    pub(crate) fn payloads(&self) -> impl Iterator<Item = &P> {
        self.payloads.values()
    }

    pub(crate) fn has_resident_prefix(&self, prefix: &str) -> bool {
        self.by_tag.iter().any(|(tag, reservation)| {
            tag_matches_prefix(tag, prefix) && self.payloads.contains_key(&reservation.id)
        })
    }
}

impl<Id: ReusableId, M> TagReservations<Id, (), M> {
    /// The same tags and ids, ready to hold payloads of type `P`, every tag suspended.
    pub(crate) fn into_payloads<P>(self) -> TagReservations<Id, P, M> {
        let Self {
            by_tag,
            ids,
            exclusive_cap,
            ..
        } = self;
        TagReservations {
            by_tag,
            payloads: HashMap::new(),
            ids,
            exclusive_cap,
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_ui::BitmapId;

    use super::*;

    type Table = TagReservations<BitmapId, &'static str, bool>;

    fn table() -> Table {
        TagReservations::new(u16::MAX)
    }

    #[test]
    fn a_fresh_reservation_reads_as_suspended() {
        let mut table = table();
        let id = table.reserve("w:a", false);

        assert_eq!(
            id.map(AssetTagState::Suspended),
            Some(table.tag_state("w:a"))
        );
    }

    #[test]
    fn a_resident_tag_skips_the_loader() {
        let mut table = table();
        let id = table.register_with("w:a", false, |_| Ok::<_, ()>("first"));

        assert_eq!(table.register_with("w:a", false, |_| Err(())), id);
    }

    #[test]
    fn restoring_a_suspended_tag_loads_with_its_stored_metadata() {
        let mut table = table();
        table.reserve("w:a", true);
        let mut seen = None;
        let _ = table.register_with("w:a", false, |meta| {
            seen = Some(meta);
            Ok::<_, ()>("payload")
        });

        assert_eq!(seen, Some(true));
    }

    #[test]
    fn a_failed_fresh_load_releases_its_id() {
        let mut table = table();
        let first = table.register_with("w:broken", false, |_| Err::<&str, _>(()));
        assert_eq!(first, Err(RegisterError::Load(())));

        assert_eq!(
            table.reserve("w:next", false).map(BitmapId::to_wire),
            Some(1)
        );
    }

    #[test]
    fn a_failed_restore_keeps_the_suspended_reservation() {
        let mut table = table();
        let id = table.reserve("w:a", false).expect("BUG: the first id fits");
        let _ = table.register_with("w:a", false, |_| Err::<&str, _>(()));

        assert_eq!(table.tag_state("w:a"), AssetTagState::Suspended(id));
    }

    #[test]
    fn replace_hands_back_the_displaced_payload() {
        let mut table = table();
        let _ = table.register_with("w:a", false, |_| Ok::<_, ()>("old"));

        assert_eq!(
            table.replace("w:a", true, "new").map(|(_, old)| old),
            Ok(Some("old"))
        );
        assert_eq!(table.meta("w:a"), Some(true));
    }

    #[test]
    fn replace_hands_the_payload_back_when_no_id_is_free() {
        let mut table: Table = TagReservations::new(2);
        table.reserve("w:a", false);

        assert_eq!(table.replace("w:b", false, "new"), Err("new"));
    }

    #[test]
    fn evicting_a_prefix_sweeps_only_its_own_segment() {
        let mut table = table();
        let _ = table.register_with("w1:a", false, |_| Ok::<_, ()>("a"));
        table.reserve("w1:b", false);
        let _ = table.register_with("w10:c", false, |_| Ok::<_, ()>("c"));

        let mut payloads: Vec<_> = table
            .evict_prefix("w1")
            .into_iter()
            .map(|(_, payload)| payload)
            .collect();
        payloads.sort_unstable();
        assert_eq!(payloads, [None, Some("a")]);
    }

    #[test]
    fn an_evicted_id_is_reused_before_a_fresh_one() {
        let mut table = table();
        let first = table.reserve("w:a", false);
        table.reserve("w:b", false);
        table.evict("w:a");

        assert_eq!(table.reserve("w:c", false), first);
    }

    #[test]
    fn taking_on_payloads_keeps_every_tag_its_id() {
        let mut pending: TagReservations<BitmapId, (), bool> = TagReservations::new(u16::MAX);
        let kept = pending.reserve("w:a", true);
        let released = pending.reserve("w:b", false);
        pending.evict("w:b");

        let mut live: Table = pending.into_payloads();

        assert_eq!(
            kept.map(AssetTagState::Suspended),
            Some(live.tag_state("w:a"))
        );
        assert_eq!(live.meta("w:a"), Some(true), "metadata carries over");
        assert_eq!(live.reserve("w:c", false), released, "freed ids carry over");
    }
}

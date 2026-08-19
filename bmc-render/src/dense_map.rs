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

//! Keyed collection with contiguous values: dense `Vec`, sparse key → index.
//!
//! For collections **iterated far more often than looked up**, holding small
//! values. A `HashMap` scatters values across buckets, so a full pass chases
//! pointers; here they sit in one run with no holes and no per-element branch,
//! and the map holds only indices. Keyed access stays O(1) with one extra
//! indirection.
//!
//! Not a general `HashMap` replacement — pick this only when the iteration
//! pattern justifies keeping two containers in sync.
//!
//! # Removal keeps the run dense
//!
//! Removal cannot shift the tail: that would invalidate every index past the
//! hole. Instead the last entry is swapped into the vacated slot and its index
//! repaired — so there are no tombstones to skip and no free list to consult,
//! and `values` is a straight walk.
//!
//! The key is stored with its value rather than only in the map, because a swap
//! has to know which key just moved. Iteration order is therefore an
//! implementation detail; callers must not rely on it.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Index;

/// A keyed collection whose values live in one contiguous run.
///
/// `K` keys the sparse index; `(K, V)` pairs are stored densely.
#[derive(Debug)]
pub struct DenseMap<K, V> {
    /// Key/value pairs with no gaps. Iterated directly; the reason this exists.
    entries: Vec<(K, V)>,
    /// Key to position in `entries`.
    index: HashMap<K, usize>,
}

impl<K: Hash + Eq + Clone, V> DenseMap<K, V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            index: HashMap::with_capacity(capacity),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    #[must_use]
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.entries.get(*self.index.get(key)?).map(|(_, v)| v)
    }

    #[must_use]
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.entries.get_mut(*self.index.get(key)?).map(|(_, v)| v)
    }

    #[must_use]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.index.contains_key(key)
    }

    /// Insert or overwrite, returning the previous value if the key was present.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if let Some(&slot) = self.index.get(&key) {
            return Some(std::mem::replace(&mut self.entries[slot].1, value));
        }
        self.index.insert(key.clone(), self.entries.len());
        self.entries.push((key, value));
        None
    }

    /// Remove a key and hand back its value, keeping the run dense.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let slot = self.index.remove(key)?;
        let (_, value) = self.entries.swap_remove(slot);
        self.repair_after_swap(slot);
        Some(value)
    }

    /// Point the entry now sitting at `slot` — swapped in from the tail — at its
    /// new position. A no-op when the removed entry was already last.
    fn repair_after_swap(&mut self, slot: usize) {
        if let Some((moved_key, _)) = self.entries.get(slot)
            && let Some(position) = self.index.get_mut(moved_key)
        {
            *position = slot;
        }
    }

    /// Live values, in dense order — the fast path this type exists for.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }

    /// Live values mutably, in dense order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries.iter_mut().map(|(_, v)| v)
    }

    /// Key/value pairs, in dense order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// Drop every entry for which `keep` returns `false`.
    ///
    /// Walks the dense run, swapping survivors down over the dropped, so a
    /// generation sweep stays contiguous.
    pub fn retain(&mut self, mut keep: impl FnMut(&V) -> bool) {
        let mut slot = 0;
        while slot < self.entries.len() {
            if keep(&self.entries[slot].1) {
                slot += 1;
                continue;
            }
            let (key, _) = self.entries.swap_remove(slot);
            self.index.remove(&key);
            self.repair_after_swap(slot);
            // `slot` now holds the swapped-in entry, which is still unchecked.
        }
    }
}

impl<K: Hash + Eq + Clone, V> Default for DenseMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V, Q> Index<&Q> for DenseMap<K, V>
where
    K: Eq + Hash + Clone + Borrow<Q>,
    Q: Eq + Hash + ?Sized,
{
    type Output = V;

    fn index(&self, key: &Q) -> &Self::Output {
        self.get(key).expect("BUG: no entry for key")
    }
}

#[cfg(test)]
mod tests {
    use super::DenseMap;

    #[test]
    fn get_and_overwrite_round_trip() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        assert!(map.is_empty());

        assert_eq!(map.insert("a".to_owned(), 1), None);
        assert_eq!(map.insert("b".to_owned(), 2), None);
        assert_eq!(map.insert("a".to_owned(), 10), Some(1));

        assert_eq!(map.len(), 2);
        assert_eq!(map.get("a"), Some(&10));
        assert_eq!(map["b"], 2);
        assert_eq!(map.get("missing"), None);
    }

    #[test]
    fn removal_repairs_the_moved_entrys_index() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        for (k, v) in [("a", 1), ("b", 2), ("c", 3)] {
            map.insert(k.to_owned(), v);
        }

        assert_eq!(map.remove("b"), Some(2));
        assert_eq!(map.remove("b"), None, "a second remove finds nothing live");
        assert_eq!(map.len(), 2);

        // Removing `b` swaps `c` into its slot, so `c`'s index has to be
        // repaired or it reads `b`'s old position.
        assert_eq!(map.get("c"), Some(&3));
        assert_eq!(map.get("a"), Some(&1));
    }

    #[test]
    fn removal_keeps_the_run_dense() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        map.insert("a".to_owned(), 1);
        map.insert("b".to_owned(), 2);
        map.remove("a");

        // A different key claims the free slot, so the dense array holds.
        map.insert("c".to_owned(), 3);
        assert_eq!(map.len(), 2);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("c"), Some(&3));
        assert_eq!(map.get("a"), None);
    }

    #[test]
    fn reinserting_a_removed_key_reuses_the_freed_slot() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        map.insert("a".to_owned(), 1);
        map.remove("a");
        assert_eq!(map.insert("a".to_owned(), 9), None, "the key was gone");

        assert_eq!(map.len(), 1);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("a"), Some(&9));
    }

    /// A freed slot is claimable by any key, so the removed key must not stay
    /// in the index — it would alias whatever value lands in its old slot.
    #[test]
    fn a_removed_key_does_not_alias_the_value_that_takes_its_slot() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        map.insert("a".to_owned(), 1);
        map.remove("a");
        map.insert("b".to_owned(), 2);

        assert_eq!(map.get("b"), Some(&2));
        assert_eq!(map.get("a"), None);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn iteration_yields_every_live_value() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        for (k, v) in [("a", 1), ("b", 2), ("c", 3)] {
            map.insert(k.to_owned(), v);
        }
        map.remove("b");

        let mut seen: Vec<u32> = map.values().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 3]);
    }

    /// Same hazard as `remove`: a dropped entry must lose its key, or that key
    /// aliases whatever value later claims the freed slot.
    #[test]
    fn retain_drops_keys_not_just_values() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        map.insert("a".to_owned(), 1);
        map.insert("b".to_owned(), 2);

        map.retain(|v| *v != 1);
        map.insert("c".to_owned(), 3);

        assert_eq!(map.get("a"), None, "dropped key must not alias slot reuse");
        assert_eq!(map.get("c"), Some(&3));
        assert_eq!(map.len(), 2);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn retain_drops_and_keeps_the_run_dense() {
        let mut map: DenseMap<String, u32> = DenseMap::new();
        for (k, v) in [("a", 1), ("b", 2), ("c", 3), ("d", 4)] {
            map.insert(k.to_owned(), v);
        }

        map.retain(|v| v % 2 == 0);

        assert_eq!(map.len(), 2);
        let mut seen: Vec<u32> = map.values().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![2, 4]);

        // Survivors stay reachable by key after being swapped down.
        assert_eq!(map.get("b"), Some(&2));
        assert_eq!(map.get("d"), Some(&4));
        assert_eq!(map.get("a"), None);
        assert_eq!(map.get("c"), None);
    }
}

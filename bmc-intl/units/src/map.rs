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

//! Maps read in place from a file's bytes, their order checked by [`crate::Units::from_bytes`].
//!
//! Laid out as zerovec's `ZeroMap` and `ZeroMap2d` write theirs,
//! which take a file's key order on trust (zerovec#1433):
//! read from unsorted keys, their lookups miss in release and panic in debug.
//! The check runs after the read, not in it: postcard drops the text of an error raised inside serde.

use std::collections::BTreeMap;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use zerovec::vecs::VarZeroVecOwned;
use zerovec::{VarZeroVec, ZeroVec};

use crate::Plural;

/// Why a map was not built: its text past what one of its vectors indexes,
/// 64 KiB with zerovec's default 16-bit offsets.
const TOO_LARGE: &str = "more text than one map holds";

fn strings(items: &[&str]) -> Result<VarZeroVec<'static, str>, &'static str> {
    VarZeroVecOwned::try_from_elements(items)
        .map(VarZeroVec::from)
        .map_err(|_| TOO_LARGE)
}

/// Text to text: `10p9` → `G{0}`.
///
/// Only postcard is read back; the readable form is a dump.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Map<'data> {
    #[serde(borrow)]
    keys: VarZeroVec<'data, str>,
    #[serde(borrow)]
    values: VarZeroVec<'data, str>,
}

impl Map<'_> {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(self.keys.binary_search(key).ok()?)
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.keys.iter().zip(self.values.iter())
    }

    /// Why a lookup into this map could miss what it holds, if it could.
    pub(crate) fn check(&self) -> Result<(), &'static str> {
        if self.keys.len() != self.values.len() {
            return Err("more keys than values, or fewer");
        }
        if !self.keys.iter().is_sorted_by(|key, next| key < next) {
            return Err("keys out of order");
        }
        Ok(())
    }
}

impl Map<'static> {
    /// `entries` in any order.
    ///
    /// # Errors
    /// When the keys or the values are more text than the map indexes.
    pub fn new<'a>(
        entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, &'static str> {
        let sorted: BTreeMap<&str, &str> = entries.into_iter().collect();
        Ok(Self {
            keys: strings(&sorted.keys().copied().collect::<Vec<_>>())?,
            values: strings(&sorted.values().copied().collect::<Vec<_>>())?,
        })
    }
}

/// Text, then a plural category, to text: `temperature-celsius` → few → `{0} °C`.
///
/// Only postcard is read back; the readable form is a dump.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Map2d<'data> {
    #[serde(borrow)]
    keys0: VarZeroVec<'data, str>,
    /// Where each first key's row ends in `keys1` and `values`.
    #[serde(borrow)]
    joiner: ZeroVec<'data, u32>,
    #[serde(borrow)]
    keys1: ZeroVec<'data, Plural>,
    #[serde(borrow)]
    values: VarZeroVec<'data, str>,
}

impl Map2d<'_> {
    #[must_use]
    pub fn get(&self, key0: &str, key1: Plural) -> Option<&str> {
        let row = self.row(self.keys0.binary_search(key0).ok()?);
        let found = self
            .keys1
            .as_slice()
            .get_subslice(row.clone())?
            .binary_search(&key1)
            .ok()?;
        self.values.get(row.start + found)
    }

    /// Where row `index` ends in `keys1` and `values`; 0 past the last row.
    fn end(&self, index: usize) -> usize {
        self.joiner.get(index).map_or(0, |end| {
            usize::try_from(end).expect("BUG: a u32 fits the device's usize")
        })
    }

    /// Where each row ends, after a leading 0 where the first begins.
    fn ends(&self) -> impl Iterator<Item = usize> {
        std::iter::once(0).chain((0..self.joiner.len()).map(|index| self.end(index)))
    }

    /// The positions of row `index` in `keys1` and `values`.
    fn row(&self, index: usize) -> std::ops::Range<usize> {
        index
            .checked_sub(1)
            .map_or(0, |previous| self.end(previous))..self.end(index)
    }

    fn row_entries(&self, index: usize) -> impl Iterator<Item = (Plural, &str)> {
        self.row(index)
            .map_while(|at| Some((self.keys1.get(at)?, self.values.get(at)?)))
    }

    /// Why a lookup into this map could miss what it holds, if it could.
    pub(crate) fn check(&self) -> Result<(), &'static str> {
        // Every row holds at least one entry, and the last ends where the entries do.
        let rows_add_up = self.keys0.len() == self.joiner.len()
            && self.keys1.len() == self.values.len()
            && self.ends().last() == Some(self.keys1.len())
            && self.ends().is_sorted_by(|end, next| end < next);
        if !rows_add_up {
            return Err("rows that do not add up");
        }
        if !self.keys0.iter().is_sorted_by(|key, next| key < next) {
            return Err("keys out of order");
        }
        if !(0..self.keys0.len()).all(|index| {
            self.row_entries(index)
                .map(|(key1, _)| key1)
                .is_sorted_by(|key1, next| key1 < next)
        }) {
            return Err("plural categories out of order within a key");
        }
        Ok(())
    }
}

impl Map2d<'static> {
    /// `entries` in any order.
    ///
    /// # Errors
    /// When the first keys or the values are more text than the map indexes.
    pub fn new<'a>(
        entries: impl IntoIterator<Item = (&'a str, Plural, &'a str)>,
    ) -> Result<Self, &'static str> {
        let sorted: BTreeMap<(&str, Plural), &str> = entries
            .into_iter()
            .map(|(key0, key1, value)| ((key0, key1), value))
            .collect();
        let mut keys0: Vec<&str> = Vec::new();
        let mut joiner: Vec<u32> = Vec::new();
        for (index, &(key0, _)) in sorted.keys().enumerate() {
            if keys0.last() != Some(&key0) {
                keys0.push(key0);
                joiner.push(0);
            }
            *joiner.last_mut().expect("BUG: a row was just begun") = u32::try_from(index + 1)
                .expect("BUG: a language has far fewer patterns than u32 counts");
        }
        Ok(Self {
            keys0: strings(&keys0)?,
            joiner: ZeroVec::alloc_from_slice(&joiner),
            keys1: sorted.keys().map(|&(_, key1)| key1).collect(),
            values: strings(&sorted.values().copied().collect::<Vec<_>>())?,
        })
    }
}

impl Serialize for Map<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_map(self.iter())
        } else {
            (&self.keys, &self.values).serialize(serializer)
        }
    }
}

/// Row `.1` of the map `.0`, as the readable dump shows it.
struct Row<'m, 'data>(&'m Map2d<'data>, usize);

impl Serialize for Row<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.row_entries(self.1))
    }
}

impl Serialize for Map2d<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            let mut map = serializer.serialize_map(Some(self.keys0.len()))?;
            for (index, key0) in self.keys0.iter().enumerate() {
                map.serialize_entry(key0, &Row(self, index))?;
            }
            map.end()
        } else {
            (&self.keys0, &self.joiner, &self.keys1, &self.values).serialize(serializer)
        }
    }
}

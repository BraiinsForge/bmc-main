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

//! A language's unit patterns, cut from CLDR by `intl cldr-units`
//! and read in place from the bytes of its pack's `units.postcard`.
//!
//! The examples are Czech, as the pinned CLDR has it.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use yoke::{Yoke, Yokeable};
use zerovec::ZeroVec;

mod map;

pub use map::{Map, Map2d};

/// The layout this code writes and reads.
pub const FORMAT: u16 = 1;

pub const FILE: &str = "units.postcard";

/// A plural category, as CLDR names them.
#[zerovec::make_ule(PluralULE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Plural {
    Zero = 0,
    One = 1,
    Two = 2,
    Few = 3,
    Many = 4,
    Other = 5,
}

/// One language's unit data, borrowed from the bytes of its `units.postcard`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Yokeable)]
pub struct Units<'data> {
    /// [`FORMAT`] for this code.
    pub format: u16,
    /// `5 °C`, `G{0}`, `{0}/h`.
    #[serde(borrow)]
    pub short: Width<'data>,
    /// The tightest form, which CLDR may make the same as short: `5 °C` in Czech too.
    #[serde(borrow)]
    pub narrow: Width<'data>,
    /// A range of two numbers: `{0}–{1}`, read as `2–4`.
    #[serde(borrow)]
    pub range: Cow<'data, str>,
    /// The plural a range takes from the plurals of its ends, as (start, end, range):
    /// Czech (one, few, few), so `1–3` reads in few's form, as `3` does.
    /// In no particular order.
    #[serde(borrow)]
    pub plural_ranges: ZeroVec<'data, (Plural, Plural, Plural)>,
    /// The plural each part of a compound takes, from `grammaticalFeatures`.
    /// Czech has no entry of its own and takes `und`'s.
    pub derivations: Derivations,
    /// Where a currency goes beside its number: `#,##0.00 ¤`, `1 234,50 Kč`.
    #[serde(borrow)]
    pub currency: Currency<'data>,
}

/// One width's patterns, keyed by CLDR's own names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Width<'data> {
    /// `temperature-celsius` → one, few, many, other → `{0} °C`.
    /// A unit may bring `other` alone, as Czech `duration-hour` does: `{0} h`.
    #[serde(borrow)]
    pub units: Map2d<'data>,
    /// `duration-hour` → `{0}/h`, where CLDR has a unit's own form after "per".
    #[serde(borrow)]
    pub per_unit: Map<'data>,
    /// `10p9` → `G{0}`, `1024p3` → `Gi{0}`: a prefix put onto a unit's symbol.
    #[serde(borrow)]
    pub prefixes: Map<'data>,
    /// `per` → `{0}/{1}`, `times` → `{0}⋅{1}`: two units joined.
    #[serde(borrow)]
    pub compounds: Map<'data>,
    /// `power2` → other → `{0}²`, `power3` → other → `{0}³`.
    #[serde(borrow)]
    pub powers: Map2d<'data>,
}

impl Width<'_> {
    /// The first of this width's maps whose lookups could miss what it holds.
    fn check(&self, width: &'static str) -> Result<(), Refused> {
        [
            ("units", self.units.check()),
            ("per_unit", self.per_unit.check()),
            ("prefixes", self.prefixes.check()),
            ("compounds", self.compounds.check()),
            ("powers", self.powers.check()),
        ]
        .into_iter()
        .try_for_each(|(map, checked)| checked.map_err(|why| Refused::Map { width, map, why }))
    }
}

/// For each structure, the plural each of its two parts takes.
///
/// `und`'s, which Czech takes: in `5 km/h` the kilometres take the count's plural
/// and the hour is singular, `[Compound, Fixed(One)]`; in `5 kW⋅h`, `5 m²` and `5 GHz`
/// the first part is singular and the last carries the count, `[Fixed(One), Compound]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivations {
    pub per: [Derive; 2],
    pub times: [Derive; 2],
    pub power: [Derive; 2],
    pub prefix: [Derive; 2],
}

/// The plural a compound's part takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Derive {
    /// The compound's own, as the count selects it.
    Compound,
    /// Always this one, whatever the count.
    Fixed(Plural),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Currency<'data> {
    /// `#,##0.00 ¤`: the number, then the symbol.
    #[serde(borrow)]
    pub standard: Cow<'data, str>,
    /// The form for a symbol of letters, such as `CHF`, beside the number:
    /// `¤ #,##0.00` in Chinese, where `standard` is `¤#,##0.00`.
    #[serde(borrow)]
    pub alpha_next_to_number: Cow<'data, str>,
    /// The space put between a letter-symbol and the number on either side:
    /// `Kč` gets a no-break space, a symbol such as `$` none.
    #[serde(borrow)]
    pub before: Spacing<'data>,
    #[serde(borrow)]
    pub after: Spacing<'data>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spacing<'data> {
    /// `[[:^S:]&[:^Z:]]`: a symbol that is neither a symbol character nor a space, as `Kč` is.
    #[serde(borrow)]
    pub currency_match: Cow<'data, str>,
    /// `[:digit:]`: the number's digit beside it.
    #[serde(borrow)]
    pub surrounding_match: Cow<'data, str>,
    /// A no-break space, U+00A0.
    #[serde(borrow)]
    pub insert_between: Cow<'data, str>,
}

#[derive(Debug, PartialEq)]
pub enum Refused {
    Format(u16),
    Malformed(postcard::Error),
    /// Bytes left after the units.
    Trailing(usize),
    /// A map whose lookups could miss what it holds.
    Map {
        width: &'static str,
        map: &'static str,
        why: &'static str,
    },
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(found) => write!(f, "units format {found}, this code reads {FORMAT}"),
            Self::Malformed(error) => write!(f, "malformed: {error}"),
            Self::Trailing(left) => write!(f, "{left} bytes after the units"),
            Self::Map { width, map, why } => write!(f, "`{width}.{map}`: {why}"),
        }
    }
}

impl std::error::Error for Refused {}

impl<'data> Units<'data> {
    /// `None` where CLDR gives none.
    #[must_use]
    pub fn plural_range(&self, start: Plural, end: Plural) -> Option<Plural> {
        self.plural_ranges
            .iter()
            .find(|&(from, to, _)| from == start && to == end)
            .map(|(_, _, range)| range)
    }

    /// Borrowed from `bytes`, their maps checked.
    ///
    /// # Errors
    /// Bytes in another layout, not units at all, or a map whose lookups could miss what it holds.
    pub fn from_bytes(bytes: &'data [u8]) -> Result<Self, Refused> {
        let (format, _) = postcard::take_from_bytes::<u16>(bytes).map_err(Refused::Malformed)?;
        if format != FORMAT {
            return Err(Refused::Format(format));
        }
        let units: Self = match postcard::take_from_bytes(bytes).map_err(Refused::Malformed)? {
            (units, []) => units,
            (_, left) => return Err(Refused::Trailing(left.len())),
        };
        units.short.check("short")?;
        units.narrow.check("narrow")?;
        Ok(units)
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("BUG: postcard writes to memory, which never fails")
    }
}

/// Units read in place from the bytes they keep.
pub type Loaded = Yoke<Units<'static>, Arc<[u8]>>;

/// # Errors
/// As [`Units::from_bytes`].
pub fn load(bytes: Arc<[u8]>) -> Result<Loaded, Refused> {
    Yoke::try_attach_to_cart(bytes, |bytes| Units::from_bytes(bytes))
}

#[cfg(test)]
mod tests;

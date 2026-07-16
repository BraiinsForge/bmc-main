// Copyright (C) 2025  Braiins Systems s.r.o.
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

use crate::time::Timezone;
use crate::timezone_variants_raw::TIMEZONE_VARIANTS_RAW;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Curated list of `Timezone` values built from the raw const slice
/// in [`crate::timezone_variants_raw`]. The const carries the data;
/// this layer adds the `Timezone::new` parsing pass.
pub(crate) static TIMEZONE_VARIANTS: LazyLock<Vec<Timezone>> = LazyLock::new(|| {
    TIMEZONE_VARIANTS_RAW
        .iter()
        .map(|(iana, posix)| Timezone::new(iana, posix))
        .collect()
});

/// IANA-name → `&'static Timezone` index over [`TIMEZONE_VARIANTS`] so lookups
/// avoid the ~460-entry linear scan. References point into the `Vec` above;
/// both statics live for the program lifetime.
pub(crate) static TIMEZONE_BY_IANA: LazyLock<HashMap<&'static str, &'static Timezone>> =
    LazyLock::new(|| TIMEZONE_VARIANTS.iter().map(|tz| (tz.iana(), tz)).collect());

#[test]
fn all_timezone_variants_are_supported() {
    // Just to make sure Timezone::new() does not panic due to IANA normalization.
    // This is here just to invoke Deref on LazyLock.
    let _variants = LazyLock::force(&TIMEZONE_VARIANTS);
    let _index = LazyLock::force(&TIMEZONE_BY_IANA);
}

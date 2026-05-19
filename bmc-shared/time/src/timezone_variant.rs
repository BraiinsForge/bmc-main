// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::time::Timezone;
use crate::timezone_variants_raw::TIMEZONE_VARIANTS_RAW;
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

#[test]
fn all_timezone_variants_are_supported() {
    // Just to make sure Timezone::new() does not panic due to IANA normalization.
    // This is here just to invoke Deref on LazyLock.
    let _variants = LazyLock::force(&TIMEZONE_VARIANTS);
}

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

//! Built, never run: `icu4x-datagen --markers-for-bin` reads off this binary
//! the ICU4X data `bmc-intl` can request,
//! and the base blob and every pack's blob carry exactly that.
//!
//! It makes the calls `bmc-intl` makes, over a blob read at run time so none is optimised out.
//! It cannot be `bmc-intl` itself, which embeds the base blob this writes.
//! A formatter `bmc-intl` starts using is added here, and the blobs written again.

use std::collections::HashMap;
use std::sync::Arc;

use formatjs_intl::{Intl, IntlContext, MessageCatalog, MessageDescriptorRef};
use icu_locale::LocaleExpander;
use icu_provider_blob::BlobDataProvider;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("BUG: a blob path, though this is never run");
    let blob = std::fs::read(path).expect("BUG: a readable blob, though this is never run");
    let provider = BlobDataProvider::try_new_from_blob(blob.into_boxed_slice())
        .expect("BUG: an ICU4X blob, though this is never run");

    let expander = LocaleExpander::try_new_extended_with_buffer_provider(&provider)
        .expect("BUG: likely subtags, though this is never run");

    let context = IntlContext::try_with_provider(provider)
        .expect("BUG: locale fallback data, though this is never run");

    let mut catalog = MessageCatalog::new();
    catalog
        .insert("en", HashMap::new())
        .expect("BUG: `en` is a locale");

    // Every formatter a catalog may need, the way the loader checks a pack.
    let checked = context.check_catalog(&catalog);
    let intl = Intl::try_new_with_context(["en"], "en", Arc::new(catalog), &context)
        .expect("BUG: an `en` catalog, though this is never run");
    let spoken = intl.format_message_to_string_or_default_ref(
        MessageDescriptorRef::new("probe", "{n, plural, one {{n} probe} other {{n} probes}}"),
        &HashMap::from([("n".to_owned(), 2_i64.into())]),
    );

    let mut locale = "zh".parse().expect("BUG: `zh` is a locale");
    expander.maximize(&mut locale);
    println!("{spoken} {locale} {checked:?}");
}

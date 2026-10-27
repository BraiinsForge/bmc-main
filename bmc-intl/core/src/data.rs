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

//! CLDR data, composed at run time from blobs.
//!
//! The embedded base blob holds English, root and the locale-fallback data;
//! every other language's comes from its pack.
//! The base blob answers first, so no pack can change English or root.

use std::sync::{Arc, LazyLock};

use formatjs_intl::IntlContext;
use icu_locale::{LocaleExpander, LocaleFallbacker};
use icu_locale_core::LanguageIdentifier;
use icu_locale_core::subtags::Language as LanguageSubtag;
use icu_provider::{DataError, DataErrorKind, DataIdentifierBorrowed, DataMarkerInfo, DataRequest};
use icu_provider_adapters::filter::FilterDataProvider;
use icu_provider_adapters::fork::MultiForkByErrorProvider;
use icu_provider_adapters::fork::predicates::ForkByErrorPredicate;
use icu_provider_blob::BlobDataProvider;

use crate::Language;

const BASE: &[u8] = bmc_shared_utils::include_lfs!("../../../assets/intl/cldr-base.postcard");

/// A blob behind the requests it may answer.
type Layer = FilterDataProvider<
    BlobDataProvider,
    Box<dyn Fn(DataIdentifierBorrowed<'_>) -> bool + Send + Sync>,
>;

type Blobs = MultiForkByErrorProvider<Layer, MissingData>;

fn embedded() -> Layer {
    FilterDataProvider {
        inner: BlobDataProvider::try_new_from_static_blob(BASE)
            .expect("BUG: `just intl::cldr-data` writes valid blobs"),
        predicate: Box::new(|_| true),
        filter_name: "embedded",
    }
}

/// Answering for `language` alone, whatever else the blob carries.
fn pack(language: Language, blob: BlobDataProvider) -> Layer {
    let primary: LanguageSubtag = LanguageIdentifier::from(language).language;
    FilterDataProvider {
        inner: blob,
        predicate: Box::new(move |id| id.locale.language == primary),
        filter_name: "another language's pack",
    }
}

/// Falls through to the next blob whether this one lacks the locale or the marker:
/// a pack made against another firmware can miss a marker altogether.
#[derive(Debug)]
struct MissingData;

impl ForkByErrorPredicate for MissingData {
    const UNIT_ERROR: DataErrorKind = DataErrorKind::IdentifierNotFound;

    fn test(&self, _: DataMarkerInfo, _: Option<DataRequest<'_>>, err: DataError) -> bool {
        matches!(
            err.kind,
            DataErrorKind::IdentifierNotFound | DataErrorKind::MarkerNotFound
        )
    }
}

pub struct Data {
    blobs: Arc<Blobs>,
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data")
            .field("blobs", &self.blobs.inner().len())
            .finish_non_exhaustive()
    }
}

impl Data {
    pub(crate) fn with_packs(packs: Vec<(Language, BlobDataProvider)>) -> Self {
        let mut blobs = vec![embedded()];
        blobs.extend(
            packs
                .into_iter()
                .map(|(language, blob)| pack(language, blob)),
        );
        Self {
            blobs: Arc::new(MultiForkByErrorProvider::new_with_predicate(
                blobs,
                MissingData,
            )),
        }
    }

    /// The embedded blob alone, composed once.
    #[must_use]
    pub fn builtin() -> Arc<Self> {
        static BUILTIN: LazyLock<Arc<Data>> =
            LazyLock::new(|| Arc::new(Data::with_packs(Vec::new())));
        BUILTIN.clone()
    }

    /// With a parse cache of its own.
    #[must_use]
    pub fn context(&self) -> IntlContext {
        IntlContext::try_with_provider(self.blobs.clone())
            .expect("BUG: the base blob carries the locale-fallback data")
    }

    pub(crate) fn expander(&self) -> LocaleExpander {
        LocaleExpander::try_new_extended_with_buffer_provider(&*self.blobs)
            .expect("BUG: the base blob carries the likely subtags")
    }

    pub(crate) fn fallbacker(&self) -> LocaleFallbacker {
        LocaleFallbacker::try_new_with_buffer_provider(&*self.blobs)
            .expect("BUG: the base blob carries the locale-fallback data")
    }
}

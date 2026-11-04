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

use std::sync::Arc;

use bmc_intl::{Intl, Language, Translations};
use connectrpc::RequestContext;
use http::HeaderValue;
use http::header::ACCEPT_LANGUAGE;

/// The formatter a handler words its errors with, for the language the request asks for.
pub(crate) fn intl(ctx: &RequestContext) -> Arc<Intl> {
    let translations = Translations::shared();
    bmc_intl::shared_formatter(requested_language(
        &translations,
        ctx.header(ACCEPT_LANGUAGE),
    ))
}

/// The one tag `Accept-Language` holds, when the device offers that language.
/// Anything else is English, a weighted list included: the web sends a single tag,
/// and a script that asks for nothing reads the same text whatever the device speaks.
/// Only an offered language reaches the formatter cache, which never shrinks.
fn requested_language(translations: &Translations, header: Option<&HeaderValue>) -> Language {
    header
        .and_then(|value| value.to_str().ok())
        .and_then(|tag| crate::i18n::offered(translations, tag))
        .unwrap_or(Language::EN)
}

/// The formatter a request asking for `tag` gets,
/// over the firmware's own language packs.
#[cfg(test)]
pub(crate) fn asking_for(tag: &'static str) -> Intl {
    let translations = tests::profile_translations();
    let language = requested_language(&translations, Some(&HeaderValue::from_static(tag)));
    translations.formatter(language)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    pub(super) fn profile_translations() -> Translations {
        let profile =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/intl/profile-languages");
        let (translations, report) = Translations::load(&[profile]);
        assert!(report.is_clean(), "{report}");
        translations
    }

    fn requested(value: &'static str) -> Language {
        requested_language(
            &profile_translations(),
            Some(&HeaderValue::from_static(value)),
        )
    }

    #[test]
    fn the_request_header_selects_the_shared_formatter() {
        let mut headers = http::HeaderMap::new();
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US"));
        let ctx = RequestContext::new(headers);

        assert!(Arc::ptr_eq(
            &intl(&ctx),
            &bmc_intl::shared_formatter(Language::EN_US)
        ));
    }

    #[test]
    fn no_header_is_english() {
        assert_eq!(
            requested_language(&profile_translations(), None),
            Language::EN
        );
    }

    #[test]
    fn a_single_tag_is_the_language_asked_for() {
        assert_eq!(requested("zh"), Language::ZH);
    }

    #[test]
    fn a_language_the_device_does_not_offer_is_english() {
        assert_eq!(requested("fr"), Language::EN);
        assert_eq!(requested("zh-x-probe"), Language::EN);
    }

    #[test]
    fn a_weighted_list_is_english() {
        assert_eq!(requested("zh,en;q=0.9"), Language::EN);
    }

    #[test]
    fn a_value_that_is_no_tag_is_english() {
        assert_eq!(requested("not a tag"), Language::EN);
    }

    #[test]
    fn a_value_that_is_not_text_is_english() {
        let value = HeaderValue::from_bytes(&[0xFF]).expect("BUG: an opaque header value");
        assert_eq!(
            requested_language(&profile_translations(), Some(&value)),
            Language::EN
        );
    }
}

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

//! Locale selection and the embedded translation catalogs.
//!
//! English is the source: every message carries its `default_message` inline and `en` has no catalog.
//! Other languages are flat `{id: pattern}` maps compiled from the translator files and embedded here.
//!
//! Catalogs are keyed by the locale ICU4X's fallbacker minimises to — `zh` for Simplified Chinese
//! (`zh-Hans`, `zh-CN` and `zh-Hans-CN` all resolve to it), `zh-Hant` for Traditional.
//! A catalog keyed `zh-Hans` is never a fallback candidate and can never be selected.
//!
//! Copy is declared where it is used, its placeholders checked against the English at compile time:
//!
//! ```
//! let intl = bmc_intl::formatter(bmc_intl::Language::En);
//! let text = bmc_intl::format_message!(intl, default_message: "Connect to {ssid}", values: { ssid: "deck" });
//! assert_eq!(text, "Connect to deck");
//! ```
//!
//! A placeholder the English has and the call does not give fails the build:
//!
//! ```compile_fail
//! let intl = bmc_intl::formatter(bmc_intl::Language::En);
//! bmc_intl::format_message!(intl, default_message: "Connect to {ssid}");
//! ```
//!
//! So does a value the English has no placeholder for:
//!
//! ```compile_fail
//! let intl = bmc_intl::formatter(bmc_intl::Language::En);
//! bmc_intl::format_message!(intl, default_message: "Connect", values: { ssid: "deck" });
//! ```

mod formatter;
mod localized;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

pub use bmc_intl_message::Message;
pub use bmc_shared_utils::language::Language;
use formatjs_icu_messageformat::Options;
pub use formatjs_icu_messageformat::{Value, Values};
pub use formatjs_icu_messageformat_parser::error::ParserError;
use formatjs_icu_messageformat_parser::parser::Parser;
use formatjs_intl::{IntlCache, MessageCatalog, MessageDescriptorRef};
pub use formatjs_intl::{Messages, format_message};
pub use formatter::Intl;
pub use localized::Localized;
use thiserror::Error;

const DEFAULT_LOCALE: &str = "en";

// `CATALOGS`: every `lang/<key>.json` translator file, compiled by `build.rs`.
include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// Parse `pattern` exactly as the runtime will, for a build step
/// to refuse a translation the device would otherwise silently fall back from.
///
/// # Errors
/// The parser's, when `pattern` is not ICU MessageFormat the runtime accepts.
pub fn check_pattern(pattern: &str) -> Result<(), ParserError> {
    Parser::new(pattern, Options::default_parser_options())
        .parse()
        .map(|_| ())
}

/// Word a [`Message`] declared elsewhere in `intl`'s language,
/// its translation by id or its English default, as `format_message!` does.
#[must_use]
pub fn format_lazy(intl: &Intl, message: &Message) -> String {
    let mut descriptor = MessageDescriptorRef::new(message.id, message.default_message);
    if let Some(description) = message.description {
        descriptor = descriptor.with_description(description);
    }
    intl.format_message_to_string_or_default_ref(descriptor, &Values::new())
}

/// A formatter over the bundled catalogs for `language`.
///
/// Not free: `Intl::try_new` parses two locales, builds an ICU4X `LocaleFallbacker`
/// and walks its chain, allocating a string per candidate.
/// Hold the result and build a new one when the language changes — never per render.
#[must_use]
pub fn formatter(language: Language) -> Intl {
    Translations::shared()
        .formatter(language)
        .expect("BUG: every device language has a valid catalog locale")
}

/// The process-wide formatter for `language`, built on first use and kept:
/// for code answering many callers in many languages at once (a request handler)
/// which has no one language to hold a [`Localized`] for.
#[must_use]
pub fn shared_formatter(language: Language) -> &'static Intl {
    static EVERY: LazyLock<Vec<Intl>> =
        LazyLock::new(|| Language::ALL.into_iter().map(formatter).collect());
    let index = Language::ALL
        .iter()
        .position(|candidate| *candidate == language)
        .expect("BUG: every language is listed in `Language::ALL`");
    &EVERY[index]
}

/// Immutable translation catalogs and their shared compiled-message cache.
#[derive(Clone)]
pub struct Translations {
    catalog: Arc<MessageCatalog>,
    cache: Arc<IntlCache>,
}

impl fmt::Debug for Translations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.catalog.available_locales())
            .finish()
    }
}

impl Translations {
    /// Catalogs from `(catalog key, messages)` pairs, beside the empty English one.
    ///
    /// # Errors
    /// [`enum@Error`] when a key is not a well-formed BCP-47 tag.
    pub fn from_catalogs<'a>(
        catalogs: impl IntoIterator<Item = (&'a str, Messages)>,
    ) -> Result<Self, Error> {
        let mut catalog = MessageCatalog::new();
        catalog.insert(DEFAULT_LOCALE, HashMap::new())?;
        for (key, messages) in catalogs {
            catalog.insert(key, messages)?;
        }
        Ok(Self {
            catalog: Arc::new(catalog),
            cache: Arc::new(IntlCache::new()),
        })
    }

    /// The catalogs embedded in the binary.
    ///
    /// # Errors
    /// [`enum@Error`] when an embedded catalog is not valid JSON or FormatJS rejects it.
    pub fn bundled() -> Result<Self, Error> {
        let catalogs = CATALOGS
            .iter()
            .map(|(key, json)| Ok((*key, serde_json::from_str(json)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        Self::from_catalogs(catalogs)
    }

    /// The bundled catalogs, loaded once per process.
    #[must_use]
    pub fn shared() -> &'static Self {
        static SHARED: LazyLock<Translations> = LazyLock::new(|| {
            Translations::bundled().expect("BUG: `build.rs` embeds only catalogs that parse")
        });
        &SHARED
    }

    /// A formatter for one language over these catalogs.
    ///
    /// # Errors
    /// [`enum@Error`] when FormatJS rejects the locale configuration.
    pub fn formatter(&self, language: Language) -> Result<Intl, Error> {
        Intl::over(language, self.catalog.clone(), self.cache.clone())
    }
}

/// Localisation setup failure.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid embedded translation catalog: {0}")]
    Catalog(#[from] serde_json::Error),
    #[error("localisation runtime failed: {0}")]
    Runtime(#[from] formatjs_intl::Error),
    #[error("not a BCP-47 locale: {0}")]
    Locale(#[from] icu_locale_core::ParseError),
}

#[cfg(test)]
mod build_compile {
    include!("../build/compile.rs");

    #[test]
    fn a_translator_file_compiles_to_its_translations() {
        let catalog = compile(
            "lang/zh.json",
            r#"{
                "a": { "source": "Connect", "translation": "连接" },
                "b": { "source": "Cancel", "translation": null, "description": "A button" }
            }"#,
        )
        .expect("BUG: the fixture is a valid translator file");
        assert_eq!(
            catalog,
            BTreeMap::from([("a".to_owned(), "连接".to_owned())]),
            "the untranslated entry is left out"
        );
    }

    #[test]
    fn a_broken_pattern_names_the_file_and_the_entry() {
        let error = compile(
            "lang/zh.json",
            r#"{ "broken": { "source": "Connect to {ssid}", "translation": "连接到 {ssid" } }"#,
        )
        .expect_err("an unclosed argument is not ICU MessageFormat");
        assert!(error.contains("lang/zh.json"), "{error}");
        assert!(error.contains("`broken`"), "{error}");
        assert!(error.contains("连接到 {ssid"), "{error}");
    }
}

#[cfg(test)]
mod tests {
    use formatjs_icu_messageformat::Locale;
    use formatjs_intl::negotiate_locale;

    use super::*;

    /// Whether a catalog keyed `locale` can ever be reached: whether some device language negotiates to it.
    /// `zh-Hans` cannot — the fallback chain from the device's `zh-Hans` is `zh`, `und` —
    /// so a catalog under that key is dead weight nobody notices.
    fn catalog_locale_is_reachable(locale: &str) -> Result<bool, Error> {
        let mut offered = MessageCatalog::new();
        offered.insert(DEFAULT_LOCALE, HashMap::new())?;
        offered.insert(locale, HashMap::new())?;
        let key = locale.parse::<Locale>()?.to_string();
        let default_locale: Locale = DEFAULT_LOCALE.parse()?;
        for language in Language::ALL {
            if negotiate_locale([language.tag()], &default_locale, &offered)?.to_string() == key {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn chinese(messages: &[(&str, &str)]) -> Result<Translations, Error> {
        let messages = messages
            .iter()
            .map(|(id, translation)| ((*id).to_owned(), (*translation).to_owned()))
            .collect();
        Translations::from_catalogs([(Language::ZhHans.catalog_key(), messages)])
    }

    #[test]
    fn a_message_speaks_english_with_no_catalog() -> Result<(), Error> {
        let intl = Translations::from_catalogs([])?.formatter(Language::ZhHans)?;
        let text = format_message!(intl, id: "probe", default_message: "Connect to {ssid}", values: { ssid: "deck" });
        assert_eq!(text, "Connect to deck");
        Ok(())
    }

    #[test]
    fn a_message_speaks_its_translation_once_the_catalog_is_loaded() -> Result<(), Error> {
        let translations = chinese(&[("probe", "连接到 {ssid}")])?;

        let zh = translations.formatter(Language::ZhHans)?;
        let text = format_message!(zh, id: "probe", default_message: "Connect to {ssid}", values: { ssid: "deck" });
        assert_eq!(text, "连接到 deck");

        let en = translations.formatter(Language::En)?;
        let text = format_message!(en, id: "probe", default_message: "Connect to {ssid}", values: { ssid: "deck" });
        assert_eq!(text, "Connect to deck");
        Ok(())
    }

    #[test]
    fn every_simplified_chinese_spelling_reaches_the_zh_catalog() -> Result<(), Error> {
        let translations = chinese(&[])?;
        for requested in ["zh-Hans", "zh-CN", "zh-Hans-CN"] {
            let intl = formatjs_intl::Intl::try_new(
                [requested],
                DEFAULT_LOCALE,
                translations.catalog.clone(),
                translations.cache.clone(),
            )?;
            assert_eq!(intl.locale().to_string(), "zh", "requested {requested}");
        }
        Ok(())
    }

    #[test]
    fn a_lazy_message_is_worded_by_id_and_falls_back_to_its_english() -> Result<(), Error> {
        let translations = chinese(&[("probe", "探针")])?;
        let translated = Message {
            id: "probe",
            default_message: "Probe",
            description: Some("A test fixture"),
        };
        let untranslated = Message {
            id: "probe.untranslated",
            default_message: "Untranslated probe",
            description: None,
        };

        let zh = translations.formatter(Language::ZhHans)?;
        assert_eq!(format_lazy(&zh, &translated), "探针");
        assert_eq!(format_lazy(&zh, &untranslated), "Untranslated probe");

        let en = translations.formatter(Language::En)?;
        assert_eq!(format_lazy(&en, &translated), "Probe");
        Ok(())
    }

    #[test]
    fn the_bundle_holds_the_default_locale_and_every_translator_file() -> Result<(), Error> {
        let bundled = Translations::bundled()?;
        let mut locales: Vec<&str> = bundled.catalog.available_locales().collect();
        locales.sort_unstable();
        let mut expected: Vec<&str> = CATALOGS.iter().map(|(key, _)| *key).collect();
        expected.push(DEFAULT_LOCALE);
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(locales, expected);
        Ok(())
    }

    #[test]
    fn only_minimised_catalog_keys_are_reachable() -> Result<(), Error> {
        // The table in `bmc_shared_utils` is hand-written; ICU4X is the judge.
        for language in Language::ALL {
            assert!(
                catalog_locale_is_reachable(language.catalog_key())?,
                "{language:?}'s catalog key is not where ICU4X negotiates its tag to"
            );
        }
        assert!(!catalog_locale_is_reachable("zh-Hans")?);
        assert!(!catalog_locale_is_reachable("zh-CN")?);
        assert!(!catalog_locale_is_reachable("de")?);
        assert!(catalog_locale_is_reachable("zh_CN").is_err());
        Ok(())
    }

    #[test]
    fn check_pattern_refuses_what_the_runtime_refuses() {
        assert!(check_pattern("Connect to {ssid}").is_ok());
        assert!(check_pattern("Connect to {ssid").is_err());
        assert!(
            check_pattern("{count, plural, one {{count} probe}}").is_err(),
            "the runtime requires an `other` branch"
        );
    }
}

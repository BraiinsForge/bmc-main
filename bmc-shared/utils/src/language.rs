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

use std::fmt;

use icu_locale_core::subtags::{Language as LanguageSubtag, Region, Script, Variants, language};
use icu_locale_core::{LanguageIdentifier, Locale, ParseError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The language the device speaks: web UI, overlays and widgets alike.
///
/// A BCP-47 tag's language, script and region; variants and extensions are dropped at parse.
/// A language pack brings any language beyond the consts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Language {
    primary: LanguageSubtag,
    script: Option<Script>,
    region: Option<Region>,
}

impl Language {
    /// English, the source every catalog translates.
    pub const EN: Self = Self::bare(language!("en"));
    /// Simplified Chinese, spelled as its pack is filed: Han simplified is `zh`'s default script.
    pub const ZH: Self = Self::bare(language!("zh"));

    const fn bare(primary: LanguageSubtag) -> Self {
        Self {
            primary,
            script: None,
            region: None,
        }
    }

    /// The language a BCP-47 tag names, in any casing (`cs-cz` reads as `cs-CZ`).
    ///
    /// # Errors
    /// [`ParseError`] when `tag` is not a BCP-47 language identifier.
    pub fn parse(tag: &str) -> Result<Self, ParseError> {
        // A `Locale`, not a `LanguageIdentifier`: the former takes extensions, which are dropped.
        Ok(Locale::try_from_str(tag)?.id.into())
    }

    /// The language alone, without its script or region: `zh` for `zh-Hant`.
    #[must_use]
    pub const fn primary(self) -> Self {
        Self::bare(self.primary)
    }

    /// BCP-47 tag, the form that crosses process boundaries.
    #[must_use]
    pub fn tag(self) -> String {
        self.to_string()
    }
}

/// Variants are dropped, as at parse.
impl From<LanguageIdentifier> for Language {
    fn from(identifier: LanguageIdentifier) -> Self {
        Self {
            primary: identifier.language,
            script: identifier.script,
            region: identifier.region,
        }
    }
}

impl From<Language> for LanguageIdentifier {
    fn from(language: Language) -> Self {
        Self {
            language: language.primary,
            script: language.script,
            region: language.region,
            variants: Variants::new(),
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::EN
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&LanguageIdentifier::from(*self), f)
    }
}

impl fmt::Debug for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Language({self})")
    }
}

impl Serialize for Language {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn czech() -> Language {
        Language::parse("cs").expect("BUG: `cs` is a tag")
    }

    #[test]
    fn a_tag_reads_in_any_casing_and_writes_canonically() {
        assert_eq!(Language::parse("cs-cz").expect("BUG: a tag").tag(), "cs-CZ");
        assert_eq!(
            Language::parse("ZH-hans").expect("BUG: a tag").tag(),
            "zh-Hans"
        );
        assert_eq!(Language::parse("zh").expect("BUG: a tag"), Language::ZH);
        assert_eq!(
            Language::parse("en-US-u-ca-buddhist")
                .expect("BUG: a tag")
                .tag(),
            "en-US",
            "extensions are dropped"
        );
        assert!(Language::parse("zh_CN").is_err());
        assert!(Language::parse("").is_err());
    }

    #[test]
    fn the_primary_language_drops_the_script_and_the_region() {
        let primary = |tag| Language::parse(tag).expect("BUG: a tag").primary();
        assert_eq!(primary("zh-Hant-TW"), Language::ZH);
        assert_eq!(primary("cs"), czech());
    }

    #[test]
    fn the_stored_form_is_the_tag() {
        assert_eq!(
            serde_json::to_string(&Language::ZH).expect("BUG: serialises"),
            r#""zh""#
        );
        for (stored, language) in [(r#""zh""#, Language::ZH), (r#""cs""#, czech())] {
            assert_eq!(
                serde_json::from_str::<Language>(stored).expect(stored),
                language
            );
        }
        assert!(serde_json::from_str::<Language>(r#""Klingon!""#).is_err());
    }
}

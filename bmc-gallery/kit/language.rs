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

//! The device language the window-bar switcher selects.
//!
//! The gallery reads the packs a device profile ships and puts them in effect for every scene,
//! so a scene offers what such a device would.

use std::path::Path;
use std::sync::{LazyLock, Mutex, PoisonError};

use bmc_intl::{Language, Translations};
use gallery::GlobalKey;

/// The window bar's language switcher.
pub const LANGUAGE: GlobalKey<Language> = GlobalKey::new("Language");

static OFFERED: LazyLock<Vec<Language>> = LazyLock::new(|| {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (translations, report) =
        Translations::load(&[root.join("../assets/intl/profile-languages")]);
    if !report.is_clean() {
        tracing::warn!("{report}");
    }
    let languages = translations.languages();
    Translations::install(translations);
    languages
});

static SELECTED: Mutex<Language> = Mutex::new(Language::EN_GB);

/// The switcher's stand-in for a device that offers no languages:
/// `und` names none, and no pack can be filed under it.
pub(crate) static NO_LANGUAGES: LazyLock<Language> =
    LazyLock::new(|| Language::parse("und").expect("BUG: `und` is a tag"));

/// Every language the switcher offers: the profile's packs, loaded on first use.
pub(crate) fn offered() -> &'static [Language] {
    &OFFERED
}

/// Make `language` the scenes' language, with the gallery's packs in effect.
pub(crate) fn select(language: Language) {
    LazyLock::force(&OFFERED);
    *SELECTED.lock().unwrap_or_else(PoisonError::into_inner) = language;
}

/// The language the switcher currently shows;
/// `None` for a device that offers no languages, as one without the i18n marker.
#[must_use]
pub fn language() -> Option<Language> {
    let selected = *SELECTED.lock().unwrap_or_else(PoisonError::into_inner);
    (selected != *NO_LANGUAGES).then_some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switcher_offers_british_english_first_and_the_profiles_packs() {
        let offered = offered();
        assert_eq!(offered.first(), Some(&Language::EN_GB));
        assert!(offered.contains(&Language::ZH), "{offered:?}");
    }

    #[test]
    fn und_reads_as_a_device_without_languages() {
        select(*NO_LANGUAGES);
        assert_eq!(language(), None);
        select(Language::ZH);
        assert_eq!(language(), Some(Language::ZH));
    }
}

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

//! The language a surface renders in, with the formatter for it.

use crate::{Intl, Language};

/// The device language as last announced and the formatter built for it.
/// Building an [`Intl`] is not free, so it happens here, once per change, never per render.
pub struct Localized {
    language: Language,
    intl: Intl,
}

impl Localized {
    /// English, as every surface starts before the device's language is announced.
    #[must_use]
    pub fn new() -> Self {
        let language = Language::default();
        Self {
            language,
            intl: crate::formatter(language),
        }
    }

    /// Switch to `language`; `true` when it differs from the current one
    /// and the caller should mark its content dirty.
    pub fn set(&mut self, language: Language) -> bool {
        if language == self.language {
            return false;
        }
        self.language = language;
        self.intl = crate::formatter(language);
        true
    }

    #[must_use]
    pub fn language(&self) -> Language {
        self.language
    }

    #[must_use]
    pub fn intl(&self) -> &Intl {
        &self.intl
    }
}

impl Default for Localized {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Localized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Localized")
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_different_language_is_a_change() {
        let mut localized = Localized::new();
        assert_eq!(localized.language(), Language::En);
        assert!(
            !localized.set(Language::En),
            "the starting language again is not a change"
        );
        assert!(localized.set(Language::ZhHans));
        assert_eq!(localized.language(), Language::ZhHans);
        assert!(
            !localized.set(Language::ZhHans),
            "the same language again is not a change"
        );
        assert!(localized.set(Language::En));
    }

    #[test]
    fn the_formatter_follows_the_language() {
        let mut localized = Localized::new();
        assert_eq!(localized.intl().language(), Language::En);
        localized.set(Language::ZhHans);
        assert_eq!(
            localized.intl().language(),
            Language::ZhHans,
            "the formatter is rebuilt for the new language"
        );
    }
}

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

use serde::{Deserialize, Serialize};

/// The language the device speaks: web UI, overlays and widgets alike.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub enum Language {
    #[default]
    En,
    ZhHans,
}

impl Language {
    /// Every language, in declaration order.
    pub const ALL: [Self; 2] = [Self::En, Self::ZhHans];

    /// BCP-47 tag, the form that crosses process boundaries.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhHans => "zh-Hans",
        }
    }

    /// The key a catalog for this language is filed under, and so the name of its `lang/<key>.json`:
    /// the locale ICU4X minimises the tag to, `zh` for `zh-Hans`.
    #[must_use]
    pub const fn catalog_key(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhHans => "zh",
        }
    }

    /// The inverse of [`Self::catalog_key`].
    #[must_use]
    pub fn from_catalog_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.catalog_key() == key)
    }

    /// The inverse of [`Self::tag`], exact match only — for tags this code emitted itself.
    /// Locales from users or peers carry regions, scripts and casing to negotiate,
    /// which `bmc_intl` does through ICU4X.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|language| language.tag() == tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip_exactly() {
        for language in Language::ALL {
            assert_eq!(Language::from_tag(language.tag()), Some(language));
        }
        assert_eq!(Language::from_tag("zh-CN"), None);
        assert_eq!(Language::from_tag("EN"), None);
    }

    #[test]
    fn catalog_keys_round_trip_exactly() {
        for language in Language::ALL {
            assert_eq!(
                Language::from_catalog_key(language.catalog_key()),
                Some(language)
            );
        }
        assert_eq!(Language::from_catalog_key("zh-Hans"), None);
    }

    #[test]
    fn the_device_tag_and_the_catalog_key_differ_for_chinese() {
        assert_eq!(Language::ZhHans.tag(), "zh-Hans");
        assert_eq!(Language::ZhHans.catalog_key(), "zh");
    }
}

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

//! The formatter every Rust surface words its copy through.

use std::fmt;
use std::sync::Arc;

use formatjs_icu_messageformat::Locale;
use formatjs_intl::{IntlContext, MessageCatalog, MessageDescriptor, MessageDescriptorRef};

use crate::time::{Direction, TimeFormats, TimeUnit, TimeWidth};
use crate::{DEFAULT_LOCALE, Language, Values};

/// A `formatjs_intl::Intl` for one device language.
///
/// The methods are those `format_message!` expands to, so the macro works on this as on the original.
pub struct Intl {
    language: Language,
    version: u64,
    inner: formatjs_intl::Intl,
    time: Arc<TimeFormats>,
}

impl Intl {
    /// `language` resolved to the catalog it negotiates to;
    /// `context` holds the CLDR data and the parsed patterns,
    /// `time` the time formatters of the language the catalog speaks.
    pub(crate) fn over(
        language: Language,
        version: u64,
        catalog: Arc<MessageCatalog>,
        context: &IntlContext,
        time: Arc<TimeFormats>,
    ) -> Result<Self, formatjs_intl::Error> {
        Ok(Self {
            language,
            version,
            inner: formatjs_intl::Intl::try_new_with_context(
                [language.tag()],
                DEFAULT_LOCALE,
                catalog,
                context,
            )?,
            time,
        })
    }

    /// `count` units behind or ahead of now: "5 hours ago", Czech "před 5 hodinami".
    #[must_use]
    pub fn relative_time(
        &self,
        count: u64,
        unit: TimeUnit,
        width: TimeWidth,
        direction: Direction,
    ) -> String {
        self.timed(|time| time.relative(count, unit, width, direction))
    }

    /// The present moment, as relative times word it: "now", Chinese "现在".
    #[must_use]
    pub fn now(&self, width: TimeWidth) -> String {
        self.timed(|time| time.now(width))
    }

    /// `count` of `unit` alone, abbreviated: "30s".
    /// Spelled out, a count can decline with the words around it, so that is left to whole messages.
    #[must_use]
    pub fn time_count(&self, count: u64, unit: TimeUnit) -> String {
        self.timed(|time| time.count(count, unit))
    }

    /// By this language's time formatters, or English's where its data lacks one.
    fn timed(&self, word: impl Fn(&TimeFormats) -> Option<String>) -> String {
        word(&self.time)
            .or_else(|| word(&TimeFormats::english()))
            .expect("BUG: the base data carries every time formatter the markers binary builds")
    }

    #[must_use]
    pub fn language(&self) -> Language {
        self.language
    }

    /// What decides its words: the language, and the translations' version,
    /// which a pack installed or reloaded moves.
    #[must_use]
    pub fn wording(&self) -> (Language, u64) {
        (self.language, self.version)
    }

    /// The locale the language negotiated to among the catalogs.
    #[must_use]
    pub fn locale(&self) -> &Locale {
        self.inner.locale()
    }

    /// Format `descriptor`'s translation or its default, the default verbatim when formatting fails.
    #[must_use]
    pub fn format_message_to_string_or_default(
        &self,
        descriptor: MessageDescriptor,
        values: &Values<String>,
    ) -> String {
        self.inner
            .format_message_to_string_or_default(descriptor, values)
    }

    /// [`Self::format_message_to_string_or_default`] for a descriptor made at run time.
    #[must_use]
    pub fn format_message_to_string_or_default_ref(
        &self,
        descriptor: MessageDescriptorRef<'_>,
        values: &Values<String>,
    ) -> String {
        self.inner
            .format_message_to_string_or_default_ref(descriptor, values)
    }
}

impl fmt::Debug for Intl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Intl")
            .field("language", &self.language)
            .field("locale", self.locale())
            .finish_non_exhaustive()
    }
}

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

use crate::{DEFAULT_LOCALE, Language, Values};

/// A `formatjs_intl::Intl` for one device language.
///
/// The methods are those `format_message!` expands to, so the macro works on this as on the original.
pub struct Intl {
    language: Language,
    inner: formatjs_intl::Intl,
}

impl Intl {
    /// `language` resolved to the catalog it negotiates to;
    /// `context` holds the CLDR data and the parsed patterns.
    pub(crate) fn over(
        language: Language,
        catalog: Arc<MessageCatalog>,
        context: &IntlContext,
    ) -> Result<Self, formatjs_intl::Error> {
        Ok(Self {
            language,
            inner: formatjs_intl::Intl::try_new_with_context(
                [language.tag()],
                DEFAULT_LOCALE,
                catalog,
                context,
            )?,
        })
    }

    #[must_use]
    pub fn language(&self) -> Language {
        self.language
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

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

//! The file that turns the device's languages on.
//! Without it the device behaves as the BMC application did before them: their APIs and pickers are absent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bmc_intl::{Language, Translations};

pub(crate) const MARKER_PATH: &str = "/etc/bmc/i18n";

#[derive(Clone, Debug)]
pub struct I18nMarker(Arc<Path>);

impl I18nMarker {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self(path.into())
    }

    /// No file can be at the empty path.
    #[cfg(test)]
    pub(crate) fn unset() -> Self {
        Self::new(PathBuf::new())
    }

    /// Looked up on every call, so creating or removing the file takes effect at once.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.0.exists()
    }
}

/// The language `tag` names, when the device offers it, spelled as it lists it:
/// a picker sends back what it was given, so any other spelling is a client's mistake.
pub(crate) fn offered(translations: &Translations, tag: &str) -> Option<Language> {
    Language::parse(tag)
        .ok()
        .filter(|language| language.tag() == tag && translations.languages().contains(language))
}

/// `language` while the device offers it: none once it no longer does,
/// so a picker never shows as current a language its list lacks.
pub(crate) fn listed(offered: &[Language], language: Language) -> Option<Language> {
    offered.contains(&language).then_some(language)
}

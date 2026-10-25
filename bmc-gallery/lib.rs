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

//! The gallery's scenes dylib: the discovered `*.scene.rs` compiled in,
//! plus the Deck kit they draw with (re-exported through [`prelude`]).

// Scenes are compiled in from wherever they sit beside their components, so
// `crate::` in one would read as the crate it lives next to. Name this one.
extern crate self as bmc_gallery;

use std::sync::LazyLock;

use bmc_intl::{Language, Translations};
use gallery::{CatalogGlobals, GlobalControls, Icon};
use serde::{Deserialize, Serialize};

pub mod kit;
pub mod prelude;

/// What a flagless language shows in the bar, which draws filled paths and no text.
const FLAGLESS: &[u8] = include_bytes!("assets/flagless.svg");

/// The `und` button: ∅, a device that offers no languages.
const EMPTY_SET: &[u8] = include_bytes!("assets/no-languages.svg");

/// Each offered language as a window-bar button, labelled by tag:
/// the bar's font has no Han glyphs for an endonym, and the tag is what a capture recipe names anyway.
/// Last comes `und`, a device that offers no languages.
static LANGUAGE_BUTTONS: LazyLock<Vec<(String, Icon, Language)>> = LazyLock::new(|| {
    let offered = kit::language::offered();
    let translations = Translations::shared();
    let no_languages = *kit::language::NO_LANGUAGES;
    let no_languages_button = (no_languages.tag(), Icon::from_svg(EMPTY_SET), no_languages);
    offered
        .iter()
        .map(|&language| {
            let icon = match translations.flag_svg(language) {
                Ok(Some(svg)) => Icon::try_from_svg_colored(&svg).unwrap_or_else(|error| {
                    tracing::warn!(?error, %language, "language flag is not a valid SVG");
                    Icon::from_svg(FLAGLESS)
                }),
                Ok(None) => Icon::from_svg(FLAGLESS),
                Err(error) => {
                    tracing::warn!(%error, %language, "language flag unreadable");
                    Icon::from_svg(FLAGLESS)
                }
            };
            (language.tag(), icon, language)
        })
        .chain(std::iter::once(no_languages_button))
        .collect()
});

/// The controls in the window bar that every scene renders under.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Globals {
    pub language: Language,
}

impl CatalogGlobals for Globals {
    fn controls(&mut self, controls: &mut GlobalControls<'_>) {
        let options: Vec<(&str, &Icon, Language)> = LANGUAGE_BUTTONS
            .iter()
            .map(|(tag, icon, language)| (tag.as_str(), icon, *language))
            .collect();
        self.language = controls.icon_buttons(kit::LANGUAGE, &options, self.language);
    }

    fn prepare(&self, _ui: &mut gallery::egui::Ui) {
        kit::language::select(self.language);
    }
}

gallery::scenes_dylib!(Globals);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_icons_parse() {
        assert!(Icon::try_from_svg(FLAGLESS).is_ok(), "flagless.svg");
        assert!(Icon::try_from_svg(EMPTY_SET).is_ok(), "no-languages.svg");
    }
}

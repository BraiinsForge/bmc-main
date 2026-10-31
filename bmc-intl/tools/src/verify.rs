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

//! `intl verify`: the gate over every surface's translator files.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::surface::{self, Surface, SurfaceName};
use crate::{check, console, formatjs, translator};

/// A translator file as `verify` found it.
#[derive(Debug)]
pub struct Checked {
    pub file: PathBuf,
    pub untranslated: usize,
    /// How the file lags the extraction, then what its translations get wrong.
    pub problems: Vec<String>,
}

/// Fail on a surface whose translator files are stale or broken;
/// an untranslated message fails nothing.
///
/// # Errors
/// A missing tool, or an extraction or translator file that fails.
pub fn run(surfaces: &[SurfaceName], repo: &Path) -> Result<ExitCode, String> {
    formatjs::require_pinned(repo)?;
    let id_pattern = formatjs::id_pattern(repo)?;
    let mut failures = 0;
    for surface in surface::select(surfaces, repo)? {
        let checked = verify_surface(&surface, &id_pattern)?;
        let count: usize = checked.iter().map(|file| file.problems.len()).sum();
        if count == 0 {
            console::ok(&summary(&surface.name, &checked));
        } else {
            console::error(&format!(
                "{}: {} problem(s)",
                console::name(&surface.name),
                console::lit(&count.to_string())
            ));
            for file in checked.iter().filter(|file| !file.problems.is_empty()) {
                console::details(
                    &console::path(surface::relative(repo, &file.file)),
                    &file.problems,
                );
            }
        }
        failures += count;
    }
    Ok(if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// A passing surface's line: its translator files are current,
/// and how many of its messages still wait for a translator, language by language.
#[must_use]
pub fn summary(surface: &str, checked: &[Checked]) -> String {
    let name = console::name(surface);
    if checked.is_empty() {
        return format!("{name}: no translator files yet");
    }
    let counts: Vec<String> = checked
        .iter()
        .filter(|file| file.untranslated > 0)
        .map(|file| {
            format!(
                "{} in {}",
                console::lit(&file.untranslated.to_string()),
                console::lit(&surface::language_of(&file.file))
            )
        })
        .collect();
    if counts.is_empty() {
        format!("{name}: translator files current, everything translated")
    } else {
        format!(
            "{name}: translator files current; untranslated: {}",
            counts.join(", ")
        )
    }
}

/// Each of the surface's translator files, the pseudo-locale's last, checked against its extraction.
///
/// # Errors
/// The extraction's failure, or a translator file that cannot be read.
pub fn verify_surface(surface: &Surface, id_pattern: &str) -> Result<Vec<Checked>, String> {
    let extracted = formatjs::extract(surface, id_pattern)?;
    let fix = console::command(&surface.fix_command());
    let by_language = surface.translator_entries()?;
    let mut checked: Vec<Checked> = by_language
        .iter()
        .map(|(language, entries)| {
            let mut problems = translator::stale_entries(&extracted, entries, &fix);
            problems.extend(check::translations(&extracted, entries));
            Checked {
                file: surface.translator_file(language),
                untranslated: translator::untranslated(language, &by_language).count(),
                problems,
            }
        })
        .collect();
    let pseudo_file = surface.pseudo_file();
    let pseudo = translator::read_entries(&pseudo_file)?;
    checked.push(Checked {
        problems: pseudo_problems(&extracted, &pseudo, &fix),
        file: pseudo_file,
        untranslated: 0,
    });
    Ok(checked)
}

/// The pseudo-locale's file checked as a pack's is, beside its lag behind the transform:
/// comparing it with the transform alone would pass a transform that breaks a message.
fn pseudo_problems(
    extracted: &translator::Extraction,
    entries: &translator::Entries,
    fix: &str,
) -> Vec<String> {
    let mut problems = translator::stale_pseudo(extracted, entries, fix);
    problems.extend(check::translations(extracted, entries));
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translator::{Extracted, Extraction, pseudo_entries};

    #[test]
    fn a_pseudo_translation_that_drops_an_argument_is_named() {
        let extracted = Extraction::from([("aaa".to_owned(), Extracted::new("Hello {name}"))]);
        let mut entries = pseudo_entries(&extracted);
        if let Some(entry) = entries.get_mut("aaa") {
            entry.translation = Some("[Ħḗḗŀŀǿǿ]".to_owned());
        }
        let problems = pseudo_problems(&extracted, &entries, "fix");
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("drops {name}")),
            "{problems:?}"
        );
    }
}

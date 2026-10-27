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

/// Each of the surface's translator files, checked against its extraction.
///
/// # Errors
/// The extraction's failure, or a translator file that cannot be read.
pub fn verify_surface(surface: &Surface, id_pattern: &str) -> Result<Vec<Checked>, String> {
    let extracted = formatjs::extract(surface, id_pattern)?;
    let fix = console::command(&surface.fix_command());
    let by_language = surface.translator_entries()?;
    Ok(by_language
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
        .collect())
}

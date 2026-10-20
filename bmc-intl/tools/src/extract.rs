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

//! `intl extract`: the messages of each surface,
//! synced into its translator files by id.

use std::path::Path;
use std::process::ExitCode;

use crate::surface::{self, Surface, SurfaceName};
use crate::{console, formatjs, translator};

/// # Errors
/// A missing tool, or an extraction or translator file that fails.
pub fn run(surfaces: &[SurfaceName], repo: &Path) -> Result<ExitCode, String> {
    let id_pattern = formatjs::id_pattern(repo)?;
    for surface in surface::select(surfaces, repo)? {
        extract_surface(&surface, &id_pattern)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Extract `surface` and sync each of its translator files through [`translator::merge`].
///
/// # Errors
/// The extraction's failure, or a translator file that cannot be read or written.
pub fn extract_surface(surface: &Surface, id_pattern: &str) -> Result<(), String> {
    let extracted = formatjs::extract(surface, id_pattern)?;
    let files = surface.translator_files()?;
    for translator_file in &files {
        let merged = translator::merge(&extracted, &translator::read_entries(translator_file)?);
        translator::write_json(translator_file, &merged)?;
    }
    let languages: Vec<String> = files
        .iter()
        .map(|file| console::lit(&surface::language_of(file)))
        .collect();
    let synced = if languages.is_empty() {
        "no translator files yet".to_owned()
    } else {
        format!("translator files synced: {}", languages.join(", "))
    };
    console::ok(&format!(
        "{}: {} messages extracted, {synced}",
        console::name(&surface.name),
        console::lit(&extracted.len().to_string())
    ));
    Ok(())
}

// Copyright (C) 2025  Braiins Systems s.r.o.
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

//! Where widgets sit below a widgets directory. Every check that predicts
//! what the registry will discover must walk as discovery walks.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Folder levels below a widgets directory a widget may sit at; discovery looks no deeper.
pub const MAX_WIDGET_DIR_DEPTH: usize = 3;

/// The widget folders under `root`: each folder holding a `manifest.json`,
/// found by a walk at most [`MAX_WIDGET_DIR_DEPTH`] deep that follows links
/// and does not descend past a widget. Walk errors are yielded in place,
/// so a caller can skip them or refuse an incomplete listing.
#[must_use]
pub fn widget_dirs(root: &Path) -> Vec<walkdir::Result<PathBuf>> {
    let mut dirs = Vec::new();
    let mut entries = WalkDir::new(root)
        .follow_links(true)
        .min_depth(1)
        .max_depth(MAX_WIDGET_DIR_DEPTH)
        .into_iter();
    while let Some(entry) = entries.next() {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_dir() && entry.path().join("manifest.json").exists() {
                    dirs.push(Ok(entry.path().to_path_buf()));
                    entries.skip_current_dir();
                }
            }
            Err(err) => dirs.push(Err(err)),
        }
    }
    dirs
}

#[cfg(test)]
mod tests;

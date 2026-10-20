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

//! The formatjs CLI, which extracts the messages of Rust and TypeScript sources alike.

use std::path::Path;
use std::process::Command;

use crate::console;
use crate::surface::Surface;
use crate::translator::Extraction;

const FORMATJS: &str = "formatjs";

/// The file holding how a message's id is hashed, which the web build reads too.
const MESSAGE_ID: &str = "frontend/message-id.json";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageId {
    id_interpolation_pattern: String,
}

/// The id pattern the checkout hashes messages by, as `formatjs extract` takes it.
///
/// # Errors
/// The file named, when it cannot be read or does not name a pattern.
pub fn id_pattern(repo: &Path) -> Result<String, String> {
    let path = repo.join(MESSAGE_ID);
    let bytes = std::fs::read(&path).map_err(|error| console::at(&path, error))?;
    serde_json::from_slice::<MessageId>(&bytes)
        .map(|id| id.id_interpolation_pattern)
        .map_err(|error| console::at(&path, error))
}

/// The surface's messages; none when no crate can declare any yet.
///
/// # Errors
/// What `formatjs extract` printed when it failed, or why its output does not read.
pub fn extract(surface: &Surface, id_pattern: &str) -> Result<Extraction, String> {
    if surface.sources.is_empty() {
        return Ok(Extraction::new());
    }
    let output = Command::new(FORMATJS)
        .arg("extract")
        .args(&surface.sources)
        .args(
            surface
                .ignore
                .iter()
                .flat_map(|pattern| ["--ignore", pattern]),
        )
        .args([
            "--format",
            "crowdin",
            "--id-interpolation-pattern",
            id_pattern,
            "--throws",
        ])
        .output()
        .map_err(|error| console::spawn_failed(FORMATJS, &error))?;
    if !output.status.success() {
        return Err(format!(
            "{}: `formatjs extract` failed\n{}",
            surface.name,
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "{}: `formatjs extract` printed no extraction: {error}",
            surface.name
        )
    })
}

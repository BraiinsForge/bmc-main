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

/// Pins the `formatjs` release.
const CLI_PIN: &str = "nix/intl/formatjs-cli.nix";

/// The file holding how a message's id is hashed, which the web build reads too.
const MESSAGE_ID: &str = "frontend/message-id.json";

/// Refuse a `formatjs` other than the pinned release:
/// another can hash message ids differently from the macros and the web build.
///
/// # Errors
/// The release on PATH when it is not the pinned one, or why it would not start.
pub fn require_pinned(repo: &Path) -> Result<(), String> {
    let output = Command::new(FORMATJS)
        .arg("--version")
        .output()
        .map_err(|error| console::spawn_failed(FORMATJS, &error))?;
    pinned_or_refused(&String::from_utf8_lossy(&output.stdout), repo)
}

/// `printed` is what `formatjs --version` prints: `formatjs 1.8.2`.
fn pinned_or_refused(printed: &str, repo: &Path) -> Result<(), String> {
    let found = printed
        .split_whitespace()
        .last()
        .unwrap_or("of an unknown version");
    let pinned = cli_pin(repo, "version")?;
    if found == pinned {
        return Ok(());
    }
    Err(console::outside_dev_shell(&format!(
        "{FORMATJS} {found} is on PATH, {CLI_PIN} pins {pinned}"
    )))
}

/// The `name = "…";` binding of [`CLI_PIN`].
fn cli_pin(repo: &Path, name: &str) -> Result<String, String> {
    let path = repo.join(CLI_PIN);
    let text = std::fs::read_to_string(&path).map_err(|error| console::at(&path, error))?;
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(name)?
                .strip_prefix(" = \"")?
                .strip_suffix("\";")
        })
        .map(str::to_owned)
        .ok_or_else(|| format!("{} binds no `{name}`", console::path(&path)))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts the web build's id for its one message.
    const ID_SPEC: &str = "frontend/src/intl-id.spec.ts";

    const WEB_CONFIGS: [&str; 2] = ["frontend/rsbuild.config.ts", "frontend/rstest.config.ts"];

    fn checkout() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
    }

    fn read(file: &str) -> String {
        std::fs::read_to_string(checkout().join(file))
            .unwrap_or_else(|error| panic!("BUG: the checkout's {file}: {error}"))
    }

    #[test]
    fn the_cli_extracts_the_id_the_web_build_is_held_to() {
        let pattern = id_pattern(checkout()).expect("BUG: the checkout's message-id.json");
        let extracted = extract(
            &crate::surface::probe_surface(checkout(), ID_SPEC),
            &pattern,
        )
        .expect("BUG: an extraction");
        let ids: Vec<&String> = extracted.keys().collect();
        let [id] = ids[..] else {
            panic!("{ID_SPEC} declares one message, not {ids:?}");
        };
        assert!(
            read(ID_SPEC).contains(&format!("EXTRACTED_ID = '{id}'")),
            "{ID_SPEC} holds the web build to another id than the CLI extracts, {id}"
        );
    }

    #[test]
    fn the_web_build_hashes_ids_as_the_cli_extracts_them() {
        for config in WEB_CONFIGS {
            let text = read(config);
            assert!(
                text.contains("from './message-id.json'"),
                "{config} reads no message-id.json"
            );
            let Some((options, _)) = text
                .split_once("'@swc/plugin-formatjs',")
                .and_then(|(_, rest)| rest.split_once('{')?.1.split_once('}'))
            else {
                panic!("{config} configures no @swc/plugin-formatjs");
            };
            let options = options.split_whitespace().collect::<Vec<_>>().join(" ");
            // `ast` shapes `defaultMessage`, not the id; any other option could change ids unseen.
            assert_eq!(
                options.replace("ast: true, ", ""),
                "idInterpolationPattern: MESSAGE_ID.idInterpolationPattern",
                "{config} must hash ids as `intl extract` does, with no other option"
            );
        }
    }

    #[test]
    fn only_the_pinned_cli_is_accepted() {
        let pinned = cli_pin(checkout(), "version").expect("BUG: the checkout pins a version");
        assert_eq!(
            pinned_or_refused(&format!("formatjs {pinned}\n"), checkout()),
            Ok(())
        );
        let refused = pinned_or_refused("formatjs 1.4.2\n", checkout())
            .expect_err("BUG: another release is refused");
        assert!(
            refused.contains(&format!(
                "formatjs 1.4.2 is on PATH, {CLI_PIN} pins {pinned}"
            )),
            "{refused}"
        );
    }
}

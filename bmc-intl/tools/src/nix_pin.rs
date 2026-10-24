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

//! The `name = "…";` bindings of the nix files that pin upstream releases, each bound exactly once.

use std::path::Path;

use crate::console;

/// # Errors
/// The file named, when it cannot be read or does not bind `name` exactly once.
pub fn get(nix_file: &Path, name: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(nix_file).map_err(|error| console::at(nix_file, error))?;
    once(&text, nix_file, name).map(|(_, value)| value.to_owned())
}

/// Rebind the one `name = "…";` to `value`, every other byte as it was.
///
/// # Errors
/// As [`get`], or the file named when it cannot be written.
pub fn set(nix_file: &Path, name: &str, value: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(nix_file).map_err(|error| console::at(nix_file, error))?;
    let (bound_at, old) = once(&text, nix_file, name)?;
    let rebound: String = text
        .split_inclusive('\n')
        .enumerate()
        .map(|(at, line)| {
            if at == bound_at {
                line.replacen(
                    &format!("{name} = \"{old}\""),
                    &format!("{name} = \"{value}\""),
                    1,
                )
            } else {
                line.to_owned()
            }
        })
        .collect();
    std::fs::write(nix_file, rebound).map_err(|error| console::at(nix_file, error))
}

/// The line binding `name`, and its value.
fn once<'t>(text: &'t str, nix_file: &Path, name: &str) -> Result<(usize, &'t str), String> {
    let found: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .filter_map(|(at, line)| Some((at, bound(line, name)?)))
        .collect();
    match found[..] {
        [one] => Ok(one),
        _ => Err(format!(
            "{} binds `{name}` {} times, not once",
            console::path(nix_file),
            found.len()
        )),
    }
}

fn bound<'l>(line: &'l str, name: &str) -> Option<&'l str> {
    line.trim()
        .strip_prefix(name)?
        .strip_prefix(" = \"")?
        .strip_suffix("\";")
        .filter(|value| !value.contains('"'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("BUG: a temporary directory");
        let file = dir.path().join("pin.nix");
        std::fs::write(&file, text).expect("BUG: a writable temporary directory");
        (dir, file)
    }

    #[test]
    fn a_pin_is_rebound_in_place() {
        let (_dir, file) = pin("  version = \"1.8.2\";\n  cargoHash = \"sha256-old\";\n");
        set(&file, "version", "1.9.0").expect("BUG: bound once");
        assert_eq!(
            std::fs::read_to_string(&file).expect("BUG: just written"),
            "  version = \"1.9.0\";\n  cargoHash = \"sha256-old\";\n"
        );
    }

    #[test]
    fn a_pin_bound_twice_is_refused() {
        let (_dir, file) = pin("  hash = \"a\";\n  hash = \"b\";\n");
        let refused = set(&file, "hash", "c").expect_err("BUG: bound twice");
        assert!(refused.contains("binds `hash` 2 times"), "{refused}");
    }

    #[test]
    fn a_name_inside_another_binding_is_not_a_binding_of_it() {
        let (_dir, file) = pin("  tag = \"formatjs_cli_v${version}\";\n  version = \"1.8.2\";\n");
        assert_eq!(get(&file, "version"), Ok("1.8.2".to_owned()));
    }
}

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

//! Reject SVG assets that rely on features the host renderer does not honour.
//!
//! The renderer rasterises SVGs as plain filled or stroked paths. Filter graphs, clip masks,
//! and external references are dropped during compilation, so effects belong in the SDK draw
//! tree, for example through `Draw::with_drop_shadow`.

use std::path::{Path, PathBuf};

const FORBIDDEN_ELEMENTS: &[&str] = &["filter", "mask", "clipPath", "use", "image"];
const FORBIDDEN_ATTRIBUTES: &[&str] = &["filter", "mask", "clip-path"];
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

fn is_cache_dir(dir: &Path) -> bool {
    std::fs::read(dir.join("CACHEDIR.TAG")).is_ok_and(|tag| tag.starts_with(CACHEDIR_SIGNATURE))
}

fn collect_svgs(dir: &Path, paths: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("BUG: read {}: {error}", dir.display()),
    };
    for entry in entries {
        let path = entry.expect("BUG: read SVG entry").path();
        if path.is_dir() && !is_cache_dir(&path) {
            collect_svgs(&path, paths);
        } else if path.extension().is_some_and(|extension| extension == "svg") {
            paths.push(path);
        }
    }
}

fn violations(document: &roxmltree::Document<'_>) -> Vec<String> {
    let mut violations = Vec::new();
    for node in document.descendants().filter(roxmltree::Node::is_element) {
        let tag = node.tag_name().name();
        if FORBIDDEN_ELEMENTS.contains(&tag) {
            violations.push(format!("<{tag}> element"));
        }
        for attribute in node.attributes() {
            let name = attribute.name();
            if FORBIDDEN_ATTRIBUTES.contains(&name) {
                violations.push(format!("<{tag} {name}=…>"));
            }
        }
    }
    violations
}

#[test]
fn shipping_svgs_do_not_use_unsupported_features() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: manifest test crate must be below workspace root");
    let mut paths = Vec::new();
    collect_svgs(&workspace.join("widgets-wasm"), &mut paths);
    assert!(!paths.is_empty(), "BUG: no shipping SVG assets discovered");

    let mut offenders = Vec::new();
    for path in paths {
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("BUG: read {}: {error}", path.display()));
        let _compiled = bmc_svg_compiler::compile_svg(&contents);
        let document = roxmltree::Document::parse(&contents)
            .unwrap_or_else(|error| panic!("BUG: parse {}: {error}", path.display()));
        for violation in violations(&document) {
            offenders.push(format!("{}: {violation}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "shipping SVGs use unsupported features (move the effect into the SDK draw tree, e.g. \
         `Draw::with_drop_shadow`):\n  {}",
        offenders.join("\n  ")
    );
}

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

//! Reject SVG assets that rely on features the host renderer doesn't honour.
//!
//! The renderer rasterises SVGs as plain filled / stroked paths; filter
//! graphs, clip masks and external references are dropped silently at compile
//! time. Effects belong in the SDK draw tree (e.g. `Draw::with_drop_shadow`).

/// Roots that ship rasterised SVG assets, kept in sync with the workspace.
const SVG_ROOTS: &[&str] = &["widgets-wasm"];

/// Element local-names we refuse to rasterise.
const FORBIDDEN_ELEMENTS: &[&str] = &["filter", "mask", "clipPath", "use", "image"];

/// Attribute local-names that reference an unsupported filter / mask / clip.
const FORBIDDEN_ATTRIBUTES: &[&str] = &["filter", "mask", "clip-path"];

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("BUG: workspace root not above svg-compiler crate")
        .to_path_buf()
}

fn collect_svgs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => panic!("BUG: read {}: {e}", dir.display()),
    };
    for entry in entries {
        let entry = entry.expect("BUG: read entry");
        let path = entry.path();
        if path.is_dir() {
            if !is_cache_dir(&path) {
                collect_svgs(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "svg") {
            out.push(path);
        }
    }
}

/// Cache Directory Tagging spec (bford.info/cachedir) signature — the MD5
/// of ".IsCacheDirectory", hardcoded identically by cargo, GNU tar, and
/// backup tools. Cargo tags every target dir; local doc builds drop rustdoc
/// favicons there whose DOCTYPE roxmltree refuses to parse.
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

fn is_cache_dir(dir: &std::path::Path) -> bool {
    std::fs::read(dir.join("CACHEDIR.TAG")).is_ok_and(|tag| tag.starts_with(CACHEDIR_SIGNATURE))
}

/// Walk the parsed tree, returning one description per forbidden element or
/// attribute found, anchored on the element's tag.
fn scan_violations(doc: &roxmltree::Document<'_>) -> Vec<String> {
    let mut violations = Vec::new();
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        let tag = node.tag_name().name();
        if FORBIDDEN_ELEMENTS.contains(&tag) {
            violations.push(format!("<{tag}> element"));
        }
        for attr in node.attributes() {
            let name = attr.name();
            if FORBIDDEN_ATTRIBUTES.contains(&name) {
                violations.push(format!("<{tag} {name}=…>"));
            }
        }
    }
    violations
}

#[test]
fn shipping_svgs_do_not_use_unsupported_features() {
    let root = workspace_root();
    let mut svgs = Vec::new();
    for sub in SVG_ROOTS {
        collect_svgs(&root.join(sub), &mut svgs);
    }
    assert!(!svgs.is_empty(), "BUG: no shipping SVG assets discovered");

    let mut offenders: Vec<String> = Vec::new();
    for path in svgs {
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("BUG: read {}: {e}", path.display()));
        let doc = roxmltree::Document::parse(&contents)
            .unwrap_or_else(|e| panic!("BUG: parse {}: {e}", path.display()));
        for violation in scan_violations(&doc) {
            offenders.push(format!("{}: {violation}", path.display()));
        }
    }

    assert!(
        offenders.is_empty(),
        "shipping SVGs use unsupported features (move the effect into the SDK \
         draw tree, e.g. `Draw::with_drop_shadow`):\n  {}",
        offenders.join("\n  "),
    );
}

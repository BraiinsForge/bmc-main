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

//! Drift-guard: every widget's committed `manifest_params.rs` must match
//! what the codegen would emit for its current `manifest.json`.
//!
//! Scans every wasm-widget workspace root (`widgets-wasm-examples/`
//! for SDK examples and `widgets-wasm/` for production widgets)
//! so the check stays in lock-step with the multi-root layout used
//! by the Nix catalog (`workspace.nix:wasmWidgetCatalog`) and the justfile
//! (`bmc-wasm-runtime/tools/widget_root.py`). Add a new root here
//! when adding it there.
//!
//! Comparison normalises both sides through `prettyplease` after parsing
//! as `syn::File`, so cosmetic differences (line wraps the project
//! formatter introduces, copyright comments injected by `nix fmt`,
//! trailing whitespace) don't trip the guard — only structural drift
//! does. Doc comments survive the round-trip as `#[doc = "..."]`
//! attributes and are part of the diff.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use bmc_widget_manifest::Manifest;

#[test]
fn widget_manifest_params_are_up_to_date() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: bmc-widget-codegen has a parent workspace dir");

    let roots = [
        repo_root.join("widgets-wasm-examples"),
        repo_root.join("widgets-wasm"),
    ];

    let mut failures: Vec<String> = Vec::new();
    for root in &roots {
        check_root(root, &mut failures);
    }

    assert!(
        failures.is_empty(),
        "manifest_params.rs drift detected:\n  - {}",
        failures.join("\n  - "),
    );
}

fn check_root(root: &Path, failures: &mut Vec<String>) {
    let entries =
        fs::read_dir(root).unwrap_or_else(|e| panic!("BUG: cannot read {}: {e}", root.display()));
    for entry in entries {
        let widget_dir = entry.expect("BUG: dir entry read failed").path();
        let manifest_path = widget_dir.join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }

        let widget_name = widget_dir
            .file_name()
            .and_then(|s| s.to_str())
            .expect("BUG: widget dir has a non-UTF-8 name");

        check_widget(widget_name, &widget_dir, &manifest_path, failures);
    }
}

fn check_widget(name: &str, widget_dir: &Path, manifest_path: &Path, failures: &mut Vec<String>) {
    let manifest_src = fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
    let manifest = Manifest::from_str(&manifest_src)
        .unwrap_or_else(|e| panic!("parse {}: {e:#}", manifest_path.display()));

    let generated_path: PathBuf = widget_dir.join("src/manifest_params.rs");

    if manifest.params.is_empty() && manifest.credentials.is_empty() {
        if generated_path.exists() {
            failures.push(format!(
                "{name}: manifest declares no params or credentials but \
                 src/manifest_params.rs still exists — `just wasm::gen \
                 {name}` to remove it",
            ));
        }
        return;
    }

    if !generated_path.exists() {
        failures.push(format!(
            "{name}: manifest declares params or credentials but \
             src/manifest_params.rs is missing — \
             `just wasm::gen {name}` to create it",
        ));
        return;
    }

    let committed = fs::read_to_string(&generated_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", generated_path.display()));
    let generated = bmc_widget_codegen::generate(&manifest, "../manifest.json")
        .unwrap_or_else(|e| panic!("generate for {name}: {e:#}"));

    // Normalise both sides through `prettyplease` so the diff is structural:
    // formatter cosmetics + injected file-level comments fall away,
    // only item/attribute/doc-comment drift survives.
    let committed_ast = syn::parse_str::<syn::File>(&committed)
        .unwrap_or_else(|e| panic!("parse committed file for {name}: {e}"));
    let generated_ast = syn::parse_str::<syn::File>(&generated)
        .unwrap_or_else(|e| panic!("parse generated source for {name}: {e}"));

    let committed_norm = prettyplease::unparse(&committed_ast);
    let generated_norm = prettyplease::unparse(&generated_ast);

    if committed_norm != generated_norm {
        failures.push(format!(
            "{name}: src/manifest_params.rs is stale — \
             `just wasm::gen {name}` to refresh",
        ));
    }
}

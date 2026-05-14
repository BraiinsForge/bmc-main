// Copyright (C) 2026  Braiins Systems s.r.o.

//! Drift-guard: every example widget's committed `manifest_params.rs` must match
//! what the codegen would emit for its current `manifest.json`.
//!
//! Comparison normalises both sides through `prettyplease` after parsing as
//! `syn::File`, so cosmetic differences (line wraps the project formatter
//! introduces, copyright comments injected by `nix fmt`, trailing whitespace)
//! don't trip the guard — only structural drift does. Doc comments survive
//! the round-trip as `#[doc = "..."]` attributes and are part of the diff.

use std::fs;
use std::path::Path;
use std::str::FromStr as _;

use bmc_widget_manifest::Manifest;

#[test]
fn example_widgets_manifest_params_are_up_to_date() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: bmc-widget-codegen has a parent workspace dir")
        .join("bmc-wasm-runtime/examples");

    let mut failures: Vec<String> = Vec::new();

    let entries = fs::read_dir(&examples_dir)
        .unwrap_or_else(|e| panic!("BUG: cannot read {}: {e}", examples_dir.display()));
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

        let manifest_src = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
        let manifest = Manifest::from_str(&manifest_src)
            .unwrap_or_else(|e| panic!("parse {}: {e:#}", manifest_path.display()));

        let generated_path = widget_dir.join("src/manifest_params.rs");

        if manifest.params.is_empty() {
            if generated_path.exists() {
                failures.push(format!(
                    "{widget_name}: manifest declares no params but \
                     src/manifest_params.rs still exists — `just wasm::gen \
                     {widget_name}` to remove it",
                ));
            }
            continue;
        }

        if !generated_path.exists() {
            failures.push(format!(
                "{widget_name}: manifest declares params but \
                 src/manifest_params.rs is missing — \
                 `just wasm::gen {widget_name}` to create it",
            ));
            continue;
        }

        let committed = fs::read_to_string(&generated_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", generated_path.display()));
        let generated = bmc_widget_codegen::generate(&manifest, "../manifest.json")
            .unwrap_or_else(|e| panic!("generate for {widget_name}: {e:#}"));

        // Normalise both sides through `prettyplease` so the diff is structural:
        // formatter cosmetics + injected file-level comments fall away, only
        // item/attribute/doc-comment drift survives.
        let committed_ast = syn::parse_str::<syn::File>(&committed)
            .unwrap_or_else(|e| panic!("parse committed file for {widget_name}: {e}"));
        let generated_ast = syn::parse_str::<syn::File>(&generated)
            .unwrap_or_else(|e| panic!("parse generated source for {widget_name}: {e}"));

        let committed_norm = prettyplease::unparse(&committed_ast);
        let generated_norm = prettyplease::unparse(&generated_ast);

        if committed_norm != generated_norm {
            failures.push(format!(
                "{widget_name}: src/manifest_params.rs is stale — \
                 `just wasm::gen {widget_name}` to refresh",
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "manifest_params.rs drift detected:\n  - {}",
        failures.join("\n  - "),
    );
}

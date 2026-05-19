// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_widget_manifest::Manifest;
use std::str::FromStr;

/// Roots that host shipping widget manifests. Mirrors
/// `workspace.nix:wasmWidgetRoots` + `bmc-wasm-runtime/tools/widget_root.py`
/// for the wasm side, plus the native-widget `widgets/` root.
/// Adding a root means updating all three lists.
const MANIFEST_ROOTS: &[&str] = &["widgets", "bmc-wasm-runtime/examples", "widgets-wasm"];

fn manifest_paths() -> Vec<std::path::PathBuf> {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: no parent");
    let mut out = vec![];
    for root in MANIFEST_ROOTS {
        let dir = workspace.join(root);
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.expect("BUG: read entry");
                    let path = entry.path().join("manifest.json");
                    if path.exists() {
                        out.push(path);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("BUG: read {}: {e}", dir.display()),
        }
    }
    out
}

#[test]
fn every_shipping_manifest_loads_and_validates() {
    let paths = manifest_paths();
    assert!(!paths.is_empty(), "BUG: no shipping manifests found");
    for path in paths {
        let s = std::fs::read_to_string(&path).expect("BUG: read manifest");
        Manifest::from_str(&s).unwrap_or_else(|e| panic!("BUG: parse {path:?}: {e}"));
    }
}

#[test]
fn shipping_manifests_include_wasm_runtime_examples() {
    let paths = manifest_paths();
    let examples_root = std::path::Path::new("bmc-wasm-runtime/examples");
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(examples_root.join("hello-widget/manifest.json"))),
        "BUG: wasm runtime example manifests are not included in shipping manifest scan"
    );
}

#[test]
fn shipping_manifests_include_widgets_wasm() {
    let paths = manifest_paths();
    let widgets_wasm_root = std::path::Path::new("widgets-wasm");
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(widgets_wasm_root.join("clock/manifest.json"))),
        "BUG: widgets-wasm manifests are not included in shipping manifest scan"
    );
}

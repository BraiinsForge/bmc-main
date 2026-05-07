// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_widget::Manifest;
use std::str::FromStr;

fn manifest_paths() -> Vec<std::path::PathBuf> {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: no parent");
    let widgets_dir = workspace.join("widgets");
    let mut out = vec![];
    for entry in std::fs::read_dir(&widgets_dir).expect("BUG: read widgets dir") {
        let entry = entry.expect("BUG: read entry");
        let path = entry.path().join("manifest.json");
        if path.exists() {
            out.push(path);
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

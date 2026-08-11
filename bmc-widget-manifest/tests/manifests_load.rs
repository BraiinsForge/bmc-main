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

use bmc_widget_manifest::Manifest;
use std::str::FromStr;

/// Roots that host shipping widget manifests. Mirrors
/// `workspace.nix:wasmWidgetRoots` + `bmc-wasm-runtime/tools/widget_root.py`
/// for the wasm side, plus the native-widget `widgets/` root.
/// Adding a root means updating all three lists.
const MANIFEST_ROOTS: &[&str] = &["widgets", "widgets-wasm-examples", "widgets-wasm"];

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
fn shipping_manifests_include_wasm_examples() {
    let paths = manifest_paths();
    let examples_root = std::path::Path::new("widgets-wasm-examples");
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(examples_root.join("hello-widget/manifest.json"))),
        "BUG: wasm example manifests are not included in shipping manifest scan"
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

fn load_shipping_manifest(path: &str) -> Manifest {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: no parent");
    let path = workspace.join(path);
    let s = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("BUG: read {}: {e}", path.display()));
    Manifest::from_str(&s).unwrap_or_else(|e| panic!("BUG: parse {}: {e}", path.display()))
}

#[test]
fn shipping_manifests_support_bmm_rectangular_fullscreen_viewports() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: no parent");
    for path in manifest_paths() {
        let manifest = load_shipping_manifest(
            path.strip_prefix(workspace)
                .expect("BUG: manifest path under workspace")
                .to_str()
                .expect("BUG: workspace path must be utf-8"),
        );
        // A widget must cover both standard BMM fullscreen sizes only if its
        // declared rectangular range reaches small screens; a Deck-only widget
        // (min_width 1280) has opted out of them by declaration.
        let reaches_small_screens = manifest.supported_viewports.iter().any(|v| {
            v.viewport_shape == bmc_widget_manifest::ViewportShape::Rectangular
                && v.min_width.is_none_or(|min| min <= 480)
        });
        if !reaches_small_screens {
            continue;
        }
        assert!(
            manifest.supports_viewport(bmc_widget_manifest::ViewportShape::Rectangular, 320, 240),
            "BUG: {} must support BMM100 fullscreen viewport",
            path.display(),
        );
        assert!(
            manifest.supports_viewport(bmc_widget_manifest::ViewportShape::Rectangular, 480, 320),
            "BUG: {} must support BMM101 fullscreen viewport",
            path.display(),
        );
    }
}

#[test]
fn round_viewport_support_is_limited_to_the_expected_widgets() {
    let mut round_manifest_paths: Vec<_> = manifest_paths()
        .into_iter()
        .filter(|path| {
            let s = std::fs::read_to_string(path).expect("BUG: read manifest");
            let manifest = Manifest::from_str(&s).expect("BUG: parse manifest");
            manifest
                .supported_viewports
                .iter()
                .any(|v| v.viewport_shape == bmc_widget_manifest::ViewportShape::Round)
        })
        .map(|path| {
            path.strip_prefix(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("BUG: no parent"),
            )
            .expect("BUG: manifest path under workspace")
            .to_owned()
        })
        .collect();
    round_manifest_paths.sort();

    assert_eq!(
        round_manifest_paths,
        vec![
            std::path::PathBuf::from("widgets-wasm/blockheight/manifest.json"),
            std::path::PathBuf::from("widgets-wasm/clock/manifest.json"),
            std::path::PathBuf::from("widgets-wasm/halving-countdown/manifest.json"),
            std::path::PathBuf::from("widgets-wasm/mining-clock/manifest.json"),
            std::path::PathBuf::from("widgets-wasm/mining-info/manifest.json"),
        ],
    );
}

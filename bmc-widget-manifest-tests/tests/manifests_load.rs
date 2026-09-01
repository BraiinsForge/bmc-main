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

use std::path::{Path, PathBuf};
use std::str::FromStr;

use bmc_widget_manifest::{Manifest, ViewportShape};

const MANIFEST_ROOTS: &[&str] = &["widgets", "widgets-wasm", "widgets-wasm-examples"];
const COMMITTED_SCHEMA: &str = include_str!("../../bmc-widget-manifest/manifest.schema.json");

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: manifest test crate must be below workspace root")
        .to_path_buf()
}

fn manifest_paths() -> Vec<PathBuf> {
    let workspace = workspace_root();
    let mut paths = Vec::new();
    for root in MANIFEST_ROOTS {
        let dir = workspace.join(root);
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let path = entry
                        .expect("BUG: read manifest root entry")
                        .path()
                        .join("manifest.json");
                    if path.exists() {
                        paths.push(path);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("BUG: read {}: {error}", dir.display()),
        }
    }
    paths
}

fn load_manifest(path: &Path) -> Manifest {
    let json = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("BUG: read {}: {error}", path.display()));
    Manifest::from_str(&json)
        .unwrap_or_else(|error| panic!("BUG: parse {}: {error}", path.display()))
}

#[test]
fn every_shipping_manifest_loads_and_validates() {
    let paths = manifest_paths();
    assert!(!paths.is_empty(), "BUG: no shipping manifests found");
    for path in paths {
        load_manifest(&path);
    }
}

#[test]
fn shipping_manifest_scan_covers_every_root() {
    let workspace = workspace_root();
    let paths = manifest_paths();
    for expected in [
        "widgets/flip-clock/manifest.json",
        "widgets-wasm/clock/manifest.json",
        "widgets-wasm-examples/hello-widget/manifest.json",
    ] {
        assert!(
            paths.iter().any(|path| path == &workspace.join(expected)),
            "shipping manifest scan misses {expected}"
        );
    }
}

#[test]
fn every_example_manifest_validates_against_schema_and_parser() {
    let schema: serde_json::Value =
        serde_json::from_str(COMMITTED_SCHEMA).expect("BUG: committed schema must parse as JSON");
    let validator = jsonschema::validator_for(&schema).expect("BUG: committed schema must compile");
    let example_root = workspace_root().join("widgets-wasm-examples");
    let paths: Vec<_> = manifest_paths()
        .into_iter()
        .filter(|path| path.starts_with(&example_root))
        .collect();
    assert!(!paths.is_empty(), "BUG: no example manifests found");

    for path in paths {
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("BUG: read {}: {error}", path.display()));
        let instance: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or_else(|error| panic!("BUG: {} is not JSON: {error}", path.display()));
        assert!(
            validator.is_valid(&instance),
            "{} failed the JSON Schema validator. Errors:\n  - {}",
            path.display(),
            validator
                .iter_errors(&instance)
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n  - "),
        );
        Manifest::from_str(&body)
            .unwrap_or_else(|error| panic!("BUG: {} failed to parse: {error}", path.display()));
    }
}

#[test]
fn shipping_manifests_support_bmm_rectangular_fullscreen_viewports() {
    for path in manifest_paths() {
        let manifest = load_manifest(&path);
        let reaches_small_screens = manifest.supported_viewports.iter().any(|viewport| {
            viewport.viewport_shape == ViewportShape::Rectangular
                && viewport.min_width.is_none_or(|minimum| minimum <= 480)
        });
        if reaches_small_screens {
            assert!(
                manifest.supports_viewport(ViewportShape::Rectangular, 320, 240),
                "BUG: {} must support BMM100 fullscreen viewport",
                path.display()
            );
            assert!(
                manifest.supports_viewport(ViewportShape::Rectangular, 480, 320),
                "BUG: {} must support BMM101 fullscreen viewport",
                path.display()
            );
        }
    }
}

#[test]
fn round_viewport_support_is_limited_to_the_expected_widgets() {
    let workspace = workspace_root();
    let mut round_paths: Vec<_> = manifest_paths()
        .into_iter()
        .filter(|path| {
            load_manifest(path)
                .supported_viewports
                .iter()
                .any(|viewport| viewport.viewport_shape == ViewportShape::Round)
        })
        .map(|path| {
            path.strip_prefix(&workspace)
                .expect("BUG: manifest must be below workspace")
                .to_owned()
        })
        .collect();
    round_paths.sort();
    assert_eq!(
        round_paths,
        [
            "widgets-wasm/blockheight/manifest.json",
            "widgets-wasm/clock/manifest.json",
            "widgets-wasm/halving-countdown/manifest.json",
            "widgets-wasm/miner-info-mining/manifest.json",
            "widgets-wasm/mining-clock/manifest.json",
            "widgets-wasm/mining-info/manifest.json",
            // Last because a `PathBuf` sort compares components,
            // and `widgets-wasm` is a prefix of `widgets-wasm-examples`.
            // The SDK's demo widget lays out for the round BFM100 so
            // the testbed has something to show on every catalog platform.
            "widgets-wasm-examples/hello-widget/manifest.json",
        ]
        .map(PathBuf::from)
    );
}

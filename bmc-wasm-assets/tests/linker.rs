// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use bmc_wasm_assets::rewrite_package_asset_sections;
use wasmparser::{DataKind, Parser, Payload};

const TARGET: &str = "wasm32-unknown-unknown";

#[test]
fn bare_link_section_retains_only_selected_assets_outside_linear_memory() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("asset crate is below workspace root")
        .to_owned();
    let temporary = tempfile::tempdir().expect("create linker fixture target directory");
    for profile in [Profile::Development, Profile::Release] {
        build_fixture(&workspace, temporary.path(), profile);
        let wasm = fs::read(profile.output(temporary.path())).expect("read linked fixture module");
        let rewritten = rewrite_package_asset_sections(&wasm).expect("parse linked asset records");
        let mut names = rewritten
            .records
            .iter()
            .map(|record| record.logical_name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            ["fixture-dep::texture.jpg", "linker-widget::falcon-9.png",]
        );
        assert_payload_absent_from_data_segments(&workspace, &wasm);
        assert!(
            data_segments(&rewritten.wasm)
                .iter()
                .all(|segment| !segment.windows(8).any(|window| window == b"BMCASSET")),
            "stripped {profile:?} module retains record framing in linear memory"
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum Profile {
    Development,
    Release,
}

impl Profile {
    fn output(self, target_dir: &Path) -> PathBuf {
        let directory = match self {
            Self::Development => "debug",
            Self::Release => "release",
        };
        target_dir
            .join(TARGET)
            .join(directory)
            .join("bmc_wasm_assets_linker_widget.wasm")
    }
}

fn build_fixture(workspace: &Path, target_dir: &Path, profile: Profile) {
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", target_dir)
        .args([
            "build",
            "--locked",
            "--package",
            "bmc-wasm-assets-linker-widget",
            "--target",
            TARGET,
        ]);
    if matches!(profile, Profile::Release) {
        command.arg("--release");
    }
    let status = command.status().expect("run linker fixture Cargo build");
    assert!(status.success(), "{profile:?} linker fixture build failed");
}

fn assert_payload_absent_from_data_segments(workspace: &Path, wasm: &[u8]) {
    for relative in [
        "widgets-wasm/iss-position/src/render/texture.jpg",
        "widgets-wasm/spacex-launch/assets/falcon-9.png",
        "widgets-wasm/spacex-launch/assets/falcon-heavy.png",
    ] {
        let payload =
            fs::read(workspace.join(relative)).expect("read checked-in linker fixture payload");
        assert!(
            data_segments(wasm)
                .iter()
                .all(|segment| !contains_subslice(segment, &payload)),
            "{relative} was duplicated into linear memory"
        );
    }
}

fn data_segments(wasm: &[u8]) -> Vec<&[u8]> {
    let mut segments = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::DataSection(reader) = payload.expect("parse linked fixture module") {
            for data in reader {
                let data = data.expect("parse linked fixture data segment");
                if matches!(data.kind, DataKind::Active { .. }) {
                    segments.push(data.data);
                }
            }
        }
    }
    segments
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

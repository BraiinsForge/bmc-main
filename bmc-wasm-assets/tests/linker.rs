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

use bmc_wasm_assets::{extract_package_assets, package_asset_id, rewrite_package_asset_sections};
use bmc_wasm_protocol::{PackageAssetId, PackageAssetKind};
use wasmparser::{DataKind, Parser, Payload};

const TARGET: &str = "wasm32-unknown-unknown";

#[test]
fn extractor_recovers_only_referenced_dependency_assets_outside_linear_memory() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("asset crate is below workspace root")
        .to_owned();
    let temporary = tempfile::tempdir().expect("create linker fixture target directory");
    for profile in [Profile::Development, Profile::Release] {
        build_fixture(&workspace, temporary.path(), profile);
        let compiler_wasm = profile.output(temporary.path());
        let wasm = fs::read(&compiler_wasm).expect("read linked fixture module");
        let package = temporary.path().join(profile.directory()).join("package");
        let stripped = package.join("widget.wasm");
        let assets = package.join("assets");
        extract_package_assets(
            &compiler_wasm,
            Some(&profile.artifact_root(temporary.path())),
            &stripped,
            &assets,
        )
        .expect("recover dependency package assets");

        let dependency_payload =
            fs::read(workspace.join("widgets-wasm/iss-position/src/render/texture.jpg"))
                .expect("read dependency fixture payload");
        let widget_payload =
            fs::read(workspace.join("widgets-wasm/spacex-launch/assets/falcon-9.png"))
                .expect("read widget fixture payload");
        let unused_payload =
            fs::read(workspace.join("widgets-wasm/spacex-launch/assets/falcon-heavy.png"))
                .expect("read unused dependency fixture payload");
        assert_eq!(
            fs::read(asset_path(
                &assets,
                PackageAssetKind::Bitmap,
                package_asset_id(PackageAssetKind::Bitmap, &dependency_payload),
            ))
            .expect("read recovered dependency asset"),
            dependency_payload
        );
        assert_eq!(
            fs::read(asset_path(
                &assets,
                PackageAssetKind::Bitmap,
                package_asset_id(PackageAssetKind::Bitmap, &widget_payload),
            ))
            .expect("read extracted widget asset"),
            widget_payload
        );
        assert!(
            !asset_path(
                &assets,
                PackageAssetKind::Bitmap,
                package_asset_id(PackageAssetKind::Bitmap, &unused_payload),
            )
            .exists(),
            "an unreferenced dependency asset must not enter the widget package"
        );
        assert_payload_absent_from_data_segments(&workspace, &wasm);
        assert_payload_absent_from_data_segments(
            &workspace,
            &fs::read(stripped).expect("read stripped fixture module"),
        );
    }
}

fn asset_path(root: &Path, kind: PackageAssetKind, id: PackageAssetId) -> PathBuf {
    root.join("v1")
        .join(kind.as_str())
        .join(format!("{id}.asset"))
}

#[test]
fn asset_macros_emit_every_kind_only_in_custom_sections() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("asset crate is below workspace root")
        .to_owned();
    let temporary = tempfile::tempdir().expect("create macro fixture target directory");
    for profile in [Profile::Development, Profile::Release] {
        build_macro_fixture(&workspace, temporary.path(), profile);
        let wasm = fs::read(profile.macro_output(temporary.path()))
            .expect("read linked macro fixture module");
        let rewritten = rewrite_package_asset_sections(&wasm).expect("parse macro asset records");
        let mut kinds = rewritten
            .records
            .iter()
            .map(|record| record.kind.as_str())
            .collect::<Vec<_>>();
        kinds.sort_unstable();
        assert_eq!(
            kinds,
            [
                "audio", "bitmap", "bitmap", "bitmap", "bitmap", "bitmap", "mesh", "svg"
            ]
        );
        assert_macro_payloads_absent_from_data_segments(&workspace, &wasm);
    }
}

#[derive(Clone, Copy, Debug)]
enum Profile {
    Development,
    Release,
}

impl Profile {
    const fn directory(self) -> &'static str {
        match self {
            Self::Development => "debug",
            Self::Release => "release",
        }
    }

    fn output(self, target_dir: &Path) -> PathBuf {
        target_dir
            .join(TARGET)
            .join(self.directory())
            .join("bmc_wasm_assets_linker_widget.wasm")
    }

    fn artifact_root(self, target_dir: &Path) -> PathBuf {
        target_dir.join(TARGET).join(self.directory()).join("deps")
    }

    fn macro_output(self, target_dir: &Path) -> PathBuf {
        target_dir
            .join(TARGET)
            .join(self.directory())
            .join("bmc_wasm_assets_macro_widget.wasm")
    }
}

fn build_fixture(workspace: &Path, target_dir: &Path, profile: Profile) {
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_INCREMENTAL", "0")
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

fn build_macro_fixture(workspace: &Path, target_dir: &Path, profile: Profile) {
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", target_dir)
        .args([
            "build",
            "--locked",
            "--package",
            "bmc-wasm-assets-macro-widget",
            "--target",
            TARGET,
        ]);
    if matches!(profile, Profile::Release) {
        command.arg("--release");
    }
    let status = command.status().expect("run macro fixture Cargo build");
    assert!(status.success(), "{profile:?} macro fixture build failed");
}

fn assert_macro_payloads_absent_from_data_segments(workspace: &Path, wasm: &[u8]) {
    for relative in [
        "widgets-wasm/spacex-launch/assets/unknown.png",
        "widgets-wasm-examples/metronome/assets/sounds/Perc_MetronomeQuartz_lo.wav",
    ] {
        let payload = fs::read(workspace.join(relative)).expect("read macro fixture payload");
        assert!(
            data_segments(wasm)
                .iter()
                .all(|segment| !contains_subslice(segment, &payload)),
            "{relative} was duplicated into linear memory"
        );
    }
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

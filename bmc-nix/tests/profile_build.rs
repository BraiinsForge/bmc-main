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

use std::os::unix::fs::PermissionsExt;

use bmc_nix::types::*;
use tempfile::TempDir;

fn create_fake_store_path(
    base: &std::path::Path,
    name: &str,
    files: &[(&str, &str)],
) -> std::path::PathBuf {
    let store_path = base.join(format!("nix-store-{name}"));
    for (path, content) in files {
        let file_path = store_path.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("BUG: create dirs");
        }
        std::fs::write(&file_path, content).expect("BUG: write file");
    }
    store_path
}

/// Create an activation entrypoint in a store path that atomically switches
/// the `current` symlink (mimics what hook_activation_resolver would generate).
fn create_activation_entrypoint(store_path: &std::path::Path) {
    let activation_dir = store_path.join("core/activation");
    std::fs::create_dir_all(&activation_dir).expect("BUG: create activation dir");
    let entrypoint = activation_dir.join("entrypoint");
    std::fs::write(
        &entrypoint,
        r#"#!/bin/sh
set -euxo pipefail
profile_dir="$(dirname "$PROFILE_NEW_GENERATION")"
current_link="$profile_dir/current"
gen_dir_name="$(basename "$PROFILE_NEW_GENERATION")"
tmp_link="$profile_dir/current.tmp"
rm -f "$tmp_link"
ln -s "$gen_dir_name" "$tmp_link"
mv -Tf "$tmp_link" "$current_link"
"#,
    )
    .expect("BUG: write entrypoint");
    std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
        .expect("BUG: set permissions");
}

#[tokio::test]
#[expect(clippy::too_many_lines)]
async fn full_profile_build() {
    let tmp = TempDir::new().expect("BUG: create temp dir");

    // Create fake store paths
    let core_store = create_fake_store_path(
        tmp.path(),
        "core",
        &[
            ("bin/core-binary", "#!/bin/sh\necho core"),
            ("lib/libcore.so", "fake-lib"),
        ],
    );

    let widget_store = create_fake_store_path(
        tmp.path(),
        "widget-clock",
        &[
            ("bin/widget-clock", "#!/bin/sh\necho clock"),
            ("lib/bmc-widgets/clock/manifest.json", "{}"),
        ],
    );

    // Add activation entrypoint to the core store (simulates bmc-nix-activation package)
    create_activation_entrypoint(&core_store);

    // Create PackageIndex
    let index = PackageIndex {
        version: 1,
        provenance: Some(Provenance {
            commit: "test123".into(),
        }),
        indexes: vec![],
        caches: vec![CacheEntry {
            name: "default".into(),
            cache_url: "https://cache.test.com".into(),
            cache_key: "test-key".into(),
        }],
        packages: vec![
            PackageEntry {
                name: "core".into(),
                version: "1.0.0".into(),
                cache: None,
                store_path: core_store
                    .to_str()
                    .expect("BUG: core store path should be valid UTF-8")
                    .into(),
                category: Some("core".into()),
                description: Some("Core application".into()),
                upgrade_strategy: None,
                install_strategy: None,
                server_id: String::new(),
                metadata: std::collections::BTreeMap::new(),
            },
            PackageEntry {
                name: "widget-clock".into(),
                version: "0.5.0".into(),
                cache: None,
                store_path: widget_store
                    .to_str()
                    .expect("BUG: widget store path should be valid UTF-8")
                    .into(),
                category: Some("widget".into()),
                description: Some("Clock widget".into()),
                upgrade_strategy: None,
                install_strategy: None,
                server_id: String::new(),
                metadata: std::collections::BTreeMap::new(),
            },
        ],
    };

    // Resolve packages
    let packages = bmc_nix::index::resolve_all_from_index(&index, &["core".into()])
        .expect("the profile index contains every required system package");
    assert_eq!(packages.len(), 2);

    // Profile dir
    let profile_dir = tmp.path().join("profiles/bmc");
    std::fs::create_dir_all(&profile_dir).expect("BUG: create profile dir");

    // Generation 1
    let gen_num = bmc_nix::profile::max_generation(&profile_dir)
        .expect("BUG: scan generations should succeed")
        .unwrap_or(0)
        + 1;
    assert_eq!(gen_num, 1);

    let gen1 = bmc_nix::profile::build_profile(&profile_dir, gen_num, &packages, "hooks", None)
        .await
        .expect("BUG: build_profile should succeed");

    assert_eq!(gen1.number, 1);
    assert!(gen1.path.exists());
    assert!(gen1.path.ends_with("1-link"));

    // Verify symlinks
    assert!(gen1.path.join("bin/core-binary").is_symlink());
    assert!(gen1.path.join("bin/widget-clock").is_symlink());
    assert!(gen1.path.join("lib/libcore.so").is_symlink());
    assert!(gen1.path.join("lib/bmc-widgets").is_symlink());
    assert!(
        gen1.path
            .join("lib/bmc-widgets/clock/manifest.json")
            .exists()
    );

    // Verify manifest
    let manifest_path = gen1.path.join("manifest");
    assert!(manifest_path.exists());
    let manifest_content =
        std::fs::read_to_string(&manifest_path).expect("BUG: read manifest should succeed");
    let manifest: bmc_nix::types::Manifest =
        serde_json::from_str(&manifest_content).expect("BUG: parse manifest should succeed");
    assert_eq!(manifest.packages.len(), 2);
    let core = manifest
        .packages
        .get("core")
        .expect("BUG: core should be in the manifest");
    let widget = manifest
        .packages
        .get("widget-clock")
        .expect("BUG: widget-clock should be in the manifest");
    // Only the required packages (core, nix) may persist as `system`;
    // a factory-shipped widget must persist as `user` or its later
    // removal from the server index would block every upgrade.
    assert_eq!(core.installed_by, InstalledBy::System);
    assert_eq!(widget.installed_by, InstalledBy::User);

    // Activate (runs entrypoint which creates current symlink)
    bmc_nix::profile::activate_profile(&profile_dir, gen1.number, &gen1.path, None)
        .await
        .expect("BUG: activate should succeed");
    let current = profile_dir.join("current");
    assert!(current.is_symlink());

    // Generation 2
    let gen2_num = bmc_nix::profile::max_generation(&profile_dir)
        .expect("BUG: scan generations for gen 2 should succeed")
        .unwrap_or(0)
        + 1;
    assert_eq!(gen2_num, 2);

    let gen2 = bmc_nix::profile::build_profile(&profile_dir, gen2_num, &packages, "hooks", None)
        .await
        .expect("BUG: build_profile gen 2 should succeed");
    assert!(gen2.path.ends_with("2-link"));

    bmc_nix::profile::activate_profile(&profile_dir, gen2.number, &gen2.path, None)
        .await
        .expect("BUG: activate gen 2 should succeed");

    // Verify current points to gen 2
    let current_target =
        std::fs::read_link(&current).expect("BUG: read current symlink should succeed");
    assert_eq!(
        current_target
            .to_str()
            .expect("BUG: current target path should be valid UTF-8"),
        "2-link"
    );

    assert!(
        profile_dir.join("previous").symlink_metadata().is_err(),
        "bmc-nix must never write a previous symlink"
    );
}

// Copyright (C) 2025  Braiins Systems s.r.o.

mod common;

use std::collections::BTreeMap;

use bmc_nix::index::merge_indexes;
use bmc_nix::manifest::{compute_upgrade_plan, read_manifest};
use bmc_nix::types::*;
use bmc_nix::upgrade::merge_installed_with_new;
use common::{create_activation_entrypoint, create_fake_store_path};
use tempfile::TempDir;

/// Build a merged index from test entries with uniform priority.
fn build_test_merged_index(entries: &[(&str, &str, &str, &str)]) -> MergedIndex {
    let mut by_server: BTreeMap<String, Vec<(&str, &str, &str)>> = BTreeMap::new();
    for &(name, version, store_path, server_id) in entries {
        by_server
            .entry(server_id.to_owned())
            .or_default()
            .push((name, version, store_path));
    }

    let mut all_fetched = Vec::new();
    let mut priorities = BTreeMap::new();
    for (server_id, pkgs) in &by_server {
        priorities.insert(server_id.clone(), 1_u32);
        let cache_name = format!("cache-{server_id}");
        let packages: Vec<PackageEntry> = pkgs
            .iter()
            .map(|(name, version, store_path)| PackageEntry {
                name: (*name).into(),
                version: (*version).into(),
                cache: Some(cache_name.clone()),
                store_path: (*store_path).into(),
                category: None,
                description: None,
                upgrade_strategy: None,
                install_strategy: None,
                server_id: String::new(),
            })
            .collect();
        let index = PackageIndex {
            version: 1,
            provenance: None,
            indexes: vec![],
            caches: vec![CacheEntry {
                name: cache_name.clone(),
                cache_url: format!("https://{cache_name}.example.com"),
                cache_key: format!("{cache_name}:KEY"),
            }],
            packages,
        };
        all_fetched.push((server_id.clone(), index));
    }

    merge_indexes(all_fetched, &priorities)
}

/// Integration test: simulate a full upgrade cycle.
///
/// 1. Create initial profile (generation 1) with packages A@1.0, B@1.0
/// 2. Create a "new" merged index with A@2.0, B@1.0, C@1.0
/// 3. Compute upgrade plan (A changes, C added via add_packages)
/// 4. Build new generation (generation 2)
/// 5. Activate generation 2
/// 6. Verify manifest, symlink tree, current symlink
/// 7. Run GC (generation 1 kept since keep_generations=2)
#[tokio::test]
#[expect(clippy::too_many_lines)]
async fn full_upgrade_cycle() {
    let tmp = TempDir::new().expect("BUG: create temp dir");

    // === Phase 1: Initial install (generation 1) ===

    let core_store_v1 = create_fake_store_path(
        tmp.path(),
        "core",
        "1.0.0",
        &[("bin/core-app", "#!/bin/sh\necho core v1")],
    );
    create_activation_entrypoint(&core_store_v1);

    let widget_store_v1 = create_fake_store_path(
        tmp.path(),
        "widget",
        "1.0.0",
        &[("bin/widget-clock", "#!/bin/sh\necho widget v1")],
    );

    let initial_packages = vec![
        ResolvedPackage {
            name: "core".into(),
            version: "1.0.0".into(),
            store_path: core_store_v1.to_str().expect("BUG: valid UTF-8").into(),
            cache_url: "https://cache.example.com".into(),
            cache_name: "default".into(),
            category: Some("core".into()),
            description: None,
            upgrade_strategy: Some(UpgradeStrategy::Reboot),
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: PinStrategy::None,
        },
        ResolvedPackage {
            name: "widget".into(),
            version: "1.0.0".into(),
            store_path: widget_store_v1.to_str().expect("BUG: valid UTF-8").into(),
            cache_url: "https://cache.example.com".into(),
            cache_name: "default".into(),
            category: Some("widget".into()),
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "braiins".into(),
            pinned: PinStrategy::None,
        },
    ];

    let profile_dir = tmp.path().join("profiles/bmc");

    let gen1 = bmc_nix::profile::build_profile(&profile_dir, 1, &initial_packages, "hooks", None)
        .await
        .expect("BUG: build_profile gen 1 failed");

    assert_eq!(gen1.number, 1);
    assert!(gen1.path.join("bin/core-app").is_symlink());
    assert!(gen1.path.join("bin/widget-clock").is_symlink());

    // Activate generation 1
    bmc_nix::profile::activate_profile(&profile_dir, gen1.number, &gen1.path)
        .await
        .expect("BUG: activate gen 1 failed");

    let current_link = profile_dir.join("current");
    assert!(current_link.is_symlink());
    let current_target =
        std::fs::read_link(&current_link).expect("BUG: read current symlink failed");
    assert_eq!(current_target.to_str().expect("BUG: valid UTF-8"), "1-link");

    // === Phase 2: Upgrade ===

    // Create new store paths for upgraded/new packages
    let core_store_v2 = create_fake_store_path(
        tmp.path(),
        "core",
        "2.0.0",
        &[("bin/core-app", "#!/bin/sh\necho core v2")],
    );
    create_activation_entrypoint(&core_store_v2);

    let gadget_store = create_fake_store_path(
        tmp.path(),
        "gadget",
        "1.0.0",
        &[("bin/gadget-tool", "#!/bin/sh\necho gadget")],
    );

    // Build merged index with A@2.0, B@1.0 (widget stays same)
    let merged = build_test_merged_index(&[
        (
            "core",
            "2.0.0",
            core_store_v2.to_str().expect("BUG: valid UTF-8"),
            "braiins",
        ),
        (
            "widget",
            "1.0.0",
            widget_store_v1.to_str().expect("BUG: valid UTF-8"),
            "braiins",
        ),
    ]);

    // Read current manifest
    let current_manifest = read_manifest(&gen1.path).expect("BUG: read manifest failed");
    assert_eq!(current_manifest.packages.len(), 2);

    // Compute upgrade plan (core changes, gadget added)
    let gadget_resolved = ResolvedPackage {
        name: "gadget".into(),
        version: "1.0.0".into(),
        store_path: gadget_store.to_str().expect("BUG: valid UTF-8").into(),
        cache_url: "https://cache.example.com".into(),
        cache_name: "default".into(),
        category: None,
        description: None,
        upgrade_strategy: None,
        install_strategy: None,
        installed_by: InstalledBy::User,
        installed_from: "braiins".into(),
        pinned: PinStrategy::None,
    };

    let plan = compute_upgrade_plan(&current_manifest, Some(&merged), &[gadget_resolved], &[])
        .expect("BUG: compute_upgrade_plan failed");

    // Verify plan
    assert_eq!(plan.changed.len(), 1, "core should change version");
    assert_eq!(plan.changed[0].name, "core");
    assert_eq!(plan.changed[0].from_version, "1.0.0");
    assert_eq!(plan.changed[0].to_version, "2.0.0");
    assert_eq!(plan.added.len(), 1, "gadget should be added");
    assert_eq!(plan.added[0].name, "gadget");
    assert!(plan.removed.is_empty());
    assert!(plan.stale.is_empty());
    assert_eq!(plan.packages.len(), 3, "core + widget + gadget");

    // Merge installed with plan packages
    let all_packages = merge_installed_with_new(&current_manifest, &plan.packages);
    assert_eq!(all_packages.len(), 3);

    // Build generation 2
    let gen2 = bmc_nix::profile::build_profile(&profile_dir, 2, &all_packages, "hooks", None)
        .await
        .expect("BUG: build_profile gen 2 failed");

    assert_eq!(gen2.number, 2);
    assert!(gen2.path.join("bin/core-app").is_symlink());
    assert!(gen2.path.join("bin/widget-clock").is_symlink());
    assert!(gen2.path.join("bin/gadget-tool").is_symlink());

    // Verify gen2 core-app points to v2 store path
    let core_link =
        std::fs::read_link(gen2.path.join("bin/core-app")).expect("BUG: read core-app symlink");
    assert!(
        core_link
            .to_str()
            .expect("BUG: valid UTF-8")
            .contains("core-2.0.0"),
        "core-app should point to v2 store"
    );

    // Verify gen2 manifest
    let gen2_manifest = read_manifest(&gen2.path).expect("BUG: read gen2 manifest");
    assert_eq!(gen2_manifest.packages.len(), 3);
    assert_eq!(
        gen2_manifest
            .packages
            .get("core")
            .expect("BUG: core")
            .version,
        "2.0.0"
    );
    assert!(gen2_manifest.packages.contains_key("gadget"));

    // Activate generation 2
    bmc_nix::profile::activate_profile(&profile_dir, gen2.number, &gen2.path)
        .await
        .expect("BUG: activate gen 2 failed");

    let current_target = std::fs::read_link(&current_link).expect("BUG: read current symlink");
    assert_eq!(current_target.to_str().expect("BUG: valid UTF-8"), "2-link");

    // === Phase 3: GC ===

    let gc_config = GcConfig {
        keep_generations: 2,
        keep_days: 0,
        min_free_space: "0".into(),
        protected_generations: vec![],
    };

    bmc_nix::gc::cleanup_generations(&profile_dir, &gc_config)
        .expect("BUG: cleanup_generations failed");

    // Both generations should be kept (keep_generations=2)
    assert!(
        profile_dir.join("1-link").exists(),
        "gen 1 should be kept (keep_generations=2)"
    );
    assert!(profile_dir.join("2-link").exists(), "gen 2 should be kept");
}

/// Verify that GC removes old generations during an upgrade cycle.
#[tokio::test]
async fn gc_removes_old_generations_during_upgrade() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Create 3 generations manually
    for gen_num in 1..=3 {
        let store = create_fake_store_path(
            tmp.path(),
            "pkg",
            &format!("{gen_num}.0.0"),
            &[("bin/app", &format!("v{gen_num}"))],
        );
        create_activation_entrypoint(&store);

        let packages = vec![ResolvedPackage {
            name: "pkg".into(),
            version: format!("{gen_num}.0.0"),
            store_path: store.to_str().expect("BUG: valid UTF-8").into(),
            cache_url: String::new(),
            cache_name: "default".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }];

        let generation =
            bmc_nix::profile::build_profile(&profile_dir, gen_num, &packages, "hooks", None)
                .await
                .expect("BUG: build_profile failed");

        bmc_nix::profile::activate_profile(&profile_dir, generation.number, &generation.path)
            .await
            .expect("BUG: activate failed");
    }

    // Verify all 3 generations exist
    assert!(profile_dir.join("1-link").exists());
    assert!(profile_dir.join("2-link").exists());
    assert!(profile_dir.join("3-link").exists());

    // GC with keep_generations=1 + protected=[1]
    let gc_config = GcConfig {
        keep_generations: 1,
        keep_days: 0,
        min_free_space: "0".into(),
        protected_generations: vec![1],
    };

    bmc_nix::gc::cleanup_generations(&profile_dir, &gc_config)
        .expect("BUG: cleanup_generations failed");

    // Gen 1 protected, gen 3 is current + most recent -> gen 2 removed
    assert!(
        profile_dir.join("1-link").exists(),
        "gen 1 should be kept (protected)"
    );
    assert!(
        !profile_dir.join("2-link").exists(),
        "gen 2 should be removed"
    );
    assert!(
        profile_dir.join("3-link").exists(),
        "gen 3 should be kept (current + most recent)"
    );
}

/// Verify that a stale pinned package (no matching version in index) is kept
/// as-is in the new profile with its original store path files intact.
#[tokio::test]
async fn stale_pinned_package_kept_in_new_profile() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Gen 1: widget@1.0.0 pinned to Major
    let store_v1 = create_fake_store_path(
        tmp.path(),
        "widget",
        "1.0.0",
        &[("bin/widget-app", "#!/bin/sh\necho widget v1")],
    );
    create_activation_entrypoint(&store_v1);

    let packages_v1 = vec![ResolvedPackage {
        name: "widget".into(),
        version: "1.0.0".into(),
        store_path: store_v1.to_str().expect("BUG: valid UTF-8").into(),
        cache_url: String::new(),
        cache_name: "default".into(),
        category: None,
        description: None,
        upgrade_strategy: None,
        install_strategy: None,
        installed_by: InstalledBy::System,
        installed_from: "braiins".into(),
        pinned: PinStrategy::Major,
    }];

    let gen1 = bmc_nix::profile::build_profile(&profile_dir, 1, &packages_v1, "hooks", None)
        .await
        .expect("BUG: build gen 1 failed");

    let manifest = read_manifest(&gen1.path).expect("BUG: read manifest");

    // Index only has 2.0.0 — outside Major pin range for 1.x
    let store_v2 = create_fake_store_path(
        tmp.path(),
        "widget",
        "2.0.0",
        &[("bin/widget-app", "#!/bin/sh\necho widget v2")],
    );
    let merged = build_test_merged_index(&[(
        "widget",
        "2.0.0",
        store_v2.to_str().expect("BUG: valid UTF-8"),
        "braiins",
    )]);

    let plan = compute_upgrade_plan(&manifest, Some(&merged), &[], &[]).expect("BUG: plan failed");

    // Package should be stale — no matching version within pin range
    assert_eq!(plan.stale.len(), 1);
    assert_eq!(plan.stale[0].name, "widget");
    assert_eq!(plan.stale[0].version, "1.0.0");
    assert!(plan.changed.is_empty());

    // Build gen 2 — stale package should be carried over
    let all_packages = merge_installed_with_new(&manifest, &plan.packages);
    let gen2 = bmc_nix::profile::build_profile(&profile_dir, 2, &all_packages, "hooks", None)
        .await
        .expect("BUG: build gen 2 failed");

    // Verify old package files are present in the new profile
    assert!(
        gen2.path.join("bin/widget-app").is_symlink(),
        "stale package binary should exist in gen 2"
    );
    let link_target =
        std::fs::read_link(gen2.path.join("bin/widget-app")).expect("BUG: read symlink");
    assert!(
        link_target
            .to_str()
            .expect("BUG: valid UTF-8")
            .contains("widget-1.0.0"),
        "stale package should still point to v1 store path, got: {link_target:?}"
    );

    // Verify manifest in gen 2 still has widget@1.0.0
    let gen2_manifest = read_manifest(&gen2.path).expect("BUG: read gen2 manifest");
    assert_eq!(
        gen2_manifest
            .packages
            .get("widget")
            .expect("BUG: widget missing")
            .version,
        "1.0.0"
    );
}

/// Verify pin strategy is respected during upgrade plan computation.
#[tokio::test]
async fn upgrade_plan_respects_pin_strategy() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Create generation 1 with pinned package
    let store_v1 = create_fake_store_path(tmp.path(), "core", "1.0.0", &[("bin/core-app", "v1")]);
    create_activation_entrypoint(&store_v1);

    let packages_v1 = vec![ResolvedPackage {
        name: "core".into(),
        version: "1.0.0".into(),
        store_path: store_v1.to_str().expect("BUG: valid UTF-8").into(),
        cache_url: String::new(),
        cache_name: "default".into(),
        category: None,
        description: None,
        upgrade_strategy: None,
        install_strategy: None,
        installed_by: InstalledBy::System,
        installed_from: "braiins".into(),
        pinned: PinStrategy::Major, // Pin to major version
    }];

    let gen1 = bmc_nix::profile::build_profile(&profile_dir, 1, &packages_v1, "hooks", None)
        .await
        .expect("BUG: build gen 1 failed");

    let manifest = read_manifest(&gen1.path).expect("BUG: read manifest");

    // Create store paths for index entries
    let store_v1_1 =
        create_fake_store_path(tmp.path(), "core", "1.1.0", &[("bin/core-app", "v1.1")]);
    let store_v2 = create_fake_store_path(tmp.path(), "core", "2.0.0", &[("bin/core-app", "v2")]);

    // Index has both 1.1.0 and 2.0.0
    let merged = build_test_merged_index(&[
        (
            "core",
            "1.1.0",
            store_v1_1.to_str().expect("BUG: valid UTF-8"),
            "braiins",
        ),
        (
            "core",
            "2.0.0",
            store_v2.to_str().expect("BUG: valid UTF-8"),
            "braiins",
        ),
    ]);

    let plan =
        compute_upgrade_plan(&manifest, Some(&merged), &[], &[]).expect("BUG: upgrade plan failed");

    // With Major pin, should upgrade to 1.1.0 but NOT 2.0.0
    assert_eq!(plan.changed.len(), 1);
    assert_eq!(plan.changed[0].to_version, "1.1.0");
}

/// Verify the strategy summary collects strategies from all packages.
#[test]
fn strategy_summary_integration() {
    let packages = vec![
        ResolvedPackage {
            name: "core".into(),
            version: "1.0.0".into(),
            store_path: "/nix/store/core".into(),
            cache_url: String::new(),
            cache_name: "default".into(),
            category: None,
            description: None,
            upgrade_strategy: Some(UpgradeStrategy::Reboot),
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        },
        ResolvedPackage {
            name: "widget".into(),
            version: "1.0.0".into(),
            store_path: "/nix/store/widget".into(),
            cache_url: String::new(),
            cache_name: "default".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: Some(InstallStrategy::Reboot),
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        },
    ];
    let summary = StrategySummary::from_packages(&packages);
    assert_eq!(summary.upgrade.len(), 1);
    assert_eq!(summary.install.len(), 1);
}

/// Verify merge_installed_with_new handles the complete flow correctly.
#[test]
fn merge_installed_preserves_manifest_fields() {
    let manifest = Manifest {
        packages: BTreeMap::from([(
            "pkg-a".into(),
            ManifestPackage {
                version: "1.0.0".into(),
                cache: "my-cache".into(),
                store_path: "/nix/store/old-a".into(),
                category: Some("core".into()),
                description: Some("Package A".into()),
                upgrade_strategy: Some(UpgradeStrategy::Reboot),
                install_strategy: None,
                installed_by: InstalledBy::System,
                installed_from: "braiins".into(),
                pinned: PinStrategy::Major,
            },
        )]),
    };

    let result = merge_installed_with_new(&manifest, &[]);
    assert_eq!(result.len(), 1);
    let pkg = &result[0];
    assert_eq!(pkg.name, "pkg-a");
    assert_eq!(pkg.version, "1.0.0");
    assert!(
        pkg.cache_url.is_empty(),
        "cache_url should be empty for manifest-derived packages"
    );
    assert_eq!(pkg.cache_name, "my-cache");
    assert_eq!(pkg.category, Some("core".into()));
    assert_eq!(pkg.pinned, PinStrategy::Major);
    assert!(
        matches!(pkg.upgrade_strategy, Some(UpgradeStrategy::Reboot)),
        "upgrade_strategy should be preserved"
    );
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Integration tests for CLI operations: add-packages, remove-packages, reset-profile.
//!
//! These tests replicate the library-level flow performed by the CLI subcommands,
//! using `build_profile` + `activate_profile` directly instead of
//! `apply_profile_change` (which requires a live Nix store for verification).

mod common;

use std::path::Path;

use bmc_nix::manifest::read_manifest;
use bmc_nix::types::*;
use common::{create_activation_entrypoint, create_fake_store, test_resolved_package};
use tempfile::TempDir;

/// Apply the upgrade plan to `profile_dir`: build a new generation, activate
/// it, and return the generation.
///
/// This is the test-friendly equivalent of `apply_profile_change` — it skips
/// store-path verification so tests can use arbitrary filesystem paths.
async fn apply_plan(profile_dir: &Path, plan: &UpgradePlan) -> ProfileGeneration {
    std::fs::create_dir_all(profile_dir).expect("BUG: create profile dir");
    let gen_number =
        bmc_nix::profile::next_generation_number(profile_dir).expect("BUG: next gen number");
    let generation =
        bmc_nix::profile::build_profile(profile_dir, gen_number, &plan.packages, "hooks", None)
            .await
            .expect("BUG: build_profile failed");
    bmc_nix::profile::activate_profile(profile_dir, generation.number, &generation.path, None)
        .await
        .expect("BUG: activate_profile failed");
    generation
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Add packages to a fresh profile (no existing generations).
///
/// Mirrors: `Commands::AddPackages` with an empty profile dir.
#[tokio::test]
async fn add_packages_to_empty_profile() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Fake store path for package "a"
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let add_packages = vec![test_resolved_package(
        "a",
        "1.0.0",
        store_a.to_str().expect("BUG: valid UTF-8"),
    )];

    // Replicate CLI: read current manifest (empty), compute plan, apply
    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read manifest");
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, &add_packages, &[])
        .expect("BUG: plan should succeed");

    let generation = apply_plan(&profile_dir, &plan).await;

    // Generation 1 should exist
    assert_eq!(generation.number, 1);
    assert!(profile_dir.join("1-link").exists(), "1-link should exist");

    // current symlink should point to 1-link
    let current_target =
        std::fs::read_link(profile_dir.join("current")).expect("BUG: read current symlink");
    assert_eq!(current_target.to_str().expect("BUG: valid UTF-8"), "1-link");

    // Manifest should contain package "a"
    let manifest = read_manifest(&generation.path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 1);
    assert!(
        manifest.packages.contains_key("a"),
        "manifest should have 'a'"
    );
    assert_eq!(
        manifest.packages["a"].version, "1.0.0",
        "version should be 1.0.0"
    );

    // Symlink tree should have bin/app-a
    assert!(
        generation.path.join("bin/app-a").is_symlink(),
        "bin/app-a symlink should exist"
    );
}

/// Add a package to an existing profile that already has package "a".
///
/// Mirrors: `Commands::AddPackages` on a profile that was previously built.
#[tokio::test]
async fn add_packages_to_existing_profile() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Gen 1: package "a"
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let packages_gen1 = vec![test_resolved_package(
        "a",
        "1.0.0",
        store_a.to_str().expect("BUG: valid UTF-8"),
    )];
    let plan_gen1 =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Add package "b"
    let store_b = tmp.path().join("store-b-1.0.0");
    create_fake_store(&store_b, &["bin/app-b"]);
    // No activation entrypoint needed for "b" — "a" provides it

    let add_packages = vec![test_resolved_package(
        "b",
        "1.0.0",
        store_b.to_str().expect("BUG: valid UTF-8"),
    )];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read manifest");
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, &add_packages, &[])
        .expect("BUG: plan should succeed");

    let generation = apply_plan(&profile_dir, &plan).await;

    // Should be generation 2
    assert_eq!(generation.number, 2);

    // Manifest should contain both "a" and "b"
    let manifest = read_manifest(&generation.path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 2, "should have 2 packages");
    assert!(manifest.packages.contains_key("a"), "should still have 'a'");
    assert!(manifest.packages.contains_key("b"), "should now have 'b'");

    // Both symlinks should exist
    assert!(
        generation.path.join("bin/app-a").is_symlink(),
        "bin/app-a should exist"
    );
    assert!(
        generation.path.join("bin/app-b").is_symlink(),
        "bin/app-b should exist"
    );
}

/// Adding a package with a name that already exists replaces the old version.
///
/// Mirrors: `Commands::AddPackages` where the package name matches an existing
/// entry — `compute_upgrade_plan` treats this as a replacement.
#[tokio::test]
async fn add_packages_replaces_existing() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Gen 1: package "a" v1.0.0
    let store_a_v1 = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a_v1, &["bin/app-a"]);
    create_activation_entrypoint(&store_a_v1);

    let packages_gen1 = vec![test_resolved_package(
        "a",
        "1.0.0",
        store_a_v1.to_str().expect("BUG: valid UTF-8"),
    )];
    let plan_gen1 =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Now add package "a" v2.0.0 (should replace v1.0.0)
    let store_a_v2 = tmp.path().join("store-a-2.0.0");
    create_fake_store(&store_a_v2, &["bin/app-a"]);
    create_activation_entrypoint(&store_a_v2);

    let add_packages = vec![test_resolved_package(
        "a",
        "2.0.0",
        store_a_v2.to_str().expect("BUG: valid UTF-8"),
    )];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read manifest");
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, &add_packages, &[])
        .expect("BUG: plan should succeed");

    // Plan should report a change, not an add
    assert_eq!(plan.changed.len(), 1, "should report 'a' as changed");
    assert_eq!(plan.changed[0].from_version, "1.0.0");
    assert_eq!(plan.changed[0].to_version, "2.0.0");
    assert!(plan.added.is_empty(), "should not be reported as a new add");

    let generation = apply_plan(&profile_dir, &plan).await;

    // Manifest should have "a" at v2.0.0
    let manifest = read_manifest(&generation.path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 1);
    assert_eq!(
        manifest.packages["a"].version, "2.0.0",
        "version should be updated to 2.0.0"
    );

    // bin/app-a symlink should point into the v2 store path
    let link_target =
        std::fs::read_link(generation.path.join("bin/app-a")).expect("BUG: read symlink");
    assert!(
        link_target
            .to_str()
            .expect("BUG: valid UTF-8")
            .contains("store-a-2.0.0"),
        "bin/app-a should point to v2 store, got: {link_target:?}"
    );
}

/// Remove a package from a profile that has two packages.
///
/// Mirrors: `Commands::RemovePackages`.
#[tokio::test]
async fn remove_packages_from_profile() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Gen 1: packages "a" and "b"
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let store_b = tmp.path().join("store-b-1.0.0");
    create_fake_store(&store_b, &["bin/app-b"]);

    let packages_gen1 = vec![
        test_resolved_package("a", "1.0.0", store_a.to_str().expect("BUG: valid UTF-8")),
        test_resolved_package("b", "1.0.0", store_b.to_str().expect("BUG: valid UTF-8")),
    ];
    let plan_gen1 =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Remove package "b"
    let names_to_remove = vec!["b".into()];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read manifest");
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, &[], &names_to_remove)
        .expect("BUG: plan should succeed");

    let generation = apply_plan(&profile_dir, &plan).await;

    // Should be generation 2
    assert_eq!(generation.number, 2);

    // Manifest should only contain "a"
    let manifest = read_manifest(&generation.path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 1, "should have 1 package");
    assert!(manifest.packages.contains_key("a"), "should still have 'a'");
    assert!(
        !manifest.packages.contains_key("b"),
        "should no longer have 'b'"
    );

    // bin/app-a should still be present, bin/app-b should be gone
    assert!(
        generation.path.join("bin/app-a").is_symlink(),
        "bin/app-a should still exist"
    );
    assert!(
        !generation.path.join("bin/app-b").exists(),
        "bin/app-b should be absent from gen 2"
    );
}

/// Removing a package that does not exist in the profile is rejected by the
/// plan computation.
///
/// Mirrors: `Commands::RemovePackages` with a name not present in the manifest.
#[tokio::test]
async fn remove_nonexistent_package_errors() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Gen 1: only package "a"
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let packages_gen1 = vec![test_resolved_package(
        "a",
        "1.0.0",
        store_a.to_str().expect("BUG: valid UTF-8"),
    )];
    let plan_gen1 =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Try to remove "nonexistent" — should now error.
    let names_to_remove = vec!["nonexistent".into()];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read manifest");
    let err = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, &[], &names_to_remove)
        .expect_err("removing a missing package should error");
    assert!(
        matches!(
            err,
            bmc_nix::manifest::PlanConflict::RemoveNotInstalled(ref name)
                if name == "nonexistent"
        ),
        "got unexpected error: {err:?}"
    );
}

/// Reset ignores the existing manifest and builds from the index packages only.
///
/// Mirrors: `Commands::ResetProfile` — uses an empty manifest so existing
/// packages are not merged in.
#[tokio::test]
async fn reset_profile_ignores_existing_manifest() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Gen 1: packages "a" and "b"
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let store_b = tmp.path().join("store-b-1.0.0");
    create_fake_store(&store_b, &["bin/app-b"]);

    let packages_gen1 = vec![
        test_resolved_package("a", "1.0.0", store_a.to_str().expect("BUG: valid UTF-8")),
        test_resolved_package("b", "1.0.0", store_b.to_str().expect("BUG: valid UTF-8")),
    ];
    let plan_gen1 =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Reset: index only contains package "c" — "a" and "b" must not appear
    let store_c = tmp.path().join("store-c-1.0.0");
    create_fake_store(&store_c, &["bin/app-c"]);
    create_activation_entrypoint(&store_c);

    let index_packages = vec![test_resolved_package(
        "c",
        "1.0.0",
        store_c.to_str().expect("BUG: valid UTF-8"),
    )];

    // Replicate CLI: start from empty manifest (full reset — no merging)
    let plan = bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &index_packages, &[])
        .expect("BUG: plan should succeed");

    let generation = apply_plan(&profile_dir, &plan).await;

    // Should be generation 2
    assert_eq!(generation.number, 2);

    // Manifest should contain only "c"
    let manifest = read_manifest(&generation.path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 1, "should have exactly 1 package");
    assert!(manifest.packages.contains_key("c"), "should have 'c'");
    assert!(
        !manifest.packages.contains_key("a"),
        "'a' should not be in the reset profile"
    );
    assert!(
        !manifest.packages.contains_key("b"),
        "'b' should not be in the reset profile"
    );

    // Old generation should still exist on disk
    assert!(
        profile_dir.join("1-link").exists(),
        "old generation 1-link should still exist on disk"
    );

    // current should now point to gen 2
    let current_target =
        std::fs::read_link(profile_dir.join("current")).expect("BUG: read current symlink");
    assert_eq!(current_target.to_str().expect("BUG: valid UTF-8"), "2-link");
}

/// Running add-packages twice with the same package does not create a new
/// generation: the plan diff is empty, so `apply_profile_change` skips the
/// rebuild entirely.
#[tokio::test]
async fn add_packages_noop_skips_generation() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Seed gen 1 using the test-friendly path.
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let packages = vec![test_resolved_package(
        "a",
        "1.0.0",
        store_a.to_str().expect("BUG: valid UTF-8"),
    )];
    let plan_gen1 = bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &packages, &[])
        .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    assert!(profile_dir.join("1-link").exists(), "gen 1 should exist");
    assert!(
        !profile_dir.join("2-link").exists(),
        "gen 2 should NOT exist yet"
    );

    // Re-apply the identical set through apply_profile_change. This exercises
    // the no-op short-circuit: empty diff, no store verification, no build.
    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        &packages,
        &[],
        false,
        /* activate = */ false,
        "hooks",
        None,
    )
    .await
    .expect("BUG: no-op apply_profile_change should succeed");

    assert!(result.added.is_empty(), "no adds on a no-op");
    assert!(result.removed.is_empty(), "no removes on a no-op");
    assert!(result.changed.is_empty(), "no changes on a no-op");
    assert!(
        !profile_dir.join("2-link").exists(),
        "gen 2 must NOT be created on a no-op"
    );
    let generation = result.generation.expect("BUG: current exists");
    assert_eq!(generation.number, 1, "should point at gen 1");
    assert!(
        generation.path.ends_with("1-link"),
        "should resolve to 1-link, got {:?}",
        generation.path
    );
}

/// `add-packages` with the same name in both add and remove lists is rejected
/// at plan-computation time.
#[tokio::test]
async fn compute_plan_rejects_add_and_remove_same_name() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Seed a profile with package "a".
    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/app-a"]);
    create_activation_entrypoint(&store_a);
    let seed = vec![test_resolved_package(
        "a",
        "1.0.0",
        store_a.to_str().expect("BUG: valid UTF-8"),
    )];
    let plan_gen1 = bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), &seed, &[])
        .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Request "a" be added AND removed — must error.
    let store_a_v2 = tmp.path().join("store-a-2.0.0");
    create_fake_store(&store_a_v2, &["bin/app-a"]);
    let adds = vec![test_resolved_package(
        "a",
        "2.0.0",
        store_a_v2.to_str().expect("BUG: valid UTF-8"),
    )];
    let removes = vec!["a".into()];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read manifest");
    let err = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, &adds, &removes)
        .expect_err("add+remove of same name should error");
    assert!(
        matches!(
            err,
            bmc_nix::manifest::PlanConflict::AddAndRemove(ref name) if name == "a"
        ),
        "got unexpected error: {err:?}"
    );
}

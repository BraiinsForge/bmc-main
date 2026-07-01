// Copyright (C) 2026  Braiins Systems s.r.o.

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
use serial_test::serial;
use tempfile::TempDir;

/// Apply the upgrade plan to `profile_dir`: build a new generation, activate
/// it, and return the generation.
///
/// This is the test-friendly equivalent of `apply_profile_change` — it skips
/// store-path verification so tests can use arbitrary filesystem paths.
async fn apply_plan(profile_dir: &Path, plan: &UpgradePlan) -> ProfileGeneration {
    std::fs::create_dir_all(profile_dir).expect("BUG: create profile dir");
    let gen_number = bmc_nix::profile::max_generation(profile_dir)
        .expect("BUG: scan generations")
        .unwrap_or(0)
        + 1;
    let generation =
        bmc_nix::profile::build_profile(profile_dir, gen_number, &plan.packages, "hooks", None)
            .await
            .expect("BUG: build_profile failed");
    bmc_nix::profile::activate_profile(profile_dir, generation.number, &generation.path, None)
        .await
        .expect("BUG: activate_profile failed");
    generation
}

fn assert_generation_bin_link(generation: &ProfileGeneration, store_path: &Path) {
    let bin = generation.path.join("bin");
    let metadata = bin.symlink_metadata().expect("BUG: stat bin link");
    assert!(
        metadata.file_type().is_symlink(),
        "single-provider bin should be linked at the directory level"
    );
    assert_eq!(
        std::fs::read_link(&bin).expect("BUG: read bin symlink"),
        store_path.join("bin")
    );
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Add packages to a fresh profile (no existing generations).
///
/// Mirrors: `Commands::AddPackages` with an empty profile dir.
#[tokio::test]
#[serial]
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
    let current_manifest = Manifest::default();
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &add_packages, &[])
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

    // Single-provider directories are linked at the highest directory.
    assert_generation_bin_link(&generation, &store_a);
    assert!(
        generation.path.join("bin/app-a").exists(),
        "bin/app-a should resolve through the bin symlink"
    );
}

/// Add a package to an existing profile that already has package "a".
///
/// Mirrors: `Commands::AddPackages` on a profile that was previously built.
#[tokio::test]
#[serial]
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
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &packages_gen1, &[])
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
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read current manifest");
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &add_packages, &[])
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
#[serial]
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
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &packages_gen1, &[])
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
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read current manifest");
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &add_packages, &[])
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

    // The single-provider bin directory should point into the v2 store path.
    assert_generation_bin_link(&generation, &store_a_v2);
    assert!(
        generation.path.join("bin/app-a").exists(),
        "bin/app-a should resolve through the v2 bin symlink"
    );
}

/// Remove a package from a profile that has two packages.
///
/// Mirrors: `Commands::RemovePackages`.
#[tokio::test]
#[serial]
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
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Remove package "b"
    let names_to_remove = vec!["b".into()];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read current manifest");
    let plan =
        bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &[], &names_to_remove)
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

    // bin/app-a should still be present via package a's bin link,
    // while bin/app-b should be gone.
    assert_generation_bin_link(&generation, &store_a);
    assert!(
        generation.path.join("bin/app-a").exists(),
        "bin/app-a should still resolve through the bin symlink"
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
#[serial]
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
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &packages_gen1, &[])
            .expect("BUG: plan should succeed");
    apply_plan(&profile_dir, &plan_gen1).await;

    // Try to remove "nonexistent" — should now error.
    let names_to_remove = vec!["nonexistent".into()];

    let current_manifest =
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read current manifest");
    let err =
        bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &[], &names_to_remove)
            .expect_err("removing a missing package should error");
    assert!(
        matches!(
            err,
            bmc_nix::manifest::ComputeUpgradePlanError::Conflict(
                bmc_nix::manifest::PlanConflict::RemoveNotInstalled(ref name),
            ) if name == "nonexistent"
        ),
        "got unexpected error: {err:?}"
    );
}

/// Reset ignores the existing manifest and builds from the index packages only.
///
/// Mirrors: `Commands::ResetProfile` — uses an empty manifest so existing
/// packages are not merged in.
#[tokio::test]
#[serial]
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
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &packages_gen1, &[])
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
    let plan =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &index_packages, &[])
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
#[serial]
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
    let plan_gen1 =
        bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &packages, &[])
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
        None, // default base: try current, fall back to latest
        None, // no merged index
        &packages,
        &[],
        bmc_nix::upgrade::ActivationMode::Skip,
        None, // no GC
        None, // no progress sink
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
#[serial]
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
    let plan_gen1 = bmc_nix::manifest::compute_upgrade_plan(&Manifest::default(), None, &seed, &[])
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
        bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read current manifest");
    let err = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &adds, &removes)
        .expect_err("add+remove of same name should error");
    assert!(
        matches!(
            err,
            bmc_nix::manifest::ComputeUpgradePlanError::Conflict(
                bmc_nix::manifest::PlanConflict::AddAndRemove(ref name),
            ) if name == "a"
        ),
        "got unexpected error: {err:?}"
    );
}

// ── base selector + activation default tests ────────────────────────────────

/// Build two generations and activate gen 1. With `--base latest` (gen 2),
/// a new gen is built diffed against gen 2's manifest — not gen 1's.
#[tokio::test]
#[serial]
async fn add_packages_with_base_latest_uses_latest_not_current() {
    let tmp = TempDir::new().expect("BUG: create temp dir");
    let profile_dir = tmp.path().join("profiles/bmc");

    // Seed: gen 1 with pkg-one, gen 2 with pkg-two. Activate gen 1.
    let store_one = tmp.path().join("store-one-1.0.0");
    create_fake_store(&store_one, &["bin/one"]);
    create_activation_entrypoint(&store_one);
    let store_two = tmp.path().join("store-two-1.0.0");
    create_fake_store(&store_two, &["bin/two"]);
    // Only the first package in a merged generation provides the
    // activation entrypoint — pkg-one already supplies it for gen 2.

    let pkg_one =
        test_resolved_package("pkg-one", "1.0.0", store_one.to_str().expect("BUG: utf-8"));
    let pkg_two =
        test_resolved_package("pkg-two", "1.0.0", store_two.to_str().expect("BUG: utf-8"));

    let plan1 = bmc_nix::manifest::compute_upgrade_plan(
        &Manifest::default(),
        None,
        std::slice::from_ref(&pkg_one),
        &[],
    )
    .expect("BUG: plan1");
    apply_plan(&profile_dir, &plan1).await;

    let plan2 = bmc_nix::manifest::compute_upgrade_plan(
        &bmc_nix::manifest::read_current_manifest(&profile_dir).expect("BUG: read current"),
        None,
        std::slice::from_ref(&pkg_two),
        &[],
    )
    .expect("BUG: plan2");
    apply_plan(&profile_dir, &plan2).await;
    // Now: gen 1 (pkg-one), gen 2 (pkg-one + pkg-two), `current` -> gen 2.

    // Re-point `current` at gen 1 to create a drift: latest (gen 2) has
    // pkg-two, current (gen 1) does not.
    std::fs::remove_file(profile_dir.join("current")).expect("BUG: rm current");
    std::os::unix::fs::symlink("1-link", profile_dir.join("current"))
        .expect("BUG: symlink -> 1-link");

    // Simulate `add-packages --base latest --name pkg-three`.
    let store_three = tmp.path().join("store-three-1.0.0");
    create_fake_store(&store_three, &["bin/three"]);
    // No entrypoint — the merged base already carries one from pkg-one.
    let pkg_three = test_resolved_package(
        "pkg-three",
        "1.0.0",
        store_three.to_str().expect("BUG: utf-8"),
    );

    let base_manifest = bmc_nix::manifest::read_manifest_by_selector(
        &profile_dir,
        &bmc_nix::types::BaseSelector::Latest,
    )
    .expect("BUG: read latest");
    let plan3 = bmc_nix::manifest::compute_upgrade_plan(&base_manifest, None, &[pkg_three], &[])
        .expect("BUG: plan3");
    let gen3 = apply_plan(&profile_dir, &plan3).await;

    // Gen 3 must contain pkg-one, pkg-two, pkg-three (diffed against gen 2).
    let m = read_manifest(&gen3.path).expect("BUG: read gen3 manifest");
    assert!(m.packages.contains_key("pkg-one"), "gen3 has pkg-one");
    assert!(m.packages.contains_key("pkg-two"), "gen3 has pkg-two");
    assert!(m.packages.contains_key("pkg-three"), "gen3 has pkg-three");
}

/// `--base 1` (explicit N) diffs against that specific generation.
#[tokio::test]
#[serial]
async fn add_packages_with_base_generation_n_uses_specific_generation() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let profile_dir = tmp.path().join("profiles/bmc");

    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/a"]);
    create_activation_entrypoint(&store_a);
    let pkg_a = test_resolved_package("a", "1.0.0", store_a.to_str().expect("BUG: utf-8"));

    // gen 1: pkg-a; gen 2: pkg-noise — distinct gen-2 content lets us
    // assert !contains_key("noise") below, strengthening the
    // "base 1 ≠ latest" invariant.
    let store_noise = tmp.path().join("store-noise-1.0.0");
    create_fake_store(&store_noise, &["bin/noise"]);
    create_activation_entrypoint(&store_noise);
    let pkg_noise =
        test_resolved_package("noise", "1.0.0", store_noise.to_str().expect("BUG: utf-8"));

    let plan1 = bmc_nix::manifest::compute_upgrade_plan(
        &Manifest::default(),
        None,
        std::slice::from_ref(&pkg_a),
        &[],
    )
    .expect("BUG: plan1");
    apply_plan(&profile_dir, &plan1).await;
    let plan2 = bmc_nix::manifest::compute_upgrade_plan(
        &Manifest::default(),
        None,
        std::slice::from_ref(&pkg_noise),
        &[],
    )
    .expect("BUG: plan2");
    apply_plan(&profile_dir, &plan2).await;

    // `add-packages --base 1 --name b` should diff against gen 1 (which has
    // pkg-a) — and must NOT carry pkg-noise from gen 2.
    let store_b = tmp.path().join("store-b-1.0.0");
    create_fake_store(&store_b, &["bin/b"]);
    // No entrypoint — pkg-a (already in the base) supplies it.
    let pkg_b = test_resolved_package("b", "1.0.0", store_b.to_str().expect("BUG: utf-8"));

    let base = bmc_nix::manifest::read_manifest_by_selector(
        &profile_dir,
        &bmc_nix::types::BaseSelector::Generation(1),
    )
    .expect("BUG: read gen 1");
    let plan3 =
        bmc_nix::manifest::compute_upgrade_plan(&base, None, &[pkg_b], &[]).expect("BUG: plan3");
    let gen3 = apply_plan(&profile_dir, &plan3).await;

    let m = read_manifest(&gen3.path).expect("BUG: read gen3 manifest");
    assert!(m.packages.contains_key("a"), "gen3 carries a from gen 1");
    assert!(m.packages.contains_key("b"), "gen3 adds b");
    assert!(
        !m.packages.contains_key("noise"),
        "gen3 does NOT carry pkg-noise from gen 2"
    );
}

/// `--base 42` when generation 42 doesn't exist errors explicitly.
#[tokio::test]
#[serial]
async fn base_with_nonexistent_generation_errors() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let profile_dir = tmp.path().join("profiles/bmc");
    std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

    let err = bmc_nix::manifest::read_manifest_by_selector(
        &profile_dir,
        &bmc_nix::types::BaseSelector::Generation(42),
    )
    .expect_err("missing gen 42 must error");
    assert!(
        matches!(
            err,
            bmc_nix::manifest::ReadManifestError::GenerationNotFound { generation: 42, .. }
        ),
        "expected GenerationNotFound, got {err:?}"
    );
}

/// Default base path with a broken `current` symlink falls back to latest —
/// the new generation must diff against latest, not against an empty base.
#[tokio::test]
#[serial]
async fn add_packages_default_base_falls_back_to_latest_when_current_missing() {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let profile_dir = tmp.path().join("profiles/bmc");

    let store_a = tmp.path().join("store-a-1.0.0");
    create_fake_store(&store_a, &["bin/a"]);
    create_activation_entrypoint(&store_a);
    let pkg_a = test_resolved_package("a", "1.0.0", store_a.to_str().expect("BUG: utf-8"));

    // Seed gen 1 with pkg-a.
    let plan1 = bmc_nix::manifest::compute_upgrade_plan(
        &Manifest::default(),
        None,
        std::slice::from_ref(&pkg_a),
        &[],
    )
    .expect("BUG: plan1");
    apply_plan(&profile_dir, &plan1).await;

    // Break the `current` symlink — gen 1 still exists, but current is gone.
    std::fs::remove_file(profile_dir.join("current")).expect("BUG: rm current");

    // Add pkg-b through the default-base path.
    let store_b = tmp.path().join("store-b-1.0.0");
    create_fake_store(&store_b, &["bin/b"]);
    // No entrypoint — pkg-a (carried from latest) supplies it.
    let pkg_b = test_resolved_package("b", "1.0.0", store_b.to_str().expect("BUG: utf-8"));

    let base =
        bmc_nix::manifest::read_latest_manifest(&profile_dir).expect("BUG: read latest manifest");
    let plan =
        bmc_nix::manifest::compute_upgrade_plan(&base, None, &[pkg_b], &[]).expect("BUG: plan");
    let gen2 = apply_plan(&profile_dir, &plan).await;

    // Gen 2 must carry pkg-a (from latest = gen 1) AND pkg-b.
    let m = read_manifest(&gen2.path).expect("BUG: read gen2 manifest");
    assert!(m.packages.contains_key("a"), "gen2 keeps pkg-a from latest");
    assert!(m.packages.contains_key("b"), "gen2 adds pkg-b");
}

// NOTE: `apply_profile_change_explicit_base_always_builds_new_generation_on_noop`
// was removed — `apply_profile_change` calls `store::verify_store_paths` against
// the live Nix store for explicit-base calls (the short-circuit is gated to the
// `None` path), and the test infrastructure uses ad-hoc temp-dir paths. Covering
// the "explicit base always builds, even on no-op" invariant belongs in a unit
// test in `upgrade.rs` with an injected `CommandRunner`, tracked as follow-up
// under #BDK-363.

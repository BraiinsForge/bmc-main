// Copyright (C) 2026  Braiins Systems s.r.o.

//! End-to-end tests for `bmc-nix-cli add-packages`, `remove-packages`
//! and `reset-profile`.
//!
//! These invoke the actual compiled binary and assert on real exit
//! codes, the stderr diff output, and post-run filesystem state, so
//! they cover CLI dispatch, error mapping, and library orchestration
//! together.
//!
//! `apply_profile_change` shells out to `nix-store` to realize and
//! verify store paths; a stub `nix-store` that always exits 0 is
//! prepended to `PATH` so arbitrary temp-dir store paths pass both
//! phases.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmc_nix::manifest::read_manifest;
use common::{create_activation_entrypoint, create_fake_store};
use serial_test::serial;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_bmc-nix-cli")
}

struct CliRun {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

impl CliRun {
    fn ok(&self, ctx: &str) {
        assert!(
            self.status.success(),
            "{ctx}: expected exit 0, got {:?}. stderr:\n{}",
            self.status.code(),
            self.stderr,
        );
    }

    fn exit_code(&self) -> Option<i32> {
        self.status.code()
    }

    /// The generation path the CLI printed on stdout.
    fn generation_path(&self, ctx: &str) -> PathBuf {
        self.ok(ctx);
        let line = self.stdout.trim();
        assert!(
            !line.is_empty(),
            "{ctx}: expected a generation path on stdout"
        );
        PathBuf::from(line)
    }
}

struct TestEnv {
    tmp: TempDir,
    profile_dir: PathBuf,
    path_env: String,
}

fn setup() -> TestEnv {
    let tmp = TempDir::new().expect("BUG: tempdir");
    let profile_dir = tmp.path().join("profiles/bmc");

    let stub_dir = tmp.path().join("stub-bin");
    std::fs::create_dir_all(&stub_dir).expect("BUG: mk stub-bin");
    let stub = stub_dir.join("nix-store");
    std::fs::write(&stub, "#!/bin/sh\nexit 0\n").expect("BUG: write nix-store stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("BUG: chmod nix-store stub");

    let real_path = std::env::var("PATH").expect("BUG: PATH is set");
    let path_env = format!("{}:{real_path}", stub_dir.display());

    TestEnv {
        tmp,
        profile_dir,
        path_env,
    }
}

impl TestEnv {
    fn run(&self, args: &[&str]) -> CliRun {
        let output = Command::new(bin())
            .args(args)
            .env("PATH", &self.path_env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("BUG: failed to spawn bmc-nix-cli");
        CliRun {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn profile_dir_arg(&self) -> String {
        self.profile_dir.display().to_string()
    }

    /// Create a fake store directory named `name` providing `files`.
    fn make_store(&self, name: &str, files: &[&str]) -> PathBuf {
        let store = self.tmp.path().join(name);
        create_fake_store(&store, files);
        store
    }

    /// Run `add-packages` for the (name, version, store path) triples,
    /// with `extra` CLI flags appended.
    fn add_packages(&self, packages: &[(&str, &str, &Path)], extra: &[&str]) -> CliRun {
        let profile = self.profile_dir_arg();
        let stores: Vec<String> = packages
            .iter()
            .map(|(_, _, store)| store.display().to_string())
            .collect();
        let mut args = vec!["add-packages", "--profile-dir", &profile];
        for ((name, version, _), store) in packages.iter().zip(&stores) {
            args.extend_from_slice(&["--name", name, "--version", version, "--store-path", store]);
        }
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn current_link_target(&self) -> PathBuf {
        std::fs::read_link(self.profile_dir.join("current")).expect("BUG: read current symlink")
    }
}

fn assert_generation_bin_link(generation_path: &Path, store_path: &Path) {
    let bin = generation_path.join("bin");
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

// ── add-packages ─────────────────────────────────────────────────────────────

#[test]
#[serial]
fn add_packages_to_empty_profile() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a);

    let run = env.add_packages(&[("a", "1.0.0", &store_a)], &[]);
    let gen_path = run.generation_path("add to empty profile");

    assert!(gen_path.ends_with("1-link"), "got {gen_path:?}");
    assert!(
        env.profile_dir.join("1-link").exists(),
        "1-link should exist"
    );
    assert_eq!(env.current_link_target(), PathBuf::from("1-link"));
    assert!(
        run.stderr.contains("+ a 1.0.0"),
        "stderr should report the add: {}",
        run.stderr,
    );

    let manifest = read_manifest(&gen_path).expect("BUG: read manifest");
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
    assert_generation_bin_link(&gen_path, &store_a);
    assert!(
        gen_path.join("bin/app-a").exists(),
        "bin/app-a should resolve through the bin symlink"
    );
}

#[test]
#[serial]
fn add_packages_to_existing_profile() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a);
    env.add_packages(&[("a", "1.0.0", &store_a)], &[])
        .ok("seed gen 1");

    // No activation entrypoint needed for "b" — "a" provides it.
    let store_b = env.make_store("store-b-1.0.0", &["bin/app-b"]);
    let run = env.add_packages(&[("b", "1.0.0", &store_b)], &[]);
    let gen_path = run.generation_path("add to existing profile");

    assert!(gen_path.ends_with("2-link"), "got {gen_path:?}");

    let manifest = read_manifest(&gen_path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 2, "should have 2 packages");
    assert!(manifest.packages.contains_key("a"), "should still have 'a'");
    assert!(manifest.packages.contains_key("b"), "should now have 'b'");

    assert!(
        gen_path.join("bin/app-a").is_symlink(),
        "bin/app-a should exist"
    );
    assert!(
        gen_path.join("bin/app-b").is_symlink(),
        "bin/app-b should exist"
    );
}

#[test]
#[serial]
fn add_packages_replaces_existing() {
    let env = setup();
    let store_a_v1 = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a_v1);
    env.add_packages(&[("a", "1.0.0", &store_a_v1)], &[])
        .ok("seed gen 1");

    let store_a_v2 = env.make_store("store-a-2.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a_v2);
    let run = env.add_packages(&[("a", "2.0.0", &store_a_v2)], &[]);
    let gen_path = run.generation_path("replace a with v2");

    // Reported as a change, not an add.
    assert!(
        run.stderr.contains("~ a: 1.0.0 -> 2.0.0"),
        "stderr should report the version change: {}",
        run.stderr,
    );

    let manifest = read_manifest(&gen_path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 1);
    assert_eq!(
        manifest.packages["a"].version, "2.0.0",
        "version should be updated to 2.0.0"
    );

    // The single-provider bin directory should point into the v2 store path.
    assert_generation_bin_link(&gen_path, &store_a_v2);
    assert!(
        gen_path.join("bin/app-a").exists(),
        "bin/app-a should resolve through the v2 bin symlink"
    );
}

#[test]
#[serial]
fn add_packages_mismatched_flag_counts_error() {
    let env = setup();
    let profile = env.profile_dir_arg();
    let run = env.run(&[
        "add-packages",
        "--profile-dir",
        &profile,
        "--name",
        "a",
        "--version",
        "1.0.0",
    ]);
    assert_eq!(run.exit_code(), Some(1));
    assert!(
        run.stderr.contains("same number of times"),
        "stderr should explain the flag-count mismatch: {}",
        run.stderr,
    );
}

/// Running add-packages twice with the same package does not create a
/// new generation: the plan diff is empty, so the rebuild is skipped
/// entirely.
#[test]
#[serial]
fn add_packages_noop_skips_generation() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a);
    env.add_packages(&[("a", "1.0.0", &store_a)], &[])
        .ok("seed gen 1");

    let run = env.add_packages(&[("a", "1.0.0", &store_a)], &[]);
    let gen_path = run.generation_path("no-op re-add");

    assert!(
        run.stderr.contains("Profile unchanged."),
        "stderr should report the no-op: {}",
        run.stderr,
    );
    assert!(
        !env.profile_dir.join("2-link").exists(),
        "gen 2 must NOT be created on a no-op"
    );
    assert!(
        gen_path.ends_with("1-link"),
        "should resolve to 1-link, got {gen_path:?}"
    );
}

// ── remove-packages ──────────────────────────────────────────────────────────

#[test]
#[serial]
fn remove_packages_from_profile() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a);
    let store_b = env.make_store("store-b-1.0.0", &["bin/app-b"]);
    env.add_packages(&[("a", "1.0.0", &store_a), ("b", "1.0.0", &store_b)], &[])
        .ok("seed gen 1 with a and b");

    let profile = env.profile_dir_arg();
    let run = env.run(&["remove-packages", "--profile-dir", &profile, "--name", "b"]);
    let gen_path = run.generation_path("remove b");

    assert!(gen_path.ends_with("2-link"), "got {gen_path:?}");
    assert!(
        run.stderr.contains("- b 1.0.0"),
        "stderr should report the removal: {}",
        run.stderr,
    );

    let manifest = read_manifest(&gen_path).expect("BUG: read manifest");
    assert_eq!(manifest.packages.len(), 1, "should have 1 package");
    assert!(manifest.packages.contains_key("a"), "should still have 'a'");
    assert!(
        !manifest.packages.contains_key("b"),
        "should no longer have 'b'"
    );

    // bin/app-a should still be present via package a's bin link,
    // while bin/app-b should be gone.
    assert_generation_bin_link(&gen_path, &store_a);
    assert!(
        gen_path.join("bin/app-a").exists(),
        "bin/app-a should still resolve through the bin symlink"
    );
    assert!(
        !gen_path.join("bin/app-b").exists(),
        "bin/app-b should be absent from gen 2"
    );
}

#[test]
#[serial]
fn remove_nonexistent_package_errors() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a);
    env.add_packages(&[("a", "1.0.0", &store_a)], &[])
        .ok("seed gen 1");

    let profile = env.profile_dir_arg();
    let run = env.run(&[
        "remove-packages",
        "--profile-dir",
        &profile,
        "--name",
        "nonexistent",
    ]);
    assert_eq!(run.exit_code(), Some(1));
    assert!(
        run.stderr.contains("requested for removal but not present"),
        "stderr should report the conflict: {}",
        run.stderr,
    );
    assert!(
        !env.profile_dir.join("2-link").exists(),
        "no new generation on a rejected plan"
    );
}

// ── reset-profile ────────────────────────────────────────────────────────────

/// Write a `servers.json` with one enabled `file://` mirror pointing at
/// `configured_index`. The mandatory `factory` entry is never fetched by
/// `upgrade`, so it points at an inert path.
fn write_servers_config(env: &TestEnv, name: &str, configured_index: &Path) -> PathBuf {
    let config = format!(
        r#"{{"factory":{{"id":"factory","base_url":"file:///dev/null","known_public_key":"k","priority":0,"enabled":true}},"servers":[{{"id":"braiins","type":"mirror","base_url":"file://{}","known_public_key":"k","priority":10,"enabled":true,"required":true}}]}}"#,
        configured_index.display()
    );
    let path = env.tmp.path().join(name);
    std::fs::write(&path, config).expect("BUG: write servers.json");
    path
}

/// Write a minimal package index JSON listing the given packages.
fn write_index(env: &TestEnv, name: &str, packages: &[(&str, &str, &Path)]) -> PathBuf {
    let entries: Vec<String> = packages
        .iter()
        .map(|(pkg, version, store)| {
            format!(
                r#"{{"name":"{pkg}","version":"{version}","store_path":"{}"}}"#,
                store.display()
            )
        })
        .collect();
    let index = format!(
        r#"{{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{}]}}"#,
        entries.join(",")
    );
    let path = env.tmp.path().join(name);
    std::fs::write(&path, index).expect("BUG: write index");
    path
}

#[test]
#[serial]
fn reset_profile_ignores_existing_manifest() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/app-a"]);
    create_activation_entrypoint(&store_a);
    let store_b = env.make_store("store-b-1.0.0", &["bin/app-b"]);
    env.add_packages(&[("a", "1.0.0", &store_a), ("b", "1.0.0", &store_b)], &[])
        .ok("seed gen 1 with a and b");

    // Reset: index only contains package "c" — "a" and "b" must not appear.
    let store_c = env.make_store("store-c-1.0.0", &["bin/app-c"]);
    create_activation_entrypoint(&store_c);
    let index = write_index(&env, "index.json", &[("c", "1.0.0", &store_c)]);

    let profile = env.profile_dir_arg();
    let index_arg = index.display().to_string();
    let run = env.run(&[
        "reset-profile",
        "--profile-dir",
        &profile,
        "--index",
        &index_arg,
    ]);
    let gen_path = run.generation_path("reset to index with c");

    assert!(gen_path.ends_with("2-link"), "got {gen_path:?}");

    let manifest = read_manifest(&gen_path).expect("BUG: read manifest");
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

    // Old generation should still exist on disk.
    assert!(
        env.profile_dir.join("1-link").exists(),
        "old generation 1-link should still exist on disk"
    );
    assert_eq!(env.current_link_target(), PathBuf::from("2-link"));
}

// ── base selector + activation default tests ────────────────────────────────

/// Build two generations and activate gen 1. With `--base latest`
/// (gen 2), the new generation is diffed against gen 2's manifest —
/// not gen 1's.
#[test]
#[serial]
fn add_packages_with_base_latest_uses_latest_not_current() {
    let env = setup();

    // Seed: gen 1 with pkg-one, gen 2 with pkg-one + pkg-two.
    let store_one = env.make_store("store-one-1.0.0", &["bin/one"]);
    create_activation_entrypoint(&store_one);
    env.add_packages(&[("pkg-one", "1.0.0", &store_one)], &[])
        .ok("seed gen 1");
    // Only the first package in a merged generation provides the
    // activation entrypoint — pkg-one already supplies it for gen 2.
    let store_two = env.make_store("store-two-1.0.0", &["bin/two"]);
    env.add_packages(&[("pkg-two", "1.0.0", &store_two)], &[])
        .ok("seed gen 2");

    // Re-point `current` at gen 1 to create a drift: latest (gen 2) has
    // pkg-two, current (gen 1) does not.
    std::fs::remove_file(env.profile_dir.join("current")).expect("BUG: rm current");
    std::os::unix::fs::symlink("1-link", env.profile_dir.join("current"))
        .expect("BUG: symlink -> 1-link");

    let store_three = env.make_store("store-three-1.0.0", &["bin/three"]);
    let run = env.add_packages(
        &[("pkg-three", "1.0.0", &store_three)],
        &["--base", "latest"],
    );
    let gen_path = run.generation_path("add with --base latest");

    // Gen 3 must contain pkg-one, pkg-two, pkg-three (diffed against gen 2).
    assert!(gen_path.ends_with("3-link"), "got {gen_path:?}");
    let m = read_manifest(&gen_path).expect("BUG: read gen3 manifest");
    assert!(m.packages.contains_key("pkg-one"), "gen3 has pkg-one");
    assert!(m.packages.contains_key("pkg-two"), "gen3 has pkg-two");
    assert!(m.packages.contains_key("pkg-three"), "gen3 has pkg-three");
}

/// `--base 1` (explicit N) diffs against that specific generation.
#[test]
#[serial]
fn add_packages_with_base_generation_n_uses_specific_generation() {
    let env = setup();

    // gen 1: pkg-a. gen 2 (via reset): pkg-noise only — distinct gen-2
    // content lets us assert !contains_key("noise") below,
    // strengthening the "base 1 ≠ latest" invariant.
    let store_a = env.make_store("store-a-1.0.0", &["bin/a"]);
    create_activation_entrypoint(&store_a);
    env.add_packages(&[("a", "1.0.0", &store_a)], &[])
        .ok("seed gen 1");

    let store_noise = env.make_store("store-noise-1.0.0", &["bin/noise"]);
    create_activation_entrypoint(&store_noise);
    let index = write_index(&env, "noise.json", &[("noise", "1.0.0", &store_noise)]);
    let profile = env.profile_dir_arg();
    let index_arg = index.display().to_string();
    env.run(&[
        "reset-profile",
        "--profile-dir",
        &profile,
        "--index",
        &index_arg,
    ])
    .ok("seed gen 2 with noise");

    // `add-packages --base 1 --name b` should diff against gen 1 (which
    // has pkg-a) — and must NOT carry pkg-noise from gen 2.
    let store_b = env.make_store("store-b-1.0.0", &["bin/b"]);
    let run = env.add_packages(&[("b", "1.0.0", &store_b)], &["--base", "1"]);
    let gen_path = run.generation_path("add with --base 1");

    let m = read_manifest(&gen_path).expect("BUG: read gen3 manifest");
    assert!(m.packages.contains_key("a"), "gen3 carries a from gen 1");
    assert!(m.packages.contains_key("b"), "gen3 adds b");
    assert!(
        !m.packages.contains_key("noise"),
        "gen3 does NOT carry pkg-noise from gen 2"
    );
}

/// `--base 42` when generation 42 doesn't exist errors explicitly.
#[test]
#[serial]
fn base_with_nonexistent_generation_errors() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/a"]);
    create_activation_entrypoint(&store_a);

    let run = env.add_packages(&[("a", "1.0.0", &store_a)], &["--base", "42"]);
    assert_eq!(run.exit_code(), Some(1));
    assert!(
        run.stderr.contains("generation 42 not found"),
        "stderr should report the missing generation: {}",
        run.stderr,
    );
}

/// Default base path with a broken `current` symlink falls back to
/// latest — the new generation must diff against latest, not against an
/// empty base.
#[test]
#[serial]
fn add_packages_default_base_falls_back_to_latest_when_current_missing() {
    let env = setup();
    let store_a = env.make_store("store-a-1.0.0", &["bin/a"]);
    create_activation_entrypoint(&store_a);
    env.add_packages(&[("a", "1.0.0", &store_a)], &[])
        .ok("seed gen 1");

    // Break the `current` symlink — gen 1 still exists, but current is gone.
    std::fs::remove_file(env.profile_dir.join("current")).expect("BUG: rm current");

    let store_b = env.make_store("store-b-1.0.0", &["bin/b"]);
    let run = env.add_packages(&[("b", "1.0.0", &store_b)], &[]);
    let gen_path = run.generation_path("add with broken current");

    // Gen 2 must carry pkg-a (from latest = gen 1) AND pkg-b.
    assert!(gen_path.ends_with("2-link"), "got {gen_path:?}");
    let m = read_manifest(&gen_path).expect("BUG: read gen2 manifest");
    assert!(m.packages.contains_key("a"), "gen2 keeps pkg-a from latest");
    assert!(m.packages.contains_key("b"), "gen2 adds pkg-b");
}

// ── upgrade: servers.json + --index precedence and --next-boot staging ──────

/// End-to-end `upgrade` through the real binary: a package present in both
/// the configured `servers.json` mirror and a custom `--index` reference
/// resolves to the `--index` store path (custom entries sit at priority 0,
/// lowest wins the version tie). A follow-up `--next-boot` run stages the
/// new generation via the `next` symlink and leaves `current` in place.
#[test]
#[serial]
fn upgrade_custom_index_wins_precedence_then_next_boot_stages() {
    let env = setup();

    // Seed gen 1: clock 0.9.0 installed locally, so upgrade has a base to
    // diff against.
    let store_base = env.make_store("store-clock-0.9.0", &["bin/clock"]);
    create_activation_entrypoint(&store_base);
    env.add_packages(&[("clock", "0.9.0", &store_base)], &[])
        .ok("seed gen 1 with clock 0.9.0");

    // First upgrade: the configured mirror and the custom --index both list
    // clock 1.0.0, but at *different* store paths. The custom entry must win.
    let store_configured = env.make_store("store-clock-1.0.0-configured", &["bin/clock"]);
    let store_custom = env.make_store("store-clock-1.0.0-custom", &["bin/clock"]);
    create_activation_entrypoint(&store_custom);

    let configured_index = write_index(
        &env,
        "configured.json",
        &[("clock", "1.0.0", &store_configured)],
    );
    let custom_index = write_index(&env, "custom.json", &[("clock", "1.0.0", &store_custom)]);
    let servers = write_servers_config(&env, "servers.json", &configured_index);

    let profile = env.profile_dir_arg();
    let servers_arg = servers.display().to_string();
    let custom_ref = format!("file://{}", custom_index.display());
    let run = env.run(&[
        "upgrade",
        "--servers-config",
        &servers_arg,
        "--index",
        &custom_ref,
        "--profile-dir",
        &profile,
    ]);
    let gen_path = run.generation_path("first upgrade");

    assert!(gen_path.ends_with("2-link"), "got {gen_path:?}");
    assert_eq!(env.current_link_target(), PathBuf::from("2-link"));

    let manifest = read_manifest(&gen_path).expect("BUG: read gen2 manifest");
    assert_eq!(
        manifest.packages["clock"].version, "1.0.0",
        "clock should upgrade to the indexed 1.0.0"
    );
    assert_eq!(
        manifest.packages["clock"].store_path,
        store_custom.display().to_string(),
        "custom --index store path must win the version tie over the configured mirror"
    );

    // Second upgrade with --next-boot: the custom --index bumps clock to
    // 1.1.0, forcing a new generation. It must be staged for the target
    // firmware (next.9.9 -> 3-link), not activated: current stays at
    // 2-link.
    let store_custom_next = env.make_store("store-clock-1.1.0-custom", &["bin/clock"]);
    create_activation_entrypoint(&store_custom_next);
    let custom_index_next = write_index(
        &env,
        "custom-next.json",
        &[("clock", "1.1.0", &store_custom_next)],
    );
    let custom_ref_next = format!("file://{}", custom_index_next.display());

    let run = env.run(&[
        "upgrade",
        "--servers-config",
        &servers_arg,
        "--index",
        &custom_ref_next,
        "--profile-dir",
        &profile,
        "--next-boot",
        "9.9",
    ]);
    let gen_path = run.generation_path("next-boot upgrade");

    assert!(gen_path.ends_with("3-link"), "got {gen_path:?}");
    assert_eq!(
        std::fs::read_link(env.profile_dir.join("next.9.9")).expect("BUG: read next.9.9 symlink"),
        PathBuf::from("3-link"),
        "next-boot must stage gen 3 in the next.9.9 marker"
    );
    assert_eq!(
        env.current_link_target(),
        PathBuf::from("2-link"),
        "next-boot must leave current pointing at gen 2"
    );

    let staged = read_manifest(&gen_path).expect("BUG: read gen3 manifest");
    assert_eq!(
        staged.packages["clock"].version, "1.1.0",
        "staged generation should carry clock 1.1.0"
    );
    assert_eq!(
        staged.packages["clock"].store_path,
        store_custom_next.display().to_string(),
        "staged clock should resolve to the custom 1.1.0 store path"
    );
}

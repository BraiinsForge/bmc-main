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

//! `add-packages --if-missing`: the guarded install a device script runs
//! unattended. It must never replace an installed package, never touch a
//! staged firmware upgrade's marker, and never act for a stale generation.

mod common;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use bmc_nix::manifest::read_manifest;
use common::{create_activation_entrypoint, create_fake_store};
use serial_test::serial;
use tempfile::TempDir;

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
    fn store(&self, name: &str) -> PathBuf {
        let store = self.tmp.path().join(name);
        create_fake_store(&store, &[&format!("bin/{name}")]);
        store
    }

    fn command(&self, packages: &[(&str, &str, &Path)], extra: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bmc-nix-cli"));
        command
            .arg("add-packages")
            .arg("--profile-dir")
            .arg(&self.profile_dir);
        for (name, version, store) in packages {
            command
                .args(["--name", name, "--version", version, "--store-path"])
                .arg(store);
        }
        command
            .args(extra)
            .env("PATH", &self.path_env)
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run(&self, packages: &[(&str, &str, &Path)], extra: &[&str]) -> Output {
        self.command(packages, extra)
            .output()
            .expect("BUG: spawn bmc-nix-cli")
    }

    fn current(&self) -> PathBuf {
        self.profile_dir
            .join(std::fs::read_link(self.profile_dir.join("current")).expect("BUG: read current"))
    }

    fn generations(&self) -> usize {
        std::fs::read_dir(&self.profile_dir)
            .expect("BUG: read profile dir")
            .filter(|entry| {
                entry
                    .as_ref()
                    .expect("BUG: dir entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with("-link")
            })
            .count()
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "expected exit 0, got {:?}; stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Seed generation 1 with `core` 1.0.0, the shape of every real device.
fn seeded() -> (TestEnv, PathBuf) {
    let env = setup();
    let core = env.store("core-1");
    create_activation_entrypoint(&core);
    stdout(&env.run(&[("core", "1.0.0", &core)], &[]));
    (env, core)
}

#[test]
#[serial]
fn missing_widget_is_installed_and_core_is_kept_verbatim() {
    let (env, _) = seeded();
    let before = read_manifest(&env.profile_dir.join("1-link")).expect("BUG: read gen 1");
    let widget = env.store("widget-1");

    let out = stdout(&env.run(&[("widget-x", "1.0.0", &widget)], &["--if-missing"]));

    let generation = PathBuf::from(out.lines().last().expect("BUG: generation path printed"));
    assert!(generation.ends_with("2-link"), "got {out}");
    assert!(
        env.current().ends_with("2-link"),
        "a widget in a generation that never became current is never offered"
    );
    let mut after = read_manifest(&generation).expect("BUG: read manifest");
    let installed = after
        .packages
        .remove("widget-x")
        .expect("BUG: widget-x must be installed");
    assert_eq!(installed.store_path, widget.display().to_string());
    assert_eq!(installed.version, "1.0.0");
    assert_eq!(
        serde_json::to_value(&after.packages).expect("BUG: serialize gen 2"),
        serde_json::to_value(&before.packages).expect("BUG: serialize gen 1"),
        "the install must not touch any field of any other package"
    );
}

#[test]
#[serial]
fn installed_name_at_another_path_and_version_is_not_replaced() {
    let (env, _) = seeded();
    let newer_core = env.store("core-2");

    let out = stdout(&env.run(&[("core", "2.0.0", &newer_core)], &["--if-missing"]));

    assert_eq!(out, "already installed");
    assert_eq!(env.generations(), 1, "no generation may be built");
}

#[test]
#[serial]
fn one_missing_name_does_not_drag_installed_names_along() {
    let (env, _) = seeded();
    let before = read_manifest(&env.profile_dir.join("1-link")).expect("BUG: read gen 1");
    let newer_core = env.store("core-2");
    create_activation_entrypoint(&newer_core);
    let widget = env.store("widget-1");

    let out = stdout(&env.run(
        &[
            ("core", "2.0.0", &newer_core),
            ("widget-x", "1.0.0", &widget),
        ],
        &["--if-missing"],
    ));

    let generation = PathBuf::from(out.lines().last().expect("BUG: generation path printed"));
    let after = read_manifest(&generation).expect("BUG: read manifest");
    assert!(after.packages.contains_key("widget-x"), "got {out}");
    assert_eq!(
        serde_json::to_value(&after.packages["core"]).expect("BUG: serialize gen 2 core"),
        serde_json::to_value(&before.packages["core"]).expect("BUG: serialize gen 1 core"),
        "an installed package listed next to a missing one must stay as it is"
    );
}

#[test]
#[serial]
fn staged_next_boot_marker_blocks_the_install_and_survives() {
    let (env, _) = seeded();
    let marker = env.profile_dir.join("next.26.10");
    std::os::unix::fs::symlink("1-link", &marker).expect("BUG: stage marker");
    let widget = env.store("widget-1");

    let out = stdout(&env.run(&[("widget-x", "1.0.0", &widget)], &["--if-missing"]));

    assert_eq!(out, "next-boot generation staged");
    assert!(
        marker.symlink_metadata().is_ok(),
        "the staged firmware upgrade's marker must survive"
    );
    assert_eq!(env.generations(), 1);
}

#[test]
#[serial]
fn next_entry_that_is_not_a_symlink_does_not_block_the_install() {
    // The sweep and the activators ignore such an entry and never remove
    // it, so counting it as staged would block the install forever.
    let (env, _) = seeded();
    let junk = env.profile_dir.join("next.junk");
    std::fs::create_dir(&junk).expect("BUG: create junk entry");
    let widget = env.store("widget-1");

    let out = stdout(&env.run(&[("widget-x", "1.0.0", &widget)], &["--if-missing"]));

    assert!(out.ends_with("2-link"), "got {out}");
    assert!(junk.is_dir(), "the junk entry is not ours to remove");
}

#[test]
#[serial]
fn stale_generation_changes_nothing() {
    let (env, _) = seeded();
    let widget = env.store("widget-1");
    let stale = env.profile_dir.join("7-link");

    let out = stdout(&env.run(
        &[("widget-x", "1.0.0", &widget)],
        &["--if-missing", "--if-current", &stale.display().to_string()],
    ));

    assert_eq!(out, "profile changed: current is generation 1");
    assert_eq!(env.generations(), 1);
}

#[test]
#[serial]
fn matching_generation_installs() {
    let (env, _) = seeded();
    let widget = env.store("widget-1");
    let current = env.current();

    let out = stdout(&env.run(
        &[("widget-x", "1.0.0", &widget)],
        &[
            "--if-missing",
            "--if-current",
            &current.display().to_string(),
        ],
    ));

    assert!(out.ends_with("2-link"), "got {out}");
}

#[test]
#[serial]
fn malformed_if_current_fails() {
    let (env, _) = seeded();
    let widget = env.store("widget-1");

    let output = env.run(
        &[("widget-x", "1.0.0", &widget)],
        &[
            "--if-missing",
            "--if-current",
            "/nix/var/nix/gcroots/profiles/bmc/current",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(env.generations(), 1);
}

#[test]
#[serial]
fn flags_are_rejected_outside_their_combination() {
    let (env, _) = seeded();
    let widget = env.store("widget-1");
    let current = env.current().display().to_string();
    let widget_pkg: &[(&str, &str, &Path)] = &[("widget-x", "1.0.0", &widget)];

    for extra in [
        &["--if-current", current.as_str()][..],
        &["--if-missing", "--no-activate"][..],
        &["--if-missing", "--base", "latest"][..],
    ] {
        let output = env.run(widget_pkg, extra);
        assert_eq!(output.status.code(), Some(2), "{extra:?} must be rejected");
    }
    assert_eq!(env.generations(), 1);
}

/// Wait until some process blocks on the flock of `lock_file`
/// (`/proc/locks` lists waiters with `->`).
fn wait_for_lock_waiter(lock_file: &Path) {
    let inode = std::fs::metadata(lock_file).expect("BUG: stat lock").ino();
    let needle = format!(":{inode} ");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let locks = std::fs::read_to_string("/proc/locks").expect("BUG: read /proc/locks");
        if locks
            .lines()
            .any(|line| line.contains("->") && line.contains(&needle))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "CLI never blocked on the profile lock"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Run the CLI while this test holds the profile lock, apply `change`
/// once the CLI blocks on it, then release. Checking anything before
/// locking would miss the change.
fn run_with_change_under_lock(
    env: &TestEnv,
    packages: &[(&str, &str, &Path)],
    extra: &[&str],
    change: impl FnOnce(),
) -> Output {
    let lock = bmc_nix::profile::try_lock_profile(&env.profile_dir)
        .expect("BUG: open lock")
        .expect("BUG: lock is free");
    let child = env
        .command(packages, extra)
        .spawn()
        .expect("BUG: spawn bmc-nix-cli");
    wait_for_lock_waiter(&env.profile_dir.join(".lock"));
    change();
    drop(lock);
    child.wait_with_output().expect("BUG: wait for bmc-nix-cli")
}

fn point_current_at(profile_dir: &Path, generation: &str) {
    let tmp = profile_dir.join("current.tmp");
    std::os::unix::fs::symlink(generation, &tmp).expect("BUG: create current.tmp");
    std::fs::rename(&tmp, profile_dir.join("current")).expect("BUG: swap current");
}

#[test]
#[serial]
fn marker_is_read_under_the_lock() {
    let (env, _) = seeded();
    let widget = env.store("widget-1");

    let output = run_with_change_under_lock(
        &env,
        &[("widget-x", "1.0.0", &widget)],
        &["--if-missing"],
        || {
            std::os::unix::fs::symlink("1-link", env.profile_dir.join("next.26.10"))
                .expect("BUG: stage marker");
        },
    );

    assert_eq!(stdout(&output), "next-boot generation staged");
}

#[test]
#[serial]
fn current_is_read_under_the_lock() {
    // A worker of an older core must not install into the generation a
    // newer core activated while the worker waited.
    let (env, _) = seeded();
    let widget = env.store("widget-1");
    let pinned = env.current().display().to_string();
    std::os::unix::fs::symlink("1-link", env.profile_dir.join("2-link"))
        .expect("BUG: fabricate generation 2");

    let output = run_with_change_under_lock(
        &env,
        &[("widget-x", "1.0.0", &widget)],
        &["--if-missing", "--if-current", &pinned],
        || point_current_at(&env.profile_dir, "2-link"),
    );

    assert_eq!(stdout(&output), "profile changed: current is generation 2");
}

#[test]
#[serial]
fn installed_names_are_read_under_the_lock() {
    // A widget installed while the CLI waited must not be replaced.
    let (env, _) = seeded();
    let installed = env.store("widget-installed");
    stdout(&env.run(&[("widget-x", "1.0.0", &installed)], &["--no-activate"]));
    let other = env.store("widget-other");

    let output = run_with_change_under_lock(
        &env,
        &[("widget-x", "2.0.0", &other)],
        &["--if-missing"],
        || point_current_at(&env.profile_dir, "2-link"),
    );

    assert_eq!(stdout(&output), "already installed");
    assert_eq!(env.generations(), 2, "no generation may be built");
}

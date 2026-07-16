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

//! End-to-end tests for `bmc-nix-cli activate` and `bmc-nix-cli mount`.
//!
//! These invoke the actual compiled binary and assert on real exit codes,
//! stderr, and post-run filesystem state, so they cover the CLI dispatch,
//! error mapping, and library orchestration together — instead of just
//! restating clap's declarative parse.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serial_test::serial;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_bmc-nix-cli")
}

struct CliRun {
    status: std::process::ExitStatus,
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
}

fn run_cli(args: &[&str]) -> CliRun {
    let output = Command::new(bin())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("BUG: failed to spawn bmc-nix-cli");
    CliRun {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Create a generation `<n>-link` directory with an `entrypoint` that
/// writes its generation number into `$profile_dir/activation.log` and
/// exits with `exit_code`.
fn make_gen(profile_dir: &Path, n: usize, exit_code: i32) -> PathBuf {
    let gen_path = profile_dir.join(format!("{n}-link"));
    let entrypoint_dir = gen_path.join("core/activation");
    std::fs::create_dir_all(&entrypoint_dir).expect("BUG: mk core/activation");
    let entrypoint = entrypoint_dir.join("entrypoint");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' {n} >> \"$(dirname \"$PROFILE_NEW_GENERATION\")/activation.log\"\nexit {exit_code}\n"
    );
    std::fs::write(&entrypoint, script).expect("BUG: write entrypoint");
    std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
        .expect("BUG: chmod entrypoint");
    gen_path
}

fn make_gen_no_entrypoint(profile_dir: &Path, n: usize) -> PathBuf {
    let gen_path = profile_dir.join(format!("{n}-link"));
    std::fs::create_dir_all(&gen_path).expect("BUG: mk gen dir");
    gen_path
}

fn read_activation_log(profile_dir: &Path) -> Vec<usize> {
    match std::fs::read_to_string(profile_dir.join("activation.log")) {
        Ok(s) => s.lines().filter_map(|l| l.trim().parse().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn tmp_profile() -> TempDir {
    TempDir::new().expect("BUG: tempdir")
}

fn quarantined_marker(profile_dir: &Path, marker: &str) -> PathBuf {
    let prefix = format!("{marker}.invalid.");
    let matches = std::fs::read_dir(profile_dir)
        .expect("BUG: read profile directory")
        .map(|entry| entry.expect("BUG: read profile entry"))
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "exactly one quarantine must preserve {marker}"
    );
    let quarantine = matches
        .first()
        .expect("BUG: exactly one quarantine was asserted");
    let name = quarantine.file_name();
    let suffix = name
        .to_str()
        .expect("BUG: quarantine name must be UTF-8")
        .strip_prefix(&prefix)
        .expect("BUG: quarantine name must retain marker prefix");
    let timestamp = suffix
        .split('.')
        .next()
        .expect("BUG: quarantine name must contain a timestamp");
    assert!(
        timestamp.parse::<u128>().is_ok_and(|value| value > 0),
        "quarantine name must contain a positive numeric timestamp: {}",
        name.display()
    );
    quarantine.path()
}

// ---------------------------------------------------------------------------
// activate --generation current (default)

#[test]
#[serial]
fn activate_current_soft_skip_when_nothing_exists() {
    let dir = tmp_profile();
    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);
    run.ok("activate on empty profile dir");
    assert!(
        run.stderr.contains("skipping activation"),
        "stderr should mention skip: {}",
        run.stderr,
    );
    assert!(read_activation_log(dir.path()).is_empty());
}

#[test]
#[serial]
fn activate_current_uses_current_symlink() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 3, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);
    run.ok("activate current");
    assert_eq!(read_activation_log(dir.path()), vec![1]);
}

#[test]
#[serial]
fn activate_current_falls_back_to_latest_when_symlink_missing() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 4, 0);

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);
    run.ok("activate current with fallback");
    assert_eq!(read_activation_log(dir.path()), vec![4]);
}

#[test]
#[serial]
fn activate_current_quarantines_regular_file_and_activates_latest() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 4, 0);
    std::fs::write(dir.path().join("current"), b"invalid current")
        .expect("BUG: write invalid current");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);

    run.ok("activate current with a regular-file obstruction");
    assert_eq!(read_activation_log(dir.path()), vec![4]);
    assert_eq!(
        std::fs::read(quarantined_marker(dir.path(), "current"))
            .expect("BUG: read quarantined current"),
        b"invalid current"
    );
}

#[test]
#[serial]
fn activate_current_quarantines_directory_and_activates_latest() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 4, 0);
    std::fs::create_dir(dir.path().join("current")).expect("BUG: create invalid current");
    std::fs::write(dir.path().join("current/evidence"), b"preserved")
        .expect("BUG: write invalid current evidence");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);

    run.ok("activate current with a directory obstruction");
    assert_eq!(read_activation_log(dir.path()), vec![4]);
    assert_eq!(
        std::fs::read(quarantined_marker(dir.path(), "current").join("evidence"))
            .expect("BUG: read quarantined current evidence"),
        b"preserved"
    );
}

#[test]
#[serial]
fn activate_current_entrypoint_nonzero_is_hard_error() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 42);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);
    assert_eq!(run.exit_code(), Some(2));
    // current stays put
    assert!(dir.path().join("current").symlink_metadata().is_ok());
}

#[test]
#[serial]
fn activate_current_missing_entrypoint_falls_back_to_latest() {
    let dir = tmp_profile();
    make_gen_no_entrypoint(dir.path(), 1);
    make_gen(dir.path(), 2, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
    ]);
    run.ok("activate current, fallback via missing entrypoint");
    assert_eq!(read_activation_log(dir.path()), vec![2]);
}

// ---------------------------------------------------------------------------
// activate --generation latest / <N>

#[test]
#[serial]
fn activate_latest_picks_highest_valid_gen() {
    let dir = tmp_profile();
    make_gen(dir.path(), 2, 0);
    make_gen(dir.path(), 7, 0);
    make_gen(dir.path(), 5, 0);

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "latest",
    ]);
    run.ok("activate latest");
    assert_eq!(read_activation_log(dir.path()), vec![7]);
}

#[test]
#[serial]
fn activate_latest_missing_is_hard_error() {
    let dir = tmp_profile();
    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "latest",
    ]);
    assert_eq!(run.exit_code(), Some(2));
    assert!(
        run.stderr.contains("no profile generation"),
        "stderr should mention no-generation: {}",
        run.stderr,
    );
}

#[test]
#[serial]
fn activate_generation_number_picks_that_gen() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 4, 0);

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "1",
    ]);
    run.ok("activate specific");
    assert_eq!(read_activation_log(dir.path()), vec![1]);
}

#[test]
#[serial]
fn activate_generation_number_missing_is_hard_error() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "99",
    ]);
    assert_eq!(run.exit_code(), Some(2));
}

// ---------------------------------------------------------------------------
// activate --generation next

#[test]
#[serial]
fn activate_next_no_next_delegates_to_current() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "1.0\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);
    run.ok("activate --generation next with no next");
    assert_eq!(read_activation_log(dir.path()), vec![1]);
}

#[test]
#[serial]
fn activate_next_quarantines_regular_file_and_activates_current() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");
    std::fs::write(dir.path().join("next.1.0"), b"invalid next").expect("BUG: write invalid next");
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "1.0\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);

    run.ok("activate next with a regular-file obstruction");
    assert_eq!(read_activation_log(dir.path()), vec![1]);
    assert_eq!(
        std::fs::read(quarantined_marker(dir.path(), "next.1.0"))
            .expect("BUG: read quarantined next"),
        b"invalid next"
    );
}

#[test]
#[serial]
fn activate_next_quarantines_directory_and_activates_current() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");
    std::fs::create_dir(dir.path().join("next.1.0")).expect("BUG: create invalid next");
    std::fs::write(dir.path().join("next.1.0/evidence"), b"preserved")
        .expect("BUG: write invalid next evidence");
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "1.0\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);

    run.ok("activate next with a directory obstruction");
    assert_eq!(read_activation_log(dir.path()), vec![1]);
    assert_eq!(
        std::fs::read(quarantined_marker(dir.path(), "next.1.0").join("evidence"))
            .expect("BUG: read quarantined next evidence"),
        b"preserved"
    );
}

#[test]
#[serial]
fn activate_next_empty_bos_version_activates_current_and_keeps_markers() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 2, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");
    symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next.1.0");
    symlink("2-link", dir.path().join("next")).expect("BUG: bare next");
    // An empty version file means the version is unknown: staleness is
    // undecidable, so no marker may be consumed or swept.
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);
    run.ok("activate --generation next with empty bos version");
    assert_eq!(read_activation_log(dir.path()), vec![1]);
    assert!(dir.path().join("next.1.0").symlink_metadata().is_ok());
    assert!(dir.path().join("next").symlink_metadata().is_ok());
}

#[test]
#[serial]
fn activate_next_success_removes_next() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 2, 0);
    symlink("1-link", dir.path().join("current")).expect("BUG: current");
    symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next.1.0");
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "1.0\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);
    run.ok("activate --generation next success");
    assert_eq!(read_activation_log(dir.path()), vec![2]);
    assert!(
        dir.path().join("next.1.0").symlink_metadata().is_err(),
        "next.1.0 should be removed on success",
    );
    assert!(
        dir.path().join("previous").symlink_metadata().is_err(),
        "bmc-nix must never write a previous symlink",
    );
}

#[test]
#[serial]
fn activate_next_failure_reverts_and_exits_nonzero() {
    let dir = tmp_profile();
    make_gen(dir.path(), 1, 0);
    make_gen(dir.path(), 2, 42); // next fails
    symlink("1-link", dir.path().join("current")).expect("BUG: current");
    symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next.1.0");
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "1.0\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);
    assert_eq!(
        run.exit_code(),
        Some(2),
        "a reverted activation is still a failed activation"
    );
    // Gen 2's entrypoint ran (logged 2) then exited 42; the revert then
    // ran gen 1 (logged 1). Order matters — proves the sequence.
    assert_eq!(read_activation_log(dir.path()), vec![2, 1]);
    assert!(
        dir.path().join("next.1.0").symlink_metadata().is_ok(),
        "failed next.1.0 should stay put for inspection",
    );
    assert!(
        dir.path().join("previous").symlink_metadata().is_err(),
        "bmc-nix must never write a previous symlink",
    );
}

#[test]
#[serial]
fn activate_next_failure_no_previous_propagates_error() {
    let dir = tmp_profile();
    make_gen(dir.path(), 2, 42);
    symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next.1.0");
    let bos_version_file = dir.path().join("bos_version");
    std::fs::write(&bos_version_file, "1.0\n").expect("BUG: write bos version");

    let run = run_cli(&[
        "activate",
        "--profile-dir",
        &dir.path().display().to_string(),
        "--generation",
        "next",
        "--bos-version-file",
        &bos_version_file.display().to_string(),
    ]);
    assert_eq!(run.exit_code(), Some(2));
    assert!(
        dir.path().join("previous").symlink_metadata().is_err(),
        "no previous should have been staged (no current existed)",
    );
}

// ---------------------------------------------------------------------------
// mount

#[test]
fn mount_source_missing_exits_1() {
    let dir = tmp_profile();
    let source = dir.path().join("does-not-exist");
    let target = dir.path().join("target");
    let run = run_cli(&[
        "mount",
        "--source",
        &source.display().to_string(),
        "--target",
        &target.display().to_string(),
    ]);
    assert_eq!(run.exit_code(), Some(1));
    assert!(
        run.stderr.contains("does not exist"),
        "stderr should mention missing source: {}",
        run.stderr,
    );
}

#[test]
fn mount_target_is_regular_file_exits_2() {
    let dir = tmp_profile();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).expect("BUG: mk source");
    let target = dir.path().join("target-file");
    std::fs::write(&target, b"not a dir").expect("BUG: write target file");

    let run = run_cli(&[
        "mount",
        "--source",
        &source.display().to_string(),
        "--target",
        &target.display().to_string(),
    ]);
    assert_eq!(run.exit_code(), Some(2));
    assert!(
        run.stderr.contains("not a directory"),
        "stderr should mention non-directory target: {}",
        run.stderr,
    );
}

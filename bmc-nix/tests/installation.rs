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

use bmc_nix::installation::{InstallationStatus, inspect_installation};

fn populate_installation(root: &Path) {
    std::fs::create_dir_all(root.join("store/package")).expect("BUG: create store path");

    let database = root.join("var/nix/db/db.sqlite");
    std::fs::create_dir_all(database.parent().expect("BUG: database path has a parent"))
        .expect("BUG: create database directory");
    std::fs::write(database, "").expect("BUG: create database");

    std::fs::create_dir_all(root.join("var/nix/gcroots/profiles/bmc"))
        .expect("BUG: create BMC profile directory");
}

#[test]
fn healthy_installation_accepts_an_alias_with_the_same_identity() {
    let tmp = tempfile::tempdir().expect("BUG: create tempdir");
    let store_root = tmp.path().join("nix");
    populate_installation(&store_root);
    let mount_root = tmp.path().join("mount");
    std::os::unix::fs::symlink(&store_root, &mount_root).expect("BUG: create mount alias");

    assert_eq!(
        inspect_installation(&store_root, &mount_root).expect("BUG: inspection must succeed"),
        InstallationStatus::Ready
    );
}

#[test]
fn absent_store_root_is_distinct_from_an_incomplete_installation() {
    let tmp = tempfile::tempdir().expect("BUG: create tempdir");

    assert_eq!(
        inspect_installation(&tmp.path().join("missing"), &tmp.path().join("mount"))
            .expect("BUG: absence must be inspectable"),
        InstallationStatus::Absent
    );
}

#[test]
fn missing_or_empty_store_is_incomplete() {
    let tmp = tempfile::tempdir().expect("BUG: create tempdir");
    let store_root = tmp.path().join("nix");
    std::fs::create_dir(&store_root).expect("BUG: create store root");

    assert_eq!(
        inspect_installation(&store_root, &store_root).expect("BUG: inspection must succeed"),
        InstallationStatus::Incomplete
    );

    std::fs::create_dir(store_root.join("store")).expect("BUG: create empty store");
    assert_eq!(
        inspect_installation(&store_root, &store_root).expect("BUG: inspection must succeed"),
        InstallationStatus::Incomplete
    );
}

#[test]
fn missing_database_or_profile_is_incomplete() {
    let tmp = tempfile::tempdir().expect("BUG: create tempdir");
    let store_root = tmp.path().join("nix");
    std::fs::create_dir_all(store_root.join("store/package")).expect("BUG: create store path");

    assert_eq!(
        inspect_installation(&store_root, &store_root).expect("BUG: inspection must succeed"),
        InstallationStatus::Incomplete
    );

    let database = store_root.join("var/nix/db/db.sqlite");
    std::fs::create_dir_all(database.parent().expect("BUG: database path has a parent"))
        .expect("BUG: create database directory");
    std::fs::write(database, "").expect("BUG: create database");
    assert_eq!(
        inspect_installation(&store_root, &store_root).expect("BUG: inspection must succeed"),
        InstallationStatus::Incomplete
    );
}

#[test]
fn missing_or_different_mount_identity_is_not_mounted() {
    let tmp = tempfile::tempdir().expect("BUG: create tempdir");
    let store_root = tmp.path().join("nix");
    populate_installation(&store_root);

    assert_eq!(
        inspect_installation(&store_root, &tmp.path().join("missing"))
            .expect("BUG: inspection must succeed"),
        InstallationStatus::NotMounted
    );

    let other = tmp.path().join("other");
    std::fs::create_dir(&other).expect("BUG: create different mount target");
    assert_eq!(
        inspect_installation(&store_root, &other).expect("BUG: inspection must succeed"),
        InstallationStatus::NotMounted
    );
}

#[test]
fn operational_metadata_error_is_returned() {
    let tmp = tempfile::tempdir().expect("BUG: create tempdir");
    let loop_path = tmp.path().join("loop");
    std::os::unix::fs::symlink(PathBuf::from("loop"), &loop_path)
        .expect("BUG: create symlink loop");

    let error = inspect_installation(&loop_path, &loop_path)
        .expect_err("a symlink loop must be an operational error");
    assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
}

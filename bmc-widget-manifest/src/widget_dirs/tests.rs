// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::*;

fn widget(root: &Path, relative: &str) -> PathBuf {
    let dir = root.join(relative);
    std::fs::create_dir_all(&dir).expect("BUG: create widget dir");
    std::fs::write(dir.join("manifest.json"), "{}").expect("BUG: write manifest");
    dir
}

fn found(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = widget_dirs(root)
        .into_iter()
        .map(|dir| dir.expect("BUG: the test tree walks cleanly"))
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn widgets_are_found_as_deep_as_the_depth_cap_and_no_deeper() {
    let root = tempfile::tempdir().expect("BUG: tempdir");
    let flat = widget(root.path(), "clock");
    let grouped = widget(root.path(), "acme/clock");
    let deepest = widget(root.path(), "a/b/c");
    widget(root.path(), "a/b/c2/d");

    assert_eq!(
        found(root.path()),
        vec![deepest, grouped, flat],
        "a widget {MAX_WIDGET_DIR_DEPTH} folders down shows, one deeper does not"
    );
}

#[test]
fn the_walk_stops_at_the_first_folder_holding_a_manifest() {
    let root = tempfile::tempdir().expect("BUG: tempdir");
    let outer = widget(root.path(), "acme");
    widget(root.path(), "acme/clock");

    assert_eq!(
        found(root.path()),
        vec![outer],
        "a widget folder hides any widget below it"
    );
}

#[test]
fn a_linked_widget_folder_is_found_at_the_link() {
    let root = tempfile::tempdir().expect("BUG: tempdir");
    let elsewhere = tempfile::tempdir().expect("BUG: tempdir");
    let target = widget(elsewhere.path(), "clock");
    symlink(&target, root.path().join("clock")).expect("BUG: link widget");

    assert_eq!(
        found(root.path()),
        vec![root.path().join("clock")],
        "a profile links each widget in from the store"
    );
}

#[test]
fn a_missing_root_is_an_error_not_an_empty_listing() {
    let root = tempfile::tempdir().expect("BUG: tempdir");

    let dirs = widget_dirs(&root.path().join("missing"));

    assert!(
        matches!(dirs.as_slice(), [Err(_)]),
        "an unreadable tree must not pass for one without widgets: {dirs:?}"
    );
}

#[test]
fn a_link_loop_is_yielded_in_place_beside_the_widgets() {
    let root = tempfile::tempdir().expect("BUG: tempdir");
    let flat = widget(root.path(), "clock");
    std::fs::create_dir(root.path().join("acme")).expect("BUG: create maintainer dir");
    symlink(root.path(), root.path().join("acme/loop")).expect("BUG: link loop");

    let dirs = widget_dirs(root.path());

    assert!(
        dirs.iter()
            .any(|dir| dir.as_ref().is_err_and(|err| err.loop_ancestor().is_some())),
        "the loop is reported: {dirs:?}"
    );
    assert!(
        dirs.iter()
            .any(|dir| dir.as_ref().is_ok_and(|dir| *dir == flat)),
        "the loop does not hide the widgets beside it: {dirs:?}"
    );
}

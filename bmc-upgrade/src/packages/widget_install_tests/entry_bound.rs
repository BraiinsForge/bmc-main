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

//! The bound on how many files, folders and symlinks an upload may hold,
//! and the room they take once unpacked.

use std::path::Path;

use bmc_nix::store::required_with_headroom;

use super::Rig;
use crate::packages::{SavedUpload, WidgetInstallError};
use crate::widget_package::fixtures::{Node, dir, encode, file, package_tree, set, widget_dir};
use crate::widget_package::{MAX_ARCHIVE_ENTRIES, WidgetPackageError};

fn entries(node: &Node) -> usize {
    match node {
        Node::Directory(children) => 1 + children.values().map(entries).sum::<usize>(),
        Node::Regular { .. } | Node::Symlink { .. } => 1,
    }
}

/// The example widget with assets added until the archive holds `total`
/// entries, so only the count can tell two of them apart.
fn widget_with_entries(total: usize) -> Node {
    let mut widget = widget_dir("example", "1.2.3");
    let fixed = entries(&package_tree("example", widget.clone()));
    let padding_files = total - fixed - 1;
    let padding = Node::Directory(
        (0..padding_files)
            .map(|i| (format!("{i:04}.svg"), file(b"<svg/>")))
            .collect(),
    );
    set(&mut widget, "assets/padding", padding);
    let tree = package_tree("example", widget);
    assert_eq!(entries(&tree), total, "BUG: padding miscounted");
    tree
}

/// A flat folder of `files` one-byte files.
fn flat_tree(files: usize) -> std::collections::BTreeMap<String, Node> {
    (0..files)
        .map(|i| (format!("{i:04}"), file(b"x")))
        .collect()
}

async fn unpack(staging: &Path, tree: &Node) -> Result<std::path::PathBuf, WidgetInstallError> {
    SavedUpload::save(staging, Box::pin(std::io::Cursor::new(encode(tree))))
        .await?
        .unpack()
        .await
}

#[tokio::test]
async fn a_widget_at_the_entry_bound_installs() {
    let rig = Rig::new();

    rig.install(encode(&widget_with_entries(MAX_ARCHIVE_ENTRIES)))
        .await
        .expect("BUG: an archive at the bound is within it");

    assert_eq!(
        rig.installed_version("widget-example").as_deref(),
        Some("1.2.3")
    );
}

#[tokio::test]
async fn a_widget_one_entry_over_the_bound_is_refused_and_changes_nothing() {
    let rig = Rig::new();

    let error = rig
        .install(encode(&widget_with_entries(MAX_ARCHIVE_ENTRIES + 1)))
        .await
        .expect_err("BUG: one entry over the bound is refused");

    assert!(
        matches!(
            error,
            WidgetInstallError::Rejected(WidgetPackageError::TooManyArchiveEntries)
        ),
        "{error}"
    );
    assert_eq!(rig.installed_version("widget-example"), None);
    assert_eq!(rig.store_entries(), ["core"]);
    let staging: Vec<_> = std::fs::read_dir(rig.dir.path().join("staging"))
        .expect("BUG: staging dir")
        .collect();
    assert!(staging.is_empty(), "the staging dir is cleaned up");
}

#[tokio::test]
async fn an_archive_over_the_bound_is_refused_before_any_entry_is_unpacked() {
    let staging = tempfile::tempdir().expect("BUG: tempdir");
    let over = Node::Directory(flat_tree(MAX_ARCHIVE_ENTRIES));

    let error = unpack(staging.path(), &over)
        .await
        .expect_err("BUG: the folder and its files are one entry over");

    assert!(
        matches!(
            error,
            WidgetInstallError::Rejected(WidgetPackageError::TooManyArchiveEntries)
        ),
        "{error}"
    );
    assert!(
        !staging.path().join("package").exists(),
        "a refused archive must not be unpacked even in part"
    );
    assert!(!staging.path().join("upload.nar").exists());
}

#[tokio::test]
async fn folders_and_symlinks_count_toward_the_bound() {
    let at_bound = flat_tree(MAX_ARCHIVE_ENTRIES - 1);
    let staging = tempfile::tempdir().expect("BUG: tempdir");
    let unpacked = unpack(staging.path(), &Node::Directory(at_bound.clone()))
        .await
        .expect("BUG: the folder and its files make exactly the bound");
    assert!(unpacked.join("0000").is_file());
    assert!(
        !staging.path().join("upload.nar").exists(),
        "the saved archive is gone before the tree reaches the store"
    );

    for extra in [
        dir(Vec::new()),
        Node::Symlink {
            target: b"0000".to_vec(),
        },
    ] {
        let mut over = at_bound.clone();
        over.insert("extra".to_owned(), extra);
        let staging = tempfile::tempdir().expect("BUG: tempdir");

        let error = unpack(staging.path(), &Node::Directory(over))
            .await
            .expect_err("BUG: the extra entry is one over the bound");

        assert!(
            matches!(
                error,
                WidgetInstallError::Rejected(WidgetPackageError::TooManyArchiveEntries)
            ),
            "{error}"
        );
    }
}

#[tokio::test]
async fn a_tree_the_device_cannot_unpack_is_its_own_fault() {
    let staging = tempfile::tempdir().expect("BUG: tempdir");
    std::fs::create_dir(staging.path().join("package")).expect("BUG: create package");

    let error = unpack(staging.path(), &Node::Directory(flat_tree(1)))
        .await
        .expect_err("BUG: the unpack target is taken");

    assert!(matches!(error, WidgetInstallError::Staging(_)), "{error}");
}

#[tokio::test]
async fn a_flood_of_tiny_files_is_measured_by_the_blocks_it_unpacks_to() {
    let nar = encode(&widget_with_entries(MAX_ARCHIVE_ENTRIES));
    let room_for_the_archive = required_with_headroom(2 * nar.len() as u64);
    let rig = Rig::with_free_bytes(room_for_the_archive);

    let error = rig
        .install(nar)
        .await
        .expect_err("BUG: 4096 blocks outweigh a 4096-entry archive many times over");

    assert!(
        matches!(
            error,
            WidgetInstallError::Install(bmc_nix::upgrade::InstallError::NotEnoughSpace { .. })
        ),
        "{error}"
    );
    assert_eq!(rig.installed_version("widget-example"), None);
    assert_eq!(rig.store_entries(), ["core"]);
}

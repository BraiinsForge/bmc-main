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

//! An upload whose manifest uid an installed widget in another folder holds.

use std::collections::BTreeMap;
use std::path::Path;

use super::Rig;
use crate::packages::{WidgetInstallError, WidgetInstallOutcome};
use crate::widget_package::fixtures::{
    Node, UID, encode, file, maintained_package_tree, manifest_json, package_tree, set, valid_nar,
    widget_dir,
};

const OTHER_UID: &str = "6ba7b810-9dad-41d1-80b4-00c04fd430c8";

fn widget_with_uid(name: &str, version: &str, uid: &str) -> BTreeMap<String, Node> {
    let mut dir = widget_dir(name, version);
    let manifest = manifest_json(name, version).replace(UID, uid);
    set(&mut dir, "manifest.json", file(manifest.as_bytes()));
    dir
}

#[tokio::test]
async fn a_widget_carrying_an_installed_widgets_uid_is_refused() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.0.0"))
        .await
        .expect("BUG: first install");
    let stored = rig.store_entries();

    let error = rig
        .install(valid_nar("clock", "1.0.0"))
        .await
        .expect_err("BUG: the registry would show only one widget per uid");

    assert!(
        matches!(
            &error,
            WidgetInstallError::UidTaken { uploaded, installed }
                if uploaded == Path::new("clock") && installed == Path::new("example")
        ),
        "{error}"
    );
    assert!(
        error.to_string().contains("installed widget example"),
        "the uploader learns which widget to remove: {error}"
    );
    assert_eq!(rig.installed_version("widget-clock"), None);
    assert_eq!(
        rig.store_entries(),
        stored,
        "a refused upload is not imported"
    );
}

#[tokio::test]
async fn a_uid_held_under_a_maintainer_folder_names_that_folder() {
    let rig = Rig::new();
    rig.install(encode(&maintained_package_tree(
        "acme",
        "clock",
        widget_dir("clock", "1.0.0"),
    )))
    .await
    .expect("BUG: maintained install");

    let error = rig
        .install(valid_nar("weather", "1.0.0"))
        .await
        .expect_err("BUG: acme's clock already holds the uid");

    assert!(
        matches!(
            &error,
            WidgetInstallError::UidTaken { installed, .. } if installed == Path::new("acme/clock")
        ),
        "{error}"
    );
}

#[tokio::test]
async fn widgets_with_their_own_uids_install_side_by_side() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.0.0"))
        .await
        .expect("BUG: first install");

    rig.install(encode(&package_tree(
        "clock",
        widget_with_uid("clock", "1.0.0", OTHER_UID),
    )))
    .await
    .expect("BUG: distinct uids do not clash");

    assert_eq!(
        rig.installed_version("widget-example").as_deref(),
        Some("1.0.0")
    );
    assert_eq!(
        rig.installed_version("widget-clock").as_deref(),
        Some("1.0.0")
    );
}

#[tokio::test]
async fn a_maintained_widget_keeps_its_uid_across_an_upgrade() {
    let rig = Rig::new();
    let maintained = |version| {
        encode(&maintained_package_tree(
            "acme",
            "clock",
            widget_dir("clock", version),
        ))
    };
    rig.install(maintained("1.0.0"))
        .await
        .expect("BUG: first install");

    let upgraded = rig
        .install(maintained("1.1.0"))
        .await
        .expect("BUG: its own folder holds the uid, which is no clash");

    assert_eq!(
        upgraded.outcome,
        WidgetInstallOutcome::Replaced {
            previous_version: "1.0.0".to_owned()
        }
    );
}

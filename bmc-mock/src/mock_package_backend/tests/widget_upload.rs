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

//! Uploads unpack into the uploads dir under the same checks the device runs.

use super::*;
use bmc_upgrade::packages::{WidgetInstallError, WidgetInstallOutcome};
use bmc_upgrade::widget_package::WidgetUpload;
use bmc_upgrade::widget_package::fixtures::{
    encode, file, maintained_package_tree, manifest_json, package_tree, set, valid_nar, widget_dir,
};

fn backend(dir: &std::path::Path) -> (MockPackageBackend, std::path::PathBuf) {
    let path = write_scenario(dir, r#"{"run": "success"}"#);
    let uploads = dir.join("uploads");
    let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier())
        .with_uploads_path(Some(uploads.clone()));
    (backend, uploads)
}

async fn upload(
    backend: &MockPackageBackend,
    nar: Vec<u8>,
) -> Result<bmc_upgrade::packages::InstalledWidget, WidgetInstallError> {
    backend
        .install_widget_package(WidgetUpload::unannounced(Box::pin(std::io::Cursor::new(
            nar,
        ))))
        .await
}

#[tokio::test]
async fn an_uploaded_widget_is_unpacked_where_the_registry_discovers() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());

    let installed = upload(&backend, valid_nar("flip-clock", "1.0.0"))
        .await
        .expect("BUG: a valid archive installs");

    assert_eq!(installed.package_name, "widget-flip-clock");
    assert_eq!(installed.outcome, WidgetInstallOutcome::Installed);
    let discovered: Vec<_> = bmc_widget_manifest::widget_dirs(&uploads)
        .into_iter()
        .map(|dir| dir.expect("BUG: walk"))
        .collect();
    assert_eq!(discovered, [uploads.join("flip-clock")]);
    assert!(uploads.join("flip-clock/lib/wasm/flip-clock.wasm").exists());
}

#[tokio::test]
async fn a_widget_under_a_maintainer_folder_keeps_its_folder_and_name() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());
    let nar = encode(&maintained_package_tree(
        "acme",
        "flip-clock",
        widget_dir("flip-clock", "1.0.0"),
    ));

    let installed = upload(&backend, nar)
        .await
        .expect("BUG: a valid archive installs");

    assert_eq!(installed.package_name, "widget-acme_flip-clock");
    assert!(uploads.join("acme/flip-clock/manifest.json").exists());
    let listed = installable_widgets_from_dir(&uploads);
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].package_name, "widget-acme_flip-clock",
        "the scan must name the widget the way the install did"
    );
}

#[tokio::test]
async fn a_rejected_archive_writes_nothing() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());

    let error = upload(&backend, b"not a nar".to_vec())
        .await
        .expect_err("BUG: garbage must be rejected");

    assert!(matches!(error, WidgetInstallError::Rejected(_)));
    assert!(!uploads.exists(), "a rejected upload leaves no trace");
}

#[tokio::test]
async fn an_upload_the_client_abandons_answers_as_on_the_device() {
    use bmc_upgrade::widget_package::fixtures::ClientGone;
    use tokio::io::AsyncReadExt as _;

    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());
    let nar = valid_nar("flip-clock", "1.0.0");
    let half = nar[..nar.len().div_ceil(2)].to_vec();

    let error = backend
        .install_widget_package(WidgetUpload::unannounced(Box::pin(
            std::io::Cursor::new(half).chain(ClientGone),
        )))
        .await
        .expect_err("BUG: half an archive cannot install");

    assert!(
        matches!(error, WidgetInstallError::UploadAborted),
        "a 500 here and a 400 on the device would hide the client's doing: {error}"
    );
    assert!(!uploads.exists());
}

#[test]
fn a_failed_replacement_keeps_the_installed_widget() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let target = dir.path().join("flip-clock");
    std::fs::create_dir(&target).expect("BUG: target");
    std::fs::write(target.join("manifest.json"), b"installed").expect("BUG: manifest");
    let staging = dir.path().join("staging");
    std::fs::create_dir(&staging).expect("BUG: staging");

    let error = replace_tree(&target, &staging.join("missing"), &staging)
        .expect_err("BUG: a tree that is not there cannot move in");

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        std::fs::read(target.join("manifest.json")).expect("BUG: manifest"),
        b"installed",
        "the installed widget must be back where the registry finds it"
    );
    assert!(!staging.join("previous").exists());
}

#[tokio::test]
async fn versions_are_compared_like_the_device_does() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());
    upload(&backend, valid_nar("flip-clock", "1.0.0"))
        .await
        .expect("BUG: first install");

    let downgrade = upload(&backend, valid_nar("flip-clock", "0.9.0"))
        .await
        .expect_err("BUG: an older upload must be refused");
    assert!(matches!(downgrade, WidgetInstallError::Downgrade { .. }));

    let same = upload(&backend, valid_nar("flip-clock", "1.0.0"))
        .await
        .expect("BUG: same version");
    assert_eq!(same.outcome, WidgetInstallOutcome::AlreadyInstalled);

    let mut rebuilt = widget_dir("flip-clock", "1.0.0");
    set(&mut rebuilt, "assets/icon.svg", file(b"<svg>v2</svg>"));
    let reinstalled = upload(&backend, encode(&package_tree("flip-clock", rebuilt)))
        .await
        .expect("BUG: same version, other contents");
    assert_eq!(reinstalled.outcome, WidgetInstallOutcome::Reinstalled);
    assert_eq!(
        std::fs::read(uploads.join("flip-clock/assets/icon.svg")).expect("BUG: icon"),
        b"<svg>v2</svg>",
        "the rebuilt contents must be what the registry discovers"
    );

    let newer = upload(&backend, valid_nar("flip-clock", "1.1.0"))
        .await
        .expect("BUG: newer version");
    assert_eq!(
        newer.outcome,
        WidgetInstallOutcome::Replaced {
            previous_version: "1.0.0".to_owned()
        }
    );
}

#[tokio::test]
async fn a_flat_widget_named_like_a_maintainer_folder_leaves_its_widgets_alone() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());
    let maintained = encode(&maintained_package_tree(
        "acme",
        "flip-clock",
        widget_dir("flip-clock", "1.0.0"),
    ));
    upload(&backend, maintained)
        .await
        .expect("BUG: a maintained widget installs");

    let error = upload(&backend, valid_nar("acme", "1.0.0"))
        .await
        .expect_err("BUG: the flat widget would replace the maintainer's folder");

    assert!(
        matches!(error, WidgetInstallError::FolderTaken { .. }),
        "{error}"
    );
    assert!(uploads.join("acme/flip-clock/manifest.json").exists());
}

#[tokio::test]
async fn a_uid_a_bundled_widget_carries_is_refused() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let staged = dir.path().join("staged");
    std::fs::create_dir_all(staged.join("example")).expect("BUG: staged widget");
    std::fs::write(
        staged.join("example/manifest.json"),
        manifest_json("example", "1.0.0"),
    )
    .expect("BUG: staged manifest");
    let (backend, uploads) = backend(dir.path());
    let backend = backend.with_staging_path(Some(staged));

    let error = upload(&backend, valid_nar("flip-clock", "1.0.0"))
        .await
        .expect_err("BUG: the registry would show only one widget per uid");

    assert!(
        matches!(
            &error,
            WidgetInstallError::UidTaken { installed, .. } if installed == Path::new("example")
        ),
        "{error}"
    );
    assert!(!uploads.exists());
}

#[tokio::test]
async fn an_upload_clears_what_an_interrupted_one_left_staged() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let (backend, uploads) = backend(dir.path());
    let leftover = uploads.with_file_name("upload-staging/.tmpInterrupted");
    std::fs::create_dir_all(&leftover).expect("BUG: leftover dir");

    upload(&backend, valid_nar("flip-clock", "1.0.0"))
        .await
        .expect("BUG: a valid archive installs");

    assert!(!leftover.exists());
}

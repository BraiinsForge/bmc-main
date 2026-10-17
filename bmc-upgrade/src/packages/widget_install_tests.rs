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

//! `PackageBackend::install_widget_package` against a real profile
//! directory and a store that keeps trees on disk.

use std::path::{Path, PathBuf};

use super::{
    InstalledWidget, NixUpgradeConfig, PackageBackend, PackageUpgrader, WidgetInstallError,
    WidgetInstallOutcome,
};
use crate::widget_package::fixtures::{
    ClientGone, encode, maintained_package_tree, package_tree, set, valid_nar, widget_dir,
};
use crate::widget_package::{FolderClash, WidgetUpload, fixtures};
use bmc_nix::store::{
    AddToStoreError, RealizeEstimate, StoreCommandError, StoreOperations, StorePathError,
};
use bmc_nix::types::ResolvedPackage;

/// A store that files added trees under one directory as `<hash>-<name>`,
/// hashed over the tree like nix, and refuses to delete a path a profile
/// generation still links to, as nix does.
#[derive(Debug)]
struct DiskStore {
    root: PathBuf,
    profile_dir: PathBuf,
    free_bytes: u64,
    deleted: std::sync::Mutex<Vec<String>>,
}

fn tree_hash(path: &Path, hasher: &mut std::hash::DefaultHasher) {
    use std::hash::Hash as _;

    let mut entries: Vec<_> = std::fs::read_dir(path)
        .expect("BUG: readable tree")
        .map(|entry| entry.expect("BUG: entry").path())
        .collect();
    entries.sort();
    for entry in entries {
        entry.file_name().hash(hasher);
        if entry.is_dir() {
            tree_hash(&entry, hasher);
        } else {
            std::fs::read(&entry)
                .expect("BUG: readable file")
                .hash(hasher);
        }
    }
}

fn links_into(dir: &Path, store_path: &Path) -> bool {
    std::fs::read_dir(dir)
        .expect("BUG: readable profile")
        .map(|entry| entry.expect("BUG: entry").path())
        .any(|entry| {
            if let Ok(target) = std::fs::read_link(&entry) {
                target.starts_with(store_path)
            } else {
                entry.is_dir() && links_into(&entry, store_path)
            }
        })
}

fn store_io(command: &'static str, source: std::io::Error) -> StoreCommandError {
    StoreCommandError::Io { command, source }
}

impl StoreOperations for DiskStore {
    async fn estimate_realization(
        &self,
        _packages: &[ResolvedPackage],
    ) -> Result<RealizeEstimate, StorePathError> {
        Ok(RealizeEstimate {
            fetch_paths: 0,
            download_bytes: 0,
            unpacked_bytes: 0,
        })
    }

    fn store_free_bytes(&self, _profile_dir: &Path) -> std::io::Result<u64> {
        Ok(self.free_bytes)
    }

    async fn realize_store_paths(
        &self,
        _packages: &[ResolvedPackage],
        _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
    ) -> Result<(), StorePathError> {
        Ok(())
    }

    async fn verify_store_paths(
        &self,
        _packages: &[ResolvedPackage],
    ) -> Result<(), StorePathError> {
        Ok(())
    }

    async fn collect_garbage(
        &self,
        _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
    ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
        unreachable!("BUG: a widget install never collects garbage")
    }

    async fn add_to_store(&self, path: &Path) -> Result<String, AddToStoreError> {
        use std::hash::Hasher as _;

        let name = path
            .file_name()
            .expect("BUG: a staged tree has a name")
            .to_str()
            .expect("BUG: UTF-8 name");
        let mut hasher = std::hash::DefaultHasher::new();
        tree_hash(path, &mut hasher);
        let target = self.root.join(format!("{:016x}-{name}", hasher.finish()));
        if !target.exists() {
            copy_tree(path, &target)
                .map_err(|source| AddToStoreError::Command(store_io("--add", source)))?;
        }
        Ok(target.to_str().expect("BUG: UTF-8 path").to_owned())
    }

    async fn delete_store_path(&self, store_path: &str) -> Result<(), StoreCommandError> {
        if links_into(&self.profile_dir, Path::new(store_path)) {
            return Err(StoreCommandError::Exited {
                command: "--delete",
                status: std::process::ExitStatus::default(),
                stderr: format!("error: Cannot delete path '{store_path}' since it is still alive"),
            });
        }
        std::fs::remove_dir_all(store_path).map_err(|source| store_io("--delete", source))?;
        self.deleted
            .lock()
            .expect("BUG: mutex poisoned")
            .push(store_path.to_owned());
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Activation {
    MovesCurrent,
    Fails,
}

/// A profile already holds a `core` package whose activation entrypoint
/// moves `current`, as the real one does; the rig needs the same so a
/// widget install can activate.
fn seed_core_package(profile_dir: &Path, store_dir: &Path, mode: Activation) {
    use std::os::unix::fs::PermissionsExt as _;

    let core = store_dir.join("core");
    let activation = core.join("core/activation");
    std::fs::create_dir_all(&activation).expect("BUG: core activation dir");
    let entrypoint = activation.join("entrypoint");
    let script = match mode {
        Activation::MovesCurrent => {
            "#!/bin/sh\nln -sfn \"$(basename \"$PROFILE_NEW_GENERATION\")\" \"$(dirname \"$PROFILE_NEW_GENERATION\")/current\"\n"
        }
        Activation::Fails => "#!/bin/sh\necho 'activation broke' >&2\nexit 1\n",
    };
    std::fs::write(&entrypoint, script).expect("BUG: write entrypoint");
    std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
        .expect("BUG: chmod entrypoint");

    let generation = profile_dir.join("1-link");
    std::fs::create_dir_all(&generation).expect("BUG: generation dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "core".to_owned(),
        bmc_nix::types::ManifestPackage {
            version: "1.0.0".to_owned(),
            store_path: core.to_str().expect("BUG: UTF-8 path").to_owned(),
            category: Some("system".to_owned()),
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: bmc_nix::types::InstalledBy::System,
            installed_from: "srv".to_owned(),
            pinned: None,
        },
    );
    bmc_nix::manifest::write_manifest(&generation, &bmc_nix::types::Manifest { packages })
        .expect("BUG: write base manifest");
    std::os::unix::fs::symlink("1-link", profile_dir.join("current")).expect("BUG: current link");
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

struct Rig {
    dir: tempfile::TempDir,
    profile_dir: PathBuf,
    store_dir: PathBuf,
    upgrader: PackageUpgrader<DiskStore>,
}

impl Rig {
    fn new() -> Self {
        Self::with_activation(Activation::MovesCurrent)
    }

    fn with_free_bytes(free_bytes: u64) -> Self {
        let mut rig = Self::new();
        rig.upgrader.nix.free_bytes = free_bytes;
        rig
    }

    fn with_activation(activation: Activation) -> Self {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("profile");
        let store_dir = dir.path().join("store");
        std::fs::create_dir_all(&profile_dir).expect("BUG: profile dir");
        std::fs::create_dir_all(&store_dir).expect("BUG: store dir");
        seed_core_package(&profile_dir, &store_dir, activation);
        let upgrader = PackageUpgrader::with_store(
            NixUpgradeConfig {
                servers_config_path: dir.path().join("servers.json"),
                gc_config_path: dir.path().join("gc.json"),
                profile_dir: profile_dir.clone(),
                staging_dir: dir.path().join("staging"),
                hooks_dir: "hooks".to_owned(),
                hooks_override_path: None,
            },
            DiskStore {
                root: store_dir.clone(),
                profile_dir: profile_dir.clone(),
                free_bytes: u64::MAX,
                deleted: std::sync::Mutex::new(Vec::new()),
            },
        );
        Self {
            dir,
            profile_dir,
            store_dir,
            upgrader,
        }
    }

    /// Install `nar` announced at its true size, as a browser upload is.
    async fn install(&self, nar: Vec<u8>) -> Result<InstalledWidget, WidgetInstallError> {
        let upload = WidgetUpload {
            announced_bytes: Some(nar.len() as u64),
            nar: Box::pin(std::io::Cursor::new(nar)),
        };
        self.upgrader.install_widget_package(upload).await
    }

    fn store_entries(&self) -> Vec<String> {
        let mut entries: Vec<_> = std::fs::read_dir(&self.store_dir)
            .expect("BUG: store dir")
            .map(|entry| entry.expect("BUG: entry").file_name())
            .map(|name| name.to_str().expect("BUG: UTF-8 name").to_owned())
            .collect();
        entries.sort();
        entries
    }

    fn widget_store_paths(&self) -> Vec<String> {
        self.store_entries()
            .into_iter()
            .filter(|name| name.ends_with("-bmc-widget-example"))
            .collect()
    }

    /// An install has a checked tree staged from just before it locks
    /// the profile until it is done with the upload.
    fn has_staged_tree(&self) -> bool {
        std::fs::read_dir(self.dir.path().join("staging"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|upload| upload.path().join("bmc-widget-example").exists())
    }

    fn deleted(&self) -> Vec<String> {
        self.upgrader
            .nix
            .deleted
            .lock()
            .expect("BUG: mutex poisoned")
            .clone()
    }

    fn installed_version(&self, package_name: &str) -> Option<String> {
        bmc_nix::manifest::read_current_manifest(&self.profile_dir)
            .ok()?
            .packages
            .get(package_name)
            .map(|package| package.version.clone())
    }
}

#[tokio::test]
async fn an_uploaded_widget_lands_in_the_active_profile() {
    let rig = Rig::new();

    let installed = rig
        .install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: a valid archive installs");

    assert_eq!(
        installed,
        InstalledWidget {
            package_name: "widget-example".to_owned(),
            display_name: "Example".to_owned(),
            version: "1.2.3".to_owned(),
            outcome: WidgetInstallOutcome::Installed,
        }
    );
    assert_eq!(
        rig.installed_version("widget-example").as_deref(),
        Some("1.2.3")
    );
    let wrapper = rig
        .profile_dir
        .join("current/lib/bmc-widgets/example/bin/example");
    assert!(
        wrapper.exists(),
        "the activated generation must expose the widget where discovery looks"
    );
    assert_eq!(
        rig.widget_store_paths().len(),
        1,
        "the store path is named like an index-built widget package"
    );
}

#[tokio::test]
async fn an_upload_that_cannot_fit_is_refused_before_anything_is_staged() {
    let rig = Rig::with_free_bytes(1);

    let error = rig
        .install(valid_nar("example", "1.2.3"))
        .await
        .expect_err("BUG: no room, no install");

    assert!(
        matches!(
            error,
            WidgetInstallError::Install(bmc_nix::upgrade::InstallError::NotEnoughSpace { .. })
        ),
        "{error}"
    );
    assert!(
        !rig.dir.path().join("staging").exists(),
        "the archive must not be unpacked into a filesystem that cannot hold it"
    );
    assert_eq!(rig.store_entries(), ["core"]);
}

#[tokio::test]
async fn an_install_clears_what_an_interrupted_one_left_staged() {
    let rig = Rig::new();
    let leftover = rig.dir.path().join("staging/.tmpInterrupted/package");
    std::fs::create_dir_all(&leftover).expect("BUG: leftover dir");
    std::fs::write(leftover.join("widget.wasm"), [0_u8; 4096]).expect("BUG: leftover file");

    rig.install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: a valid package installs");

    let staging: Vec<_> = std::fs::read_dir(rig.dir.path().join("staging"))
        .expect("BUG: staging dir")
        .collect();
    assert!(
        staging.is_empty(),
        "nothing else clears the staging dir, and the space check would count what stays there"
    );
}

#[tokio::test]
async fn the_same_version_with_other_contents_is_reinstalled() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: first install");

    let mut rebuilt = widget_dir("example", "1.2.3");
    set(
        &mut rebuilt,
        "assets/icon.svg",
        fixtures::file(b"<svg>v2</svg>"),
    );
    let reinstalled = rig
        .install(encode(&package_tree("example", rebuilt)))
        .await
        .expect("BUG: a rebuilt widget installs");

    assert_eq!(reinstalled.outcome, WidgetInstallOutcome::Reinstalled);
    assert_eq!(
        rig.widget_store_paths().len(),
        2,
        "different contents are a different store path"
    );
    let icon = rig
        .profile_dir
        .join("current/lib/bmc-widgets/example/assets/icon.svg");
    assert_eq!(
        std::fs::read(icon).expect("BUG: icon in the active generation"),
        b"<svg>v2</svg>",
        "the active generation must serve the rebuilt contents"
    );
}

#[tokio::test]
async fn a_rejected_archive_changes_nothing() {
    let rig = Rig::new();

    let error = rig
        .install(b"not an archive".to_vec())
        .await
        .expect_err("BUG: garbage is rejected");

    assert!(matches!(error, WidgetInstallError::Rejected(_)), "{error}");
    assert_eq!(
        std::fs::read_link(rig.profile_dir.join("current")).expect("BUG: current"),
        Path::new("1-link"),
        "the active generation must not move"
    );
    assert_eq!(
        rig.store_entries(),
        ["core"],
        "nothing may reach the store before validation passes"
    );
}

#[tokio::test]
async fn an_older_upload_is_refused_instead_of_silently_kept() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: first install");

    let error = rig
        .install(valid_nar("example", "1.0.0"))
        .await
        .expect_err("BUG: a downgrade is refused");

    assert!(
        matches!(
            &error,
            WidgetInstallError::Downgrade { installed, uploaded, .. }
                if installed == "1.2.3" && uploaded == "1.0.0"
        ),
        "{error}"
    );
    assert_eq!(
        rig.installed_version("widget-example").as_deref(),
        Some("1.2.3")
    );
    assert!(
        rig.deleted().is_empty() && rig.widget_store_paths().len() == 1,
        "a downgrade is refused before its archive is imported, not imported and discarded"
    );
}

#[tokio::test]
async fn a_maintainer_folder_named_like_an_installed_widget_is_refused() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.0.0"))
        .await
        .expect("BUG: flat install");
    let stored = rig.store_entries();

    let error = rig
        .install(encode(&maintained_package_tree(
            "example",
            "clock",
            widget_dir("clock", "1.0.0"),
        )))
        .await
        .expect_err("BUG: discovery would show only one of the two");

    assert!(
        matches!(
            &error,
            WidgetInstallError::FolderTaken { uploaded, installed: FolderClash::FlatWidget(name) }
                if uploaded == Path::new("example/clock") && name == "example"
        ),
        "{error}"
    );
    assert_eq!(rig.installed_version("widget-example_clock"), None);
    assert_eq!(
        rig.store_entries(),
        stored,
        "a refused upload is not imported"
    );
}

#[tokio::test]
async fn a_widget_named_like_an_installed_maintainer_folder_is_refused() {
    let rig = Rig::new();
    rig.install(encode(&maintained_package_tree(
        "example",
        "clock",
        widget_dir("clock", "1.0.0"),
    )))
    .await
    .expect("BUG: maintained install");

    let error = rig
        .install(valid_nar("example", "1.0.0"))
        .await
        .expect_err("BUG: the flat widget would hide the maintainer's widgets");

    assert!(
        matches!(
            &error,
            WidgetInstallError::FolderTaken { uploaded, installed: FolderClash::MaintainerFolder(name) }
                if uploaded == Path::new("example") && name == "example"
        ),
        "{error}"
    );
    assert_eq!(rig.installed_version("widget-example"), None);
}

#[tokio::test]
async fn a_newer_version_another_writer_installs_meanwhile_still_refuses_the_older_upload() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: newer install");
    let current = rig.profile_dir.join("current");
    let point_current_at = |generation: &str| {
        std::fs::remove_file(&current).expect("BUG: current");
        std::os::unix::fs::symlink(generation, &current).expect("BUG: current");
    };
    point_current_at("1-link");
    let held = bmc_nix::profile::lock_profile(&rig.profile_dir)
        .await
        .expect("BUG: profile lock");

    // The other writer keeps the profile locked until 1.2.3 is active again
    // and lets go only once the older upload is staged,
    // past any check made before the lock.
    let other_writer = async {
        while !rig.has_staged_tree() {
            tokio::task::yield_now().await;
        }
        point_current_at("2-link");
        drop(held);
    };
    let (result, ()) = tokio::join!(rig.install(valid_nar("example", "1.0.0")), other_writer);

    let error = result.expect_err("BUG: the version under the lock is newer");
    assert!(
        matches!(error, WidgetInstallError::Downgrade { .. }),
        "{error}"
    );
    assert_eq!(
        rig.installed_version("widget-example").as_deref(),
        Some("1.2.3")
    );
    assert_eq!(
        rig.widget_store_paths().len(),
        1,
        "the refused upload must not linger in the store until the next GC"
    );
}

#[tokio::test]
async fn a_newer_upload_replaces_and_a_repeat_is_reported_as_such() {
    let rig = Rig::new();
    rig.install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: first install");

    let repeated = rig
        .install(valid_nar("example", "1.2.3"))
        .await
        .expect("BUG: a repeat install succeeds");
    assert_eq!(repeated.outcome, WidgetInstallOutcome::AlreadyInstalled);

    let replaced = rig
        .install(valid_nar("example", "1.3.0"))
        .await
        .expect("BUG: a newer version installs");
    assert_eq!(
        replaced.outcome,
        WidgetInstallOutcome::Replaced {
            previous_version: "1.2.3".to_owned()
        }
    );
    assert_eq!(
        rig.installed_version("widget-example").as_deref(),
        Some("1.3.0")
    );
}

#[tokio::test]
async fn a_failed_activation_discards_the_imported_package() {
    let rig = Rig::with_activation(Activation::Fails);

    let error = rig
        .install(valid_nar("example", "1.2.3"))
        .await
        .expect_err("BUG: a broken activation fails the install");

    assert!(matches!(error, WidgetInstallError::Install(_)), "{error}");
    assert_eq!(rig.installed_version("widget-example"), None);
    let discarded = rig
        .upgrader
        .nix
        .deleted
        .lock()
        .expect("BUG: mutex poisoned");
    assert_eq!(
        discarded.len(),
        1,
        "the store path nothing references must be deleted, not left for GC"
    );
    assert!(
        discarded[0].ends_with("-bmc-widget-example"),
        "{discarded:?}"
    );
    assert!(
        rig.widget_store_paths().is_empty(),
        "the delete must have removed it"
    );
    let generations: Vec<_> = rig
        .profile_dir
        .read_dir()
        .expect("BUG: profile dir")
        .map(|entry| entry.expect("BUG: entry").file_name())
        .filter(|name| name.to_string_lossy().contains("-link"))
        .collect();
    assert_eq!(
        generations,
        ["1-link"],
        "the generation the failed activation built must not survive to pin the path"
    );
}

#[tokio::test]
async fn a_failed_install_spares_a_generation_another_writer_built_meanwhile() {
    let rig = Rig::with_activation(Activation::Fails);
    let held = bmc_nix::profile::lock_profile(&rig.profile_dir)
        .await
        .expect("BUG: profile lock");

    // The other writer keeps the profile locked until its generation exists
    // and lets go only once the upload is staged,
    // so the install reads its floor after that generation appears.
    let other_writer = async {
        while !rig.has_staged_tree() {
            tokio::task::yield_now().await;
        }
        copy_tree(
            &rig.profile_dir.join("1-link"),
            &rig.profile_dir.join("2-link"),
        )
        .expect("BUG: other writer's generation");
        drop(held);
    };
    let (result, ()) = tokio::join!(rig.install(valid_nar("example", "1.2.3")), other_writer);

    result.expect_err("BUG: a broken activation fails the install");
    assert!(
        rig.profile_dir.join("2-link").exists(),
        "the generation the other writer built must survive the failed install's discard"
    );
    assert!(
        !rig.profile_dir.join("3-link").exists(),
        "the generation the failed install built must still be discarded"
    );
}

#[tokio::test]
async fn an_install_waiting_for_the_profile_leaves_nothing_in_the_store_for_a_gc_to_sweep() {
    let rig = Rig::new();
    let held = bmc_nix::profile::lock_profile(&rig.profile_dir)
        .await
        .expect("BUG: profile lock");

    // A GC holds the same lock and sweeps every path nothing roots.
    let gc = async {
        while !rig.has_staged_tree() && rig.widget_store_paths().is_empty() {
            tokio::task::yield_now().await;
        }
        let unrooted = rig.widget_store_paths();
        drop(held);
        unrooted
    };
    let (result, unrooted) = tokio::join!(rig.install(valid_nar("example", "1.2.3")), gc);

    assert!(
        unrooted.is_empty(),
        "the upload must not be in the store while the install waits for the lock: {unrooted:?}"
    );
    result.expect("BUG: the install goes ahead once the lock is free");
}

#[tokio::test]
async fn an_install_gives_up_on_a_profile_another_writer_keeps_locked() {
    let mut rig = Rig::new();
    rig.upgrader.profile_lock_timeout = std::time::Duration::from_millis(50);
    let _held = bmc_nix::profile::lock_profile(&rig.profile_dir)
        .await
        .expect("BUG: profile lock");

    let error = rig
        .install(valid_nar("example", "1.2.3"))
        .await
        .expect_err("BUG: the profile stays locked");

    assert!(matches!(error, WidgetInstallError::ProfileBusy), "{error}");
    assert!(
        rig.widget_store_paths().is_empty(),
        "nothing is imported before the profile is locked"
    );
}

#[tokio::test]
async fn an_install_that_cannot_open_the_profile_lock_leaves_nothing_in_the_store() {
    let rig = Rig::new();
    let lock_file = rig.profile_dir.join(".lock");
    if lock_file.exists() {
        std::fs::remove_file(&lock_file).expect("BUG: lock file");
    }
    std::fs::create_dir(&lock_file).expect("BUG: a directory where the lock file goes");

    let error = rig
        .install(valid_nar("example", "1.2.3"))
        .await
        .expect_err("BUG: the lock file cannot be opened");

    assert!(
        matches!(
            error,
            WidgetInstallError::Install(bmc_nix::upgrade::InstallError::Lock(_))
        ),
        "{error}"
    );
    assert!(
        rig.widget_store_paths().is_empty(),
        "nothing is imported before the profile is locked"
    );
}

#[tokio::test]
async fn an_oversized_upload_is_refused_and_leaves_nothing_behind() {
    let rig = Rig::new();
    let mut nar = valid_nar("example", "1.2.3");
    nar.resize(
        usize::try_from(crate::widget_package::MAX_ARCHIVE_BYTES + 1)
            .expect("BUG: 64 MiB fits in memory"),
        0,
    );

    let error = rig
        .install(nar)
        .await
        .expect_err("BUG: an archive over the limit is refused");

    assert!(
        matches!(
            error,
            WidgetInstallError::Rejected(crate::widget_package::WidgetPackageError::TooLarge)
        ),
        "{error}"
    );
    assert_eq!(rig.installed_version("widget-example"), None);
    let staging: Vec<_> = std::fs::read_dir(rig.dir.path().join("staging"))
        .expect("BUG: staging dir")
        .collect();
    assert!(staging.is_empty(), "the staging dir is cleaned up");
}

#[tokio::test]
async fn an_upload_the_client_abandons_is_not_a_store_fault() {
    use tokio::io::AsyncReadExt as _;

    let rig = Rig::new();
    let nar = valid_nar("example", "1.2.3");
    let half = nar[..nar.len().div_ceil(2)].to_vec();
    let upload = WidgetUpload {
        announced_bytes: Some(nar.len() as u64),
        nar: Box::pin(std::io::Cursor::new(half).chain(ClientGone)),
    };

    let error = rig
        .upgrader
        .install_widget_package(upload)
        .await
        .expect_err("BUG: half an archive cannot install");

    assert!(
        matches!(error, WidgetInstallError::UploadAborted),
        "{error}"
    );
    assert_eq!(rig.installed_version("widget-example"), None);
    let staging: Vec<_> = std::fs::read_dir(rig.dir.path().join("staging"))
        .expect("BUG: staging dir")
        .collect();
    assert!(staging.is_empty(), "the staging dir is cleaned up");
}

#[test]
fn a_staged_tree_the_device_cannot_read_is_its_own_fault() {
    let unreadable = crate::widget_package::WidgetPackageError::Io {
        path: "lib/bmc-widgets/example/manifest.json".to_owned(),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    };

    assert!(matches!(
        super::package_error(unreadable),
        WidgetInstallError::Failed(_)
    ));
}

#[test]
fn a_version_spelled_two_ways_is_a_reinstall_not_a_replacement() {
    assert!(super::widget_install::same_version("1.0", "1.0.0"));
    assert!(!super::widget_install::same_version("1.0", "1.0.1"));
    assert!(
        super::widget_install::same_version("nightly", "nightly"),
        "a version semver cannot read still equals itself"
    );
}

mod entry_bound;
mod uid_clash;

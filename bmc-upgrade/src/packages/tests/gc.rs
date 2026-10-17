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

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct GcStore {
    collect_calls: Arc<AtomicUsize>,
    fail: bool,
}

impl GcStore {
    fn successful() -> (Self, Arc<AtomicUsize>) {
        Self::new(false)
    }

    fn failing() -> (Self, Arc<AtomicUsize>) {
        Self::new(true)
    }

    fn new(fail: bool) -> (Self, Arc<AtomicUsize>) {
        let collect_calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                collect_calls: Arc::clone(&collect_calls),
                fail,
            },
            collect_calls,
        )
    }
}

impl bmc_nix::store::StoreOperations for GcStore {
    async fn estimate_realization(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
    ) -> Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError> {
        unreachable!("BUG: GcStore only serves garbage collection")
    }

    async fn delete_store_path(
        &self,
        _store_path: &str,
    ) -> Result<(), bmc_nix::store::StoreCommandError> {
        unreachable!("BUG: GcStore only serves garbage collection")
    }

    fn store_free_bytes(&self, _profile_dir: &std::path::Path) -> std::io::Result<u64> {
        unreachable!("BUG: GcStore only serves garbage collection")
    }

    async fn realize_store_paths(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
        _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
    ) -> Result<(), bmc_nix::store::StorePathError> {
        unreachable!("BUG: GcStore only serves garbage collection")
    }

    async fn verify_store_paths(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
    ) -> Result<(), bmc_nix::store::StorePathError> {
        unreachable!("BUG: GcStore only serves garbage collection")
    }

    async fn collect_garbage(
        &self,
        _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
    ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
        self.collect_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(bmc_nix::gc::CollectGarbageError::NixCommand(
                std::io::Error::other("store gc failed"),
            ))
        } else {
            Ok(())
        }
    }

    async fn add_to_store(&self, _path: &Path) -> Result<String, bmc_nix::store::AddToStoreError> {
        unreachable!("BUG: GcStore only serves garbage collection")
    }
}

/// What the periodic job asks for: give up on a busy profile and sweep
/// only when cleanup freed something.
fn periodic_request() -> PackageGcRequest {
    PackageGcRequest {
        on_busy: bmc_nix::gc::OnBusy::Skip,
        sweep: bmc_nix::gc::Sweep::WhenGenerationsRemoved,
    }
}

/// What the upgrade preflight asks for: wait for the profile and sweep
/// unconditionally.
fn forced_request() -> PackageGcRequest {
    PackageGcRequest {
        on_busy: bmc_nix::gc::OnBusy::Wait,
        sweep: bmc_nix::gc::Sweep::Always,
    }
}

/// Three generations with the newest current: the default policy keeps two
/// and so has exactly one generation to remove.
fn profile_with_a_removable_generation(profile_dir: &Path) {
    for number in 1..=3_usize {
        let generation = profile_dir.join(format!("{number}-link"));
        std::fs::create_dir_all(&generation).expect("BUG: create generation");
        std::fs::write(generation.join("manifest"), r#"{"packages":{}}"#)
            .expect("BUG: write manifest");
    }
    std::os::unix::fs::symlink("3-link", profile_dir.join("current"))
        .expect("BUG: link current generation");
}

#[tokio::test]
async fn gc_missing_config_uses_defaults() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

    let outcome = upgrader
        .gc(forced_request())
        .await
        .expect("missing gc config must use default policy");

    assert_eq!(outcome, PackageGcOutcome::Collected);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gc_forwards_a_conditional_sweep() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

    let outcome = upgrader
        .gc(periodic_request())
        .await
        .expect("an empty profile is a successful gc outcome");

    assert_eq!(outcome, PackageGcOutcome::NothingToCollect);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gc_sweeps_when_cleanup_removed_a_generation() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let config = test_nix_config(dir.path(), &servers);
    std::fs::create_dir(&config.profile_dir).expect("BUG: create profile");
    profile_with_a_removable_generation(&config.profile_dir);
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(config, store);

    let outcome = upgrader
        .gc(periodic_request())
        .await
        .expect("a removed generation must trigger the sweep");

    assert_eq!(outcome, PackageGcOutcome::Collected);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gc_forwards_an_unconditional_sweep() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);
    upgrader
        .gc(forced_request())
        .await
        .expect("the first forced gc must collect");

    let outcome = upgrader
        .gc(forced_request())
        .await
        .expect("a forced gc must collect with nothing to remove");

    assert_eq!(outcome, PackageGcOutcome::Collected);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_disabling_configuration_does_not_stop_backend_collection() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    std::fs::write(dir.path().join("gc.json"), r#"{"periodic":"disabled"}"#)
        .expect("BUG: write gc config");
    let config = test_nix_config(dir.path(), &servers);
    std::fs::create_dir(&config.profile_dir).expect("BUG: create profile");
    profile_with_a_removable_generation(&config.profile_dir);
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(config, store);

    let outcome = upgrader
        .gc(periodic_request())
        .await
        .expect("a disabling configuration must not fail the request");

    assert_eq!(
        outcome,
        PackageGcOutcome::Collected,
        "the toggle is enforced by the periodic job, not the backend"
    );
    assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gc_reports_a_config_read_failure() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    std::fs::create_dir(dir.path().join("gc.json")).expect("BUG: create config directory");
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

    let err = upgrader
        .gc(forced_request())
        .await
        .expect_err("a directory passed as config must fail to read");

    assert!(matches!(err, PackageGcError::ConfigRead(_)));
    assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gc_reports_a_config_parse_failure() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    std::fs::write(dir.path().join("gc.json"), "{").expect("BUG: write malformed config");
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

    let err = upgrader
        .gc(forced_request())
        .await
        .expect_err("malformed gc config must fail to parse");

    assert!(matches!(err, PackageGcError::ConfigParse(_)));
    assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gc_returns_busy_without_collecting() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let config = test_nix_config(dir.path(), &servers);
    let _lock = bmc_nix::profile::try_lock_profile(&config.profile_dir)
        .expect("profile lock attempt must succeed")
        .expect("profile lock must be available");
    let (store, collect_calls) = GcStore::successful();
    let upgrader = PackageUpgrader::with_store(config, store);

    let outcome = upgrader
        .gc(periodic_request())
        .await
        .expect("busy profile is a successful gc outcome");

    assert_eq!(outcome, PackageGcOutcome::Busy);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gc_reports_a_store_failure_as_operational() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let (store, collect_calls) = GcStore::failing();
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

    let err = upgrader
        .gc(forced_request())
        .await
        .expect_err("store collection failure must be reported");

    assert!(matches!(err, PackageGcError::Operational(_)));
    assert_eq!(err.unswept_removals(), 0);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gc_reports_removals_left_unswept_by_a_store_failure() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let config = test_nix_config(dir.path(), &servers);
    std::fs::create_dir(&config.profile_dir).expect("BUG: create profile");
    profile_with_a_removable_generation(&config.profile_dir);
    let (store, collect_calls) = GcStore::failing();
    let upgrader = PackageUpgrader::with_store(config, store);

    let err = upgrader
        .gc(forced_request())
        .await
        .expect_err("store collection failure must be reported");

    assert!(matches!(
        err,
        PackageGcError::UnsweptRemovals { removed: 1, .. }
    ));
    assert_eq!(err.unswept_removals(), 1);
    assert_eq!(collect_calls.load(Ordering::SeqCst), 1);
}

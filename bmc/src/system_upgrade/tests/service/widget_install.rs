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
use bmc_upgrade::packages::{InstalledWidget, WidgetInstallError, WidgetInstallOutcome};
use bmc_upgrade::widget_package::{FolderClash, WidgetPackageError};
use tokio::io::AsyncReadExt as _;
use tokio::sync::oneshot;

/// Answers one upload with a scripted result and records what it was given.
#[derive(Debug)]
struct ScriptedBackend {
    result: std::sync::Mutex<Option<Result<InstalledWidget, WidgetInstallError>>>,
    uploads: std::sync::Mutex<Vec<Vec<u8>>>,
    hold: std::sync::Mutex<Option<Hold>>,
}

/// Parks the install between the upload and its result until released.
#[derive(Debug)]
struct Hold {
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl ScriptedBackend {
    fn answering(result: Result<InstalledWidget, WidgetInstallError>) -> Arc<Self> {
        Arc::new(Self {
            result: std::sync::Mutex::new(Some(result)),
            uploads: std::sync::Mutex::new(Vec::new()),
            hold: std::sync::Mutex::new(None),
        })
    }

    fn holding(
        result: Result<InstalledWidget, WidgetInstallError>,
    ) -> (Arc<Self>, oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (started, started_rx) = oneshot::channel();
        let (release_tx, release) = oneshot::channel();
        let backend = Arc::new(Self {
            result: std::sync::Mutex::new(Some(result)),
            uploads: std::sync::Mutex::new(Vec::new()),
            hold: std::sync::Mutex::new(Some(Hold { started, release })),
        });
        (backend, started_rx, release_tx)
    }

    /// Panics inside the install, the way a bug in the backend would.
    fn crashing() -> Arc<Self> {
        Arc::new(Self {
            result: std::sync::Mutex::new(None),
            uploads: std::sync::Mutex::new(Vec::new()),
            hold: std::sync::Mutex::new(None),
        })
    }

    fn uploads(&self) -> usize {
        self.uploads.lock().expect("BUG: uploads poisoned").len()
    }
}

#[async_trait::async_trait]
impl PackageBackend for ScriptedBackend {
    async fn install_widget_package(
        &self,
        mut upload: bmc_upgrade::widget_package::WidgetUpload,
    ) -> Result<InstalledWidget, WidgetInstallError> {
        let mut bytes = Vec::new();
        upload
            .nar
            .read_to_end(&mut bytes)
            .await
            .expect("BUG: the scripted upload is in memory");
        self.uploads
            .lock()
            .expect("BUG: uploads poisoned")
            .push(bytes);
        let hold = self.hold.lock().expect("BUG: hold poisoned").take();
        if let Some(hold) = hold {
            hold.started
                .send(())
                .expect("BUG: the test waits for the install to start");
            hold.release
                .await
                .expect("BUG: the test releases the install");
        }
        self.result
            .lock()
            .expect("BUG: result poisoned")
            .take()
            .expect("BUG: the backend answers one upload")
    }
    async fn gc(&self, _request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn probe(
        &self,
        _firmware: Option<&str>,
        _estimate: EstimateMode,
        _install: &[String],
    ) -> PackageProbe {
        unimplemented!("{UNREACHABLE}")
    }
    async fn apply(
        &self,
        _merged: bmc_nix::types::MergedIndex,
        _install: Vec<String>,
        _progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        unimplemented!("{UNREACHABLE}")
    }
    async fn list_installable_packages(
        &self,
        _firmware: Option<&str>,
    ) -> Result<Vec<InstallablePackage>, PackageProbeError> {
        unimplemented!("{UNREACHABLE}")
    }
    fn store_free_bytes(&self) -> std::io::Result<u64> {
        unimplemented!("{UNREACHABLE}")
    }
}

#[derive(Debug, Default)]
struct RefreshCounter(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl WidgetLifecycle for RefreshCounter {
    async fn stop_all_widgets(&self) {
        unimplemented!("{UNREACHABLE}")
    }
    async fn restart_widgets(&self) {
        unimplemented!("{UNREACHABLE}")
    }
    async fn refresh_widgets(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn service_with(
    backend: Arc<ScriptedBackend>,
    lifecycle: Arc<RefreshCounter>,
) -> (
    SystemUpgradeService<StubIndex, StubManager>,
    watch::Sender<Timezone>,
) {
    let (timezone_sender, timezone_receiver) = watch::channel(Timezone::default());
    let scheduler = JobScheduler::init(timezone_receiver, None).await;
    let service = SystemUpgradeService::new(
        StubIndex,
        &PathBuf::from("/nonexistent/upgrade.img"),
        Arc::new(StubManager::default()),
        StateService::new(),
        scheduler,
        tokio::time::Instant::now(),
        capabilities(Product::Bmc100),
        backend,
        lifecycle,
        PathBuf::from("/nonexistent/pending-install"),
    );
    (service, timezone_sender)
}

fn installed() -> InstalledWidget {
    InstalledWidget {
        package_name: "widget-flip-clock".to_owned(),
        display_name: "Flip clock".to_owned(),
        version: "1.2.0".to_owned(),
        outcome: WidgetInstallOutcome::Installed,
    }
}

#[tokio::test]
async fn an_installed_upload_refreshes_the_picker_once() {
    let backend = ScriptedBackend::answering(Ok(installed()));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend.clone(), lifecycle.clone()).await;

    let result = service.install_widget_package(archive(b"nar")).await;

    assert_eq!(result.expect("BUG: scripted success"), installed());
    assert_eq!(
        lifecycle.0.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the widget must become placeable without a restart"
    );
    assert_eq!(backend.uploads(), 1);
    assert!(
        service.run_gate.try_lock().is_ok(),
        "the gate is released after the install"
    );
}

#[tokio::test]
async fn a_rejected_archive_is_the_clients_fault_and_refreshes_nothing() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::Rejected(
        WidgetPackageError::NativeBinary("bin/flip-clock".to_owned()),
    )));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend, lifecycle.clone()).await;

    let error = service
        .install_widget_package(archive(b"elf"))
        .await
        .expect_err("BUG: scripted rejection");

    assert!(matches!(
        error,
        SystemUpgradeError::WidgetPackageRejected(_)
    ));
    assert_eq!(lifecycle.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_refused_downgrade_is_reported_as_one() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::Downgrade {
        package_name: "widget-flip-clock".to_owned(),
        installed: "2.0.0".to_owned(),
        uploaded: "1.0.0".to_owned(),
    }));
    let (service, _timezone) = service_with(backend, Arc::new(RefreshCounter::default())).await;

    let error = service
        .install_widget_package(archive(b""))
        .await
        .expect_err("BUG: scripted downgrade");

    assert!(matches!(
        error,
        SystemUpgradeError::WidgetDowngradeRefused(_)
    ));
}

#[tokio::test]
async fn a_folder_an_installed_widget_holds_is_reported_as_taken() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::FolderTaken {
        uploaded: "acme/clock".into(),
        installed: FolderClash::FlatWidget("acme".to_owned()),
    }));
    let (service, _timezone) = service_with(backend, Arc::new(RefreshCounter::default())).await;

    let error = service
        .install_widget_package(archive(b""))
        .await
        .expect_err("BUG: scripted folder clash");

    assert!(
        matches!(&error, SystemUpgradeError::WidgetFolderTaken(_)),
        "{error}"
    );
}

#[tokio::test]
async fn a_uid_an_installed_widget_holds_is_reported_as_taken() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::UidTaken {
        uploaded: "clock".into(),
        installed: "acme/clock".into(),
    }));
    let (service, _timezone) = service_with(backend, Arc::new(RefreshCounter::default())).await;

    let error = service
        .install_widget_package(archive(b""))
        .await
        .expect_err("BUG: scripted uid clash");

    assert!(
        matches!(&error, SystemUpgradeError::WidgetUidTaken(_)),
        "{error}"
    );
}

#[tokio::test]
async fn a_running_upgrade_turns_the_upload_away_before_the_backend() {
    let backend = ScriptedBackend::answering(Ok(installed()));
    let (service, _timezone) =
        service_with(backend.clone(), Arc::new(RefreshCounter::default())).await;
    let _running = service.run_gate.try_lock().expect("BUG: fresh gate");

    let error = service
        .install_widget_package(archive(b"nar"))
        .await
        .expect_err("BUG: a held gate must refuse");

    assert!(matches!(error, SystemUpgradeError::WidgetInstallBusy));
    assert_eq!(backend.uploads(), 0);
}

fn archive(bytes: &'static [u8]) -> bmc_upgrade::widget_package::WidgetUpload {
    bmc_upgrade::widget_package::WidgetUpload::unannounced(Box::pin(std::io::Cursor::new(bytes)))
}

#[tokio::test]
async fn a_stalled_upload_is_the_clients_deadline_not_a_device_fault() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::UploadStalled));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend, lifecycle.clone()).await;

    let error = service
        .install_widget_package(archive(b"half an archi"))
        .await
        .expect_err("BUG: scripted stall");

    assert!(matches!(error, SystemUpgradeError::WidgetUploadStalled));
    assert_eq!(lifecycle.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_dropped_request_does_not_abandon_the_install() {
    let (backend, started, release) = ScriptedBackend::holding(Ok(installed()));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend.clone(), lifecycle.clone()).await;

    let mut request = Box::pin(service.install_widget_package(archive(b"nar")));
    tokio::select! {
        result = &mut request => panic!("BUG: the held install answered early: {result:?}"),
        started = started => started.expect("BUG: the backend reports the start"),
    }
    // What hyper does when the client's connection goes away mid-install.
    drop(request);

    assert!(
        service.run_gate.try_lock().is_err(),
        "the gate stays held while the detached install runs"
    );
    release
        .send(())
        .expect("BUG: the detached install is still waiting");
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.run_gate.try_lock().is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("BUG: the detached install must finish and release the gate");
    assert_eq!(
        lifecycle.0.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the install ran to completion without its request"
    );
}

#[tokio::test]
async fn an_abandoned_upload_is_the_clients_doing_not_a_device_fault() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::UploadAborted));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend, lifecycle.clone()).await;

    let error = service
        .install_widget_package(archive(b"half an archi"))
        .await
        .expect_err("BUG: scripted abort");

    assert!(matches!(error, SystemUpgradeError::WidgetUploadAborted));
    assert_eq!(lifecycle.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_busy_profile_answers_retry_later() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::ProfileBusy));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend, lifecycle.clone()).await;

    let error = service
        .install_widget_package(archive(b"nar"))
        .await
        .expect_err("BUG: scripted busy profile");

    assert!(
        matches!(error, SystemUpgradeError::WidgetInstallBusy),
        "another writer holding the profile is a retry-later, not a device fault: {error}"
    );
}

#[tokio::test]
async fn a_device_fault_keeps_its_detail_out_of_the_answer() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::Store(
        bmc_nix::store::StoreCommandError::Exited {
            command: "--restore",
            status: std::process::ExitStatus::default(),
            stderr: "error: filesystem error: Permission denied [/mnt/data/bmc/staging/x]"
                .to_owned(),
        },
    )));
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(backend, lifecycle.clone()).await;

    let error = service
        .install_widget_package(archive(b"nar"))
        .await
        .expect_err("BUG: scripted device fault");

    assert!(matches!(error, SystemUpgradeError::WidgetInstallFailed));
    let answer = error.to_string();
    assert!(
        !answer.contains("/mnt/data") && !answer.contains("Permission denied"),
        "the uploader gets no device paths or nix output: {answer}"
    );
}

#[tokio::test]
async fn a_crashed_install_keeps_its_panic_out_of_the_answer() {
    let lifecycle = Arc::new(RefreshCounter::default());
    let (service, _timezone) = service_with(ScriptedBackend::crashing(), lifecycle.clone()).await;

    let error = service
        .install_widget_package(archive(b"nar"))
        .await
        .expect_err("BUG: the scripted backend panics");

    assert!(matches!(error, SystemUpgradeError::WidgetInstallFailed));
    let answer = error.to_string();
    assert!(
        !answer.contains("panicked") && !answer.contains("answers one upload"),
        "the uploader gets no panic text: {answer}"
    );
    assert!(
        service.run_gate.try_lock().is_ok(),
        "a crashed install still releases the gate"
    );
}

#[tokio::test]
async fn a_full_store_is_reported_as_the_widget_install_not_an_upgrade() {
    let backend = ScriptedBackend::answering(Err(WidgetInstallError::Install(
        bmc_nix::upgrade::InstallError::NotEnoughSpace {
            free_bytes: 1,
            required_bytes: 2,
            unpacked_bytes: 2,
        },
    )));
    let (service, _timezone) = service_with(backend, Arc::new(RefreshCounter::default())).await;

    let error = service
        .install_widget_package(archive(b"nar"))
        .await
        .expect_err("BUG: scripted full store");

    assert!(matches!(error, SystemUpgradeError::WidgetInstallNoSpace));
    assert!(
        !error.to_string().contains("upgrade"),
        "the uploader installed a widget, not an upgrade: {error}"
    );
}

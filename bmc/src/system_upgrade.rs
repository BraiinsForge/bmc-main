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

mod periodic_gc;

use crate::BmcManager;
use anyhow::anyhow;
use bmc_scheduler::JobScheduler;
use bmc_scheduler::jobs::to_boxed;
use bmc_scheduler::scheduler::{JobConfig, Schedule, Task};
pub(crate) use bmc_upgrade::arbitration::Disruption;
use bmc_upgrade::arbitration::arbitrate;
use bmc_upgrade::autoupgrade::{AutoUpgrade, AutoUpgradeConfig};
use bmc_upgrade::firmware::{FirmwareDownloadError, FirmwareIndex, UpgradeDetail};
use bmc_upgrade::packages::{
    EstimateMode, PackageBackend, PackageGcRequest, PackageProbe, PackageProbeError,
};
pub(crate) use bmc_upgrade::packages::{PackagesPreview, SystemPackageChange};
use bmc_upgrade::upgrader::{
    DownloadState as UpgraderDownloadState, FirmwareUpgradeError, FirmwareUpgrader,
};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{collections::HashMap, sync::LazyLock};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::watch::{self, Receiver};
use tokio::sync::{Mutex, Notify};
use tokio::task;
use tracing::{debug, error, info, warn};

const AUTOUPGRADE_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(30);
const AUTOUPGRADE_RETRY_MAX_ATTEMPTS: u32 = 5;
const AUTOUPGRADE_RETRY_DELAY_COEFF: u32 = 2;

/// Minimum spacing between forwarded intermediate download `Progress`
/// events; without it every written chunk becomes an event on the run
/// channel and the gRPC-web stream.
const UPDATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(300);

/// No overall request timeout: it serves the long firmware image download,
/// which is instead guarded by a per-chunk idle timeout in the downloader.
/// The connect is still bounded so a dead host cannot wedge the run gate
/// before a single byte flows.
pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("BUG: static client builder failed")
});

/// Widget lifecycle control around a system upgrade. Around a disruptive
/// firmware upgrade it stops all running widget processes before the image
/// download, freeing RAM (the image lands on tmpfs) and GPU resources, and
/// respawns them when the upgrade fails (the compositor keeps running
/// throughout so the display stays alive on a retried failure). After a
/// widget-package install it re-scans the widget registry so the
/// newly-installed widget is available without a restart.
#[async_trait::async_trait]
pub(crate) trait WidgetLifecycle: Send + Sync + std::fmt::Debug {
    async fn stop_all_widgets(&self);
    async fn restart_widgets(&self);
    /// Re-scan for widgets so a just-installed widget becomes available
    /// without a restart.
    async fn refresh_widgets(&self);
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SystemUpgradePhase {
    FirmwareDownloading,
    FirmwareVerifying,
    FirmwareApplying,
    PackageRealizing,
    PackageVerifying,
    PackageBuilding,
    PackageActivating,
}

#[derive(Clone, Debug)]
pub(crate) enum UpgradeRunState {
    Phase(SystemUpgradePhase),
    Progress {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Finished,
    Failed(SystemUpgradeError),
}

#[derive(Clone, Debug)]
pub(crate) enum AvailableSystemUpgrade {
    Firmware {
        detail: UpgradeDetail,
        install: Vec<String>,
    },
    Packages {
        merged: bmc_nix::types::MergedIndex,
        install: Vec<String>,
        download_size_bytes: Option<u64>,
        unpacked_size_bytes: Option<u64>,
    },
}

/// Select the upgrade a check offers under its minted id. Firmware wins
/// over packages: the target firmware changes what the servers' index
/// offers, so packages resolve in the new firmware's context and applying
/// them first would be redone — and possibly superseded — once it lands.
fn select_offer(
    firmware: Option<&UpgradeDetail>,
    merged: Option<bmc_nix::types::MergedIndex>,
    packages: Option<&PackagesPreview>,
    install: &[String],
) -> Option<AvailableSystemUpgrade> {
    if let Some(detail) = firmware {
        return Some(AvailableSystemUpgrade::Firmware {
            detail: detail.clone(),
            install: install.to_vec(),
        });
    }
    let preview = packages?;
    let merged = merged.expect("BUG: packages preview present without a merged index");
    Some(AvailableSystemUpgrade::Packages {
        merged,
        install: install.to_vec(),
        download_size_bytes: preview.download_size_bytes,
        unpacked_size_bytes: preview.unpacked_size_bytes,
    })
}

#[derive(Debug)]
pub(crate) struct CheckOutcome {
    pub firmware: Option<UpgradeDetail>,
    pub packages: Option<PackagesPreview>,
    pub upgrade_id: Option<String>,
    pub disruption: Disruption,
}

pub(crate) struct UpgradeRunStream {
    rx: UnboundedReceiver<UpgradeRunState>,
}

impl futures::Stream for UpgradeRunStream {
    type Item = UpgradeRunState;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

fn one_shot(state: UpgradeRunState) -> UpgradeRunStream {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let _ = tx.send(state);
    UpgradeRunStream { rx }
}

fn record_pending_install(
    install: &[String],
    path: &std::path::Path,
) -> Result<(), SystemUpgradeError> {
    let pending = bmc_nix::pending_install::PendingInstall {
        install: install.to_vec(),
    };
    bmc_nix::pending_install::write_pending_install(path, &pending)
        .map_err(|err| SystemUpgradeError::PendingInstallWriteFailed(err.to_string()))
}

/// Best-effort removal of a pending-install handoff after a firmware run that
/// wrote one then failed to apply, so a later unrelated successful firmware
/// upgrade cannot consume it and install widgets nobody requested. A missing
/// file is the normal case when no install was pending, hence `debug`.
fn clear_pending_install(install: &[String], path: &std::path::Path) {
    if install.is_empty() {
        return;
    }
    if let Err(err) = std::fs::remove_file(path) {
        if err.kind() == std::io::ErrorKind::NotFound {
            debug!(path = %path.display(),
                "No pending-install handoff to clear after failed firmware upgrade");
        } else {
            warn!(error = %err, path = %path.display(),
                "Failed to clear pending-install handoff after failed firmware upgrade");
        }
    }
}

/// Restarts the widgets when dropped, unless disarmed. A firmware run stops
/// them before downloading the image (which lands on tmpfs) to free RAM, so
/// every failure path from that point must bring them back; running the
/// restart on drop covers each early return without repeating the call. The
/// success path disarms the guard, since the reboot starts widgets fresh.
struct WidgetRestartGuard {
    widget_lifecycle: Option<Arc<dyn WidgetLifecycle>>,
}

impl WidgetRestartGuard {
    fn new(widget_lifecycle: Arc<dyn WidgetLifecycle>) -> Self {
        Self {
            widget_lifecycle: Some(widget_lifecycle),
        }
    }

    fn disarm(mut self) {
        self.widget_lifecycle = None;
    }
}

impl Drop for WidgetRestartGuard {
    fn drop(&mut self) {
        if let Some(widget_lifecycle) = self.widget_lifecycle.take() {
            // `restart_widgets` is async and `drop` is not, so spawn it.
            task::spawn(async move {
                widget_lifecycle.restart_widgets().await;
            });
        }
    }
}

#[expect(clippy::cast_precision_loss)]
fn bytes_to_mb(bytes: u64) -> f32 {
    bytes as f32 / 1_000_000.0
}

fn led_event(state: &UpgradeRunState) -> Option<SystemUpgradeState> {
    match state {
        UpgradeRunState::Phase(
            SystemUpgradePhase::FirmwareApplying | SystemUpgradePhase::PackageActivating,
        ) => Some(SystemUpgradeState::UpgradeStarted),
        UpgradeRunState::Progress {
            downloaded_bytes,
            total_bytes,
        } => Some(SystemUpgradeState::DownloadProgress {
            downloaded_mb: bytes_to_mb(*downloaded_bytes),
            total_mb: total_bytes.map(bytes_to_mb),
        }),
        UpgradeRunState::Failed(_) => Some(SystemUpgradeState::Failed),
        UpgradeRunState::Finished => Some(SystemUpgradeState::Finished),
        UpgradeRunState::Phase(
            SystemUpgradePhase::FirmwareDownloading
            | SystemUpgradePhase::FirmwareVerifying
            | SystemUpgradePhase::PackageRealizing
            | SystemUpgradePhase::PackageVerifying
            | SystemUpgradePhase::PackageBuilding,
        ) => None,
    }
}

fn forward_led_events(state_service: StateService, mut run: UpgradeRunStream) -> UpgradeRunStream {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    task::spawn(async move {
        while let Some(state) = run.rx.recv().await {
            if let Some(event) = led_event(&state) {
                state_service.notify(event);
            }
            _ = tx.send(state);
        }
    });
    UpgradeRunStream { rx }
}

/// Admits the first download progress event immediately and later ones only
/// after [`UPDATE_PROGRESS_INTERVAL`] since the last admitted one.
#[derive(Debug, Default)]
struct ProgressThrottle {
    last_admitted: Option<Instant>,
}

impl ProgressThrottle {
    fn admit(&mut self, now: Instant) -> bool {
        let admitted = self
            .last_admitted
            .is_none_or(|last| now.duration_since(last) >= UPDATE_PROGRESS_INTERVAL);
        if admitted {
            self.last_admitted = Some(now);
        }
        admitted
    }
}

struct ChannelUpgradeProgress {
    sender: tokio::sync::mpsc::UnboundedSender<UpgradeRunState>,
    state_service: StateService,
    throttle: std::sync::Mutex<ProgressThrottle>,
}

impl ChannelUpgradeProgress {
    fn new(
        sender: tokio::sync::mpsc::UnboundedSender<UpgradeRunState>,
        state_service: StateService,
    ) -> Self {
        Self {
            sender,
            state_service,
            throttle: std::sync::Mutex::new(ProgressThrottle::default()),
        }
    }
}

impl bmc_nix::upgrade::UpgradeProgress for ChannelUpgradeProgress {
    fn on_phase(&self, phase: bmc_nix::upgrade::UpgradePhase) {
        let phase = match phase {
            bmc_nix::upgrade::UpgradePhase::Realizing => SystemUpgradePhase::PackageRealizing,
            bmc_nix::upgrade::UpgradePhase::Verifying => SystemUpgradePhase::PackageVerifying,
            bmc_nix::upgrade::UpgradePhase::Building => SystemUpgradePhase::PackageBuilding,
            bmc_nix::upgrade::UpgradePhase::Activating => SystemUpgradePhase::PackageActivating,
            bmc_nix::upgrade::UpgradePhase::Cleaning
            | bmc_nix::upgrade::UpgradePhase::CollectingGarbage(_) => return,
        };
        _ = self.sender.send(UpgradeRunState::Phase(phase));
    }

    fn on_realization_started(&self, _total_paths: usize) {}

    fn on_realization_finished(&self) {
        self.state_service
            .notify(SystemUpgradeState::DownloadFinished {
                hash: None,
                total_mb: None,
            });
    }

    fn on_download_status(&self, snapshot: &bmc_nix::store::progress::DownloadSnapshot) {
        if !self
            .throttle
            .lock()
            .expect("BUG: progress throttle mutex poisoned")
            .admit(Instant::now())
        {
            return;
        }
        _ = self.sender.send(UpgradeRunState::Progress {
            downloaded_bytes: snapshot.downloaded_bytes,
            total_bytes: snapshot.total_bytes,
        });
    }

    fn on_gc_deleted(&self, _deleted_paths: usize) {}

    fn on_gc_finished(&self, _deleted_paths: usize, _freed_bytes: Option<u64>) {}
}

async fn claim_upgrade(
    run_gate: &Arc<Mutex<()>>,
    system_upgrades: &Mutex<HashMap<String, AvailableSystemUpgrade>>,
    upgrade_id: &str,
) -> Result<(tokio::sync::OwnedMutexGuard<()>, AvailableSystemUpgrade), UpgradeRunStream> {
    let Ok(gate) = Arc::clone(run_gate).try_lock_owned() else {
        warn!("Upgrade already in progress");
        return Err(one_shot(UpgradeRunState::Failed(
            SystemUpgradeError::UpgradeInProgress,
        )));
    };

    let Some(upgrade) = system_upgrades.lock().await.remove(upgrade_id) else {
        warn!(upgrade_id, "Upgrade id is unknown or already consumed");
        return Err(one_shot(UpgradeRunState::Failed(
            SystemUpgradeError::UpgradeExpired,
        )));
    };

    Ok((gate, upgrade))
}

/// Free space the preflight requires beyond the dry-run estimate: the
/// estimate is parsed from nix's one-decimal summary, so its rounding error
/// alone is proportional to the printed magnitude, and the store also grows
/// bookkeeping (database, temp files) the estimate does not count.
fn required_with_headroom(unpacked_bytes: u64) -> u64 {
    #[expect(clippy::integer_division, reason = "a byte of headroom is immaterial")]
    unpacked_bytes.saturating_add(unpacked_bytes / 10)
}

async fn automatic_gc_preflight(
    gate: tokio::sync::OwnedMutexGuard<()>,
    package_backend: &Arc<dyn PackageBackend>,
    unpacked_size_bytes: Option<u64>,
) -> Result<tokio::sync::OwnedMutexGuard<()>, UpgradeRunStream> {
    // Unconditional: an upgrade needs the space whether or not a periodic
    // collection ran recently, so the periodic job's schedule and its
    // configured opt-out play no part here. Best-effort: a failed collection
    // is not what decides the upgrade — the free-space check below is.
    let request = PackageGcRequest {
        on_busy: bmc_nix::gc::OnBusy::Wait,
        sweep: bmc_nix::gc::Sweep::Always,
    };
    match package_backend.gc(request).await {
        Ok(outcome) => debug!(
            ?outcome,
            "Garbage collection before automatic upgrade finished"
        ),
        Err(err) => {
            warn!(error = %err, "Garbage collection before automatic upgrade failed; continuing");
        }
    }

    // Fail only on a certain "will not fit". Either side missing — the probe
    // estimate timed out, or free space cannot be measured — leaves nothing
    // to compare, and the realization itself fails loudly when space runs out.
    let Some(unpacked_bytes) = unpacked_size_bytes else {
        return Ok(gate);
    };
    let free_bytes = match package_backend.store_free_bytes() {
        Ok(free_bytes) => free_bytes,
        Err(err) => {
            warn!(error = %err, "Cannot measure free store space; continuing");
            return Ok(gate);
        }
    };

    let required_bytes = required_with_headroom(unpacked_bytes);
    if free_bytes < required_bytes {
        error!(
            free_bytes,
            required_bytes, unpacked_bytes, "Not enough store space for the automatic upgrade"
        );
        return Err(one_shot(UpgradeRunState::Failed(
            SystemUpgradeError::NotEnoughSpace,
        )));
    }
    Ok(gate)
}

fn spawn_packages_run(
    gate: tokio::sync::OwnedMutexGuard<()>,
    package_backend: Arc<dyn PackageBackend>,
    widget_lifecycle: Arc<dyn WidgetLifecycle>,
    merged: bmc_nix::types::MergedIndex,
    install: Vec<String>,
    download_size_bytes: Option<u64>,
    state_service: StateService,
) -> UpgradeRunStream {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    task::spawn(async move {
        let _gate = gate;
        state_service.notify(SystemUpgradeState::DownloadStarted {
            total_mb: download_size_bytes.map(bytes_to_mb),
        });
        let adapter: Arc<dyn bmc_nix::upgrade::UpgradeProgress> =
            Arc::new(ChannelUpgradeProgress::new(tx.clone(), state_service));
        match package_backend.apply(merged, install, adapter).await {
            Ok(()) => {
                info!("Package upgrade finished");
                // Re-scan so a just-installed widget is available without a
                // restart; the install already wrote it into the widgets path.
                // Whether each requested widget actually became placeable is the
                // FE's concern via GetAvailableWidgets — the install itself stuck.
                widget_lifecycle.refresh_widgets().await;
                _ = tx.send(UpgradeRunState::Finished);
            }
            Err(err) => {
                error!(error = %err, "Package upgrade failed");
                _ = tx.send(UpgradeRunState::Failed(
                    SystemUpgradeError::PackageUpgradeFailed(err.to_string()),
                ));
            }
        }
    });
    UpgradeRunStream { rx }
}

#[derive(Clone, Debug)]
pub(crate) struct StateService {
    sender: Arc<watch::Sender<Option<SystemUpgradeState>>>,
}
impl StateService {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(None);

        Self {
            sender: Arc::new(sender),
        }
    }

    fn notify(&self, value: SystemUpgradeState) {
        let value = Some(value);

        self.sender.send_if_modified(|current| {
            if *current != value {
                *current = value;
                return true;
            }
            false
        });
    }

    pub(crate) fn subscribe(&self) -> Receiver<Option<SystemUpgradeState>> {
        self.sender.subscribe()
    }
}

#[derive(Debug)]
pub(crate) struct SystemUpgradeService<T: FirmwareIndex, U: BmcManager> {
    state_service: StateService,
    firmware_upgrader: Arc<Mutex<FirmwareUpgrader<T>>>,
    bmc_manager: Arc<U>,
    scheduler: JobScheduler,
    autoupgrade: Arc<AutoUpgrade>,
    run_gate: Arc<Mutex<()>>,
    upgrade_id_seq: Arc<AtomicUsize>,
    system_upgrades: Arc<Mutex<HashMap<String, AvailableSystemUpgrade>>>,
    package_backend: Arc<dyn PackageBackend>,
    widget_lifecycle: Arc<dyn WidgetLifecycle>,
    pending_install_path: PathBuf,
}

impl<T, U> Clone for SystemUpgradeService<T, U>
where
    T: FirmwareIndex,
    U: BmcManager,
{
    fn clone(&self) -> Self {
        Self {
            state_service: self.state_service.clone(),
            firmware_upgrader: self.firmware_upgrader.clone(),
            bmc_manager: self.bmc_manager.clone(),
            scheduler: self.scheduler.clone(),
            autoupgrade: self.autoupgrade.clone(),
            run_gate: self.run_gate.clone(),
            upgrade_id_seq: self.upgrade_id_seq.clone(),
            system_upgrades: self.system_upgrades.clone(),
            package_backend: self.package_backend.clone(),
            widget_lifecycle: self.widget_lifecycle.clone(),
            pending_install_path: self.pending_install_path.clone(),
        }
    }
}

impl<T: FirmwareIndex, U: BmcManager> SystemUpgradeService<T, U> {
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        firmware_index: T,
        upgrade_image_path: &PathBuf,
        bmc_manager: Arc<U>,
        state_service: StateService,
        scheduler: JobScheduler,
        package_backend: Arc<dyn PackageBackend>,
        widget_lifecycle: Arc<dyn WidgetLifecycle>,
        pending_install_path: PathBuf,
    ) -> Self {
        let autoupgrade = AutoUpgrade::new(Notify::new(), Instant::now());
        let firmware_upgrader = FirmwareUpgrader::new(
            firmware_index,
            upgrade_image_path.to_owned(),
            CLIENT.clone(),
        );
        Self {
            state_service,
            firmware_upgrader: Arc::new(Mutex::new(firmware_upgrader)),
            bmc_manager,
            scheduler,
            autoupgrade: Arc::new(autoupgrade),
            run_gate: Arc::new(Mutex::new(())),
            upgrade_id_seq: Arc::new(AtomicUsize::new(0)),
            system_upgrades: Arc::new(Mutex::new(HashMap::new())),
            package_backend,
            widget_lifecycle,
            pending_install_path,
        }
    }

    pub(crate) async fn check_for_upgrade(
        &self,
        install: Vec<String>,
    ) -> Result<CheckOutcome, SystemUpgradeError> {
        let _gate = self
            .run_gate
            .try_lock()
            .map_err(|_| SystemUpgradeError::UpgradeInProgress)?;

        self.system_upgrades.lock().await.clear();

        let firmware = self.probe_firmware().await?;

        let probe = self
            .package_backend
            .probe(
                if firmware.is_some() {
                    EstimateMode::Skip
                } else {
                    EstimateMode::Estimate
                },
                &install,
            )
            .await;

        let packages = match probe {
            PackageProbe::Available(merged, preview) => Some((merged, preview)),
            PackageProbe::UpToDate => None,
            PackageProbe::Failed(err) => return Err(SystemUpgradeError::PackageCheckFailed(err)),
        };

        let disruption = arbitrate(firmware.as_ref(), packages.as_ref().map(|(_, p)| p));

        let (merged, packages) = match packages {
            Some((merged, preview)) => (Some(merged), Some(preview)),
            None => (None, None),
        };

        let upgrade_id = if let Some(upgrade) =
            select_offer(firmware.as_ref(), merged, packages.as_ref(), &install)
        {
            let upgrade_id = format!(
                "upgrade-{}",
                self.upgrade_id_seq.fetch_add(1, Ordering::Relaxed)
            );
            self.system_upgrades
                .lock()
                .await
                .insert(upgrade_id.clone(), upgrade);
            Some(upgrade_id)
        } else {
            None
        };

        Ok(CheckOutcome {
            firmware,
            packages,
            upgrade_id,
            disruption,
        })
    }

    pub(crate) async fn list_installable_widgets(
        &self,
    ) -> Result<Vec<bmc_upgrade::packages::InstallableWidget>, SystemUpgradeError> {
        self.package_backend
            .list_installable_widgets()
            .await
            .map_err(SystemUpgradeError::PackageCheckFailed)
    }

    pub(crate) async fn start_upgrade(&self, upgrade_id: String) -> UpgradeRunStream {
        let (gate, upgrade) =
            match claim_upgrade(&self.run_gate, &self.system_upgrades, &upgrade_id).await {
                Ok(claimed) => claimed,
                Err(stream) => return stream,
            };

        self.dispatch_claimed_upgrade(gate, upgrade)
    }

    fn dispatch_claimed_upgrade(
        &self,
        gate: tokio::sync::OwnedMutexGuard<()>,
        upgrade: AvailableSystemUpgrade,
    ) -> UpgradeRunStream {
        let run = match upgrade {
            AvailableSystemUpgrade::Firmware { detail, install } => {
                self.spawn_firmware_run(gate, detail, install)
            }
            AvailableSystemUpgrade::Packages {
                merged,
                install,
                download_size_bytes,
                ..
            } => spawn_packages_run(
                gate,
                Arc::clone(&self.package_backend),
                Arc::clone(&self.widget_lifecycle),
                merged,
                install,
                download_size_bytes,
                self.state_service.clone(),
            ),
        };
        forward_led_events(self.state_service.clone(), run)
    }

    async fn start_automatic_upgrade(&self, upgrade_id: String) -> UpgradeRunStream {
        let (gate, upgrade) =
            match claim_upgrade(&self.run_gate, &self.system_upgrades, &upgrade_id).await {
                Ok(claimed) => claimed,
                Err(stream) => return stream,
            };
        // A firmware image downloads to tmpfs, not the store, so only the
        // packages branch has an estimate to check the store against.
        let unpacked_size_bytes = match &upgrade {
            AvailableSystemUpgrade::Firmware { .. } => None,
            AvailableSystemUpgrade::Packages {
                unpacked_size_bytes,
                ..
            } => *unpacked_size_bytes,
        };
        let gate =
            match automatic_gc_preflight(gate, &self.package_backend, unpacked_size_bytes).await {
                Ok(gate) => gate,
                Err(stream) => return stream,
            };
        self.dispatch_claimed_upgrade(gate, upgrade)
    }

    #[expect(clippy::too_many_lines)]
    fn spawn_firmware_run(
        &self,
        gate: tokio::sync::OwnedMutexGuard<()>,
        detail: UpgradeDetail,
        install: Vec<String>,
    ) -> UpgradeRunStream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let firmware_upgrader = Arc::clone(&self.firmware_upgrader);
        let bmc_manager = Arc::clone(&self.bmc_manager);
        let widget_lifecycle = Arc::clone(&self.widget_lifecycle);
        let state_service = self.state_service.clone();
        let pending_install_path = self.pending_install_path.clone();
        task::spawn(async move {
            let _gate = gate;
            let upgrader = firmware_upgrader.lock().await;
            let release = &detail.latest_release;
            let total_bytes = release.file_size as u64;

            _ = tx.send(UpgradeRunState::Phase(
                SystemUpgradePhase::FirmwareDownloading,
            ));
            state_service.notify(SystemUpgradeState::DownloadStarted {
                total_mb: Some(bytes_to_mb(total_bytes)),
            });
            // Stop widgets before the download starts: the image lands on tmpfs
            // (RAM), so freeing their memory first leaves room for it. The guard
            // restarts them on any failure return below.
            widget_lifecycle.stop_all_widgets().await;
            let widget_guard = WidgetRestartGuard::new(widget_lifecycle);
            let mut download_rx =
                upgrader.download_firmware(release.url.clone(), release.hash.clone(), total_bytes);
            let mut download_finished = false;
            let mut throttle = ProgressThrottle::default();
            while let Some(event) = download_rx.recv().await {
                match event {
                    UpgraderDownloadState::Progress { downloaded_mb, .. } => {
                        if !throttle.admit(Instant::now()) {
                            continue;
                        }
                        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let downloaded_bytes = (f64::from(downloaded_mb) * 1_000_000.0) as u64;
                        _ = tx.send(UpgradeRunState::Progress {
                            downloaded_bytes,
                            total_bytes: Some(total_bytes),
                        });
                    }
                    UpgraderDownloadState::Finished { hash } => {
                        state_service.notify(SystemUpgradeState::DownloadFinished {
                            hash: Some(hash),
                            total_mb: Some(bytes_to_mb(total_bytes)),
                        });
                        download_finished = true;
                    }
                    UpgraderDownloadState::Failed(err) => {
                        error!(error = %err, "Firmware download failed");
                        _ = tx.send(UpgradeRunState::Failed(err.into()));
                        return;
                    }
                }
            }
            if !download_finished {
                error!("Firmware download ended without completing");
                _ = tx.send(UpgradeRunState::Failed(
                    SystemUpgradeError::FailedToDownload("download ended unexpectedly".to_owned()),
                ));
                return;
            }

            _ = tx.send(UpgradeRunState::Phase(
                SystemUpgradePhase::FirmwareVerifying,
            ));
            if let Err(err) = upgrader.verify_firmware(&release.hash).await {
                warn!(error = %err, "Failed to verify downloaded firmware");
                _ = tx.send(UpgradeRunState::Failed(err.into()));
                return;
            }

            // Written after verify so an earlier download/verify failure can
            // never leave a stale handoff behind. A successful upgrade
            // writes-then-consumes it within the `bmc_manager.upgrade` call
            // below; a write failure here aborts and the guard restarts widgets.
            if !install.is_empty()
                && let Err(err) = record_pending_install(&install, &pending_install_path)
            {
                _ = tx.send(UpgradeRunState::Failed(err));
                return;
            }

            let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let adapter = ChannelUpgradeProgress::new(tx.clone(), state_service);
            let reader = task::spawn(async move {
                while let Some(line) = line_rx.recv().await {
                    bmc_nix::progress::feed_line(&line, &adapter);
                }
            });

            let result = bmc_manager
                .upgrade(true, upgrader.upgrade_image_path(), Some(line_tx))
                .await;
            // `upgrade` consumed the only sender, so the reader drains its
            // backlog and exits; awaiting it keeps every `Package*` event
            // ahead of the terminal Phase/Failed event.
            _ = reader.await;
            match result {
                Ok(()) => {
                    // Handoff accepted; sysupgrade has staged the image and the
                    // reboot follows within the shutdown-grace window. The
                    // reboot starts widgets fresh, so cancel the restart.
                    info!("Firmware upgrade handoff accepted");
                    widget_guard.disarm();
                    _ = tx.send(UpgradeRunState::Phase(SystemUpgradePhase::FirmwareApplying));
                    // Drop the sender to end the stream with a clean OK trailer,
                    // then park so the run gate stays held until the reboot.
                    drop(tx);
                    std::future::pending::<()>().await;
                }
                Err(err) => {
                    // `widget_guard` restarts the widgets when this arm returns.
                    error!(error = %err, "Firmware upgrade failed");
                    // The only failure point after the handoff was written.
                    clear_pending_install(&install, &pending_install_path);
                    let failure = match err {
                        crate::UpgradeError::InvalidImage => SystemUpgradeError::InvalidImage,
                        crate::UpgradeError::Failed(_) => SystemUpgradeError::UpgradeFailed,
                    };
                    _ = tx.send(UpgradeRunState::Failed(failure));
                }
            }
        });
        UpgradeRunStream { rx }
    }

    async fn probe_firmware(&self) -> Result<Option<UpgradeDetail>, SystemUpgradeError> {
        let Ok(upgrader) = self.firmware_upgrader.try_lock() else {
            return Err(SystemUpgradeError::UpgradeInProgress);
        };

        let platform = self.bmc_manager.platform();
        let Some(version) = self.bmc_manager.version().await else {
            error!("Failed to detect current firmware version");
            return Err(SystemUpgradeError::FailedToDetectCurrentVersion);
        };

        info!(platform = %platform, version = %version.full, "Checking for firmware upgrade");

        let Some(release_info) = upgrader
            .check_for_upgrade(platform, version.full)
            .await
            .inspect_err(|err| error!(error = %err, platform = %platform, "Failed to check for firmware upgrade"))
            .map_err(SystemUpgradeError::UnableToCheckForUpgrade)?
        else {
            info!("No firmware upgrade available");
            return Ok(None);
        };

        info!(
            hash = %release_info.latest_release.hash,
            version = %release_info.latest_release.version,
            file_size = release_info.latest_release.file_size,
            "Firmware upgrade available"
        );

        Ok(Some(release_info))
    }

    pub async fn autoupgrade_init(&self, config: AutoUpgradeConfig) {
        if let Err(err) = self.autoupgrade_reschedule(config).await {
            error!(?err, "Failed to reschedule autoupgrade");
        }

        let self_clone = self.clone();
        let notifier = self.autoupgrade.notifier.clone();

        tokio::task::spawn(async move {
            loop {
                notifier.notified().await;
                match self_clone.autoupgrade_trigger().await {
                    Ok(()) => {}
                    Err(err) if err.is_retriable() => {
                        warn!(error = %err, "Auto-upgrade failed with retriable error, starting backoff retries");
                        Self::retry_autoupgrade_with_backoff(&self_clone).await;
                    }
                    Err(err) => {
                        error!(error = %err, "Auto-upgrade failed with non-retriable error");
                    }
                }
            }
        });
    }

    pub(crate) async fn gc_init(&self, started: tokio::time::Instant, gc_config_path: PathBuf) {
        let gc = Arc::new(periodic_gc::PeriodicGc::new(
            started,
            Arc::clone(&self.run_gate),
            Arc::clone(&self.package_backend),
            gc_config_path,
        ));
        if let Err(err) = gc.schedule(&self.scheduler).await {
            error!(error = %err, "Failed to schedule periodic garbage collection");
        }
    }

    async fn retry_autoupgrade_with_backoff(service: &Self) {
        let mut delay = AUTOUPGRADE_RETRY_INITIAL_DELAY;

        for attempt in 1..=AUTOUPGRADE_RETRY_MAX_ATTEMPTS {
            info!(
                attempt,
                delay_secs = delay.as_secs(),
                "Scheduling auto-upgrade retry"
            );

            tokio::time::sleep(delay).await;

            match service.autoupgrade_trigger().await {
                Ok(()) => {
                    info!(attempt, "Auto-upgrade retry succeeded");
                    return;
                }
                Err(err) if err.is_retriable() => {
                    warn!(attempt, error = %err, "Auto-upgrade retry failed, will retry");
                }
                Err(err) => {
                    error!(attempt, error = %err, "Auto-upgrade retry failed with non-retriable error, stopping retries");
                    return;
                }
            }

            delay *= AUTOUPGRADE_RETRY_DELAY_COEFF;
        }

        warn!("Auto-upgrade retries exhausted, will wait for next scheduled trigger");
    }

    async fn autoupgrade_trigger(&self) -> Result<(), SystemUpgradeError> {
        debug!("Auto-upgrade triggered");
        let outcome = self.check_for_upgrade(Vec::new()).await?;

        let Some(upgrade_id) = outcome.upgrade_id else {
            debug!("No upgrade available");
            return Ok(());
        };

        info!(upgrade_id, "Auto-upgrade found an upgrade, starting");
        let mut run = self.start_automatic_upgrade(upgrade_id).await;
        while let Some(state) = run.next().await {
            match state {
                UpgradeRunState::Phase(phase) => debug!(?phase, "Auto-upgrade phase"),
                UpgradeRunState::Progress {
                    downloaded_bytes,
                    total_bytes,
                } => debug!(downloaded_bytes, total_bytes, "Auto-upgrade progress"),
                UpgradeRunState::Finished => info!("Auto-upgrade finished"),
                UpgradeRunState::Failed(err) => {
                    error!(error = %err, "Auto-upgrade run failed");
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub async fn autoupgrade_reschedule(
        &self,
        new_config: AutoUpgradeConfig,
    ) -> anyhow::Result<()> {
        // First, cancel existing jobs if there are any
        self.scheduler
            .cancel_jobs(AutoUpgrade::AUTOUPGRADE_SOURCE_NAME.to_owned())
            .await;

        if new_config.enabled {
            let Some(cron) = new_config.cron else {
                return Err(anyhow!("Missing Cron in AutoUpgrade config"));
            };
            let schedule = Schedule::Cron(cron);
            let task = Task::Async(to_boxed(self.autoupgrade.task.clone()));
            let job_config = JobConfig::new(AutoUpgrade::AUTOUPGRADE_SOURCE_NAME);

            self.scheduler.schedule(schedule, task, job_config).await?;
        }

        Ok(())
    }

    pub async fn get_autoupgrade_next_run(&self) -> Option<DateTime<Utc>> {
        let Ok(jobs) = self.scheduler.jobs().await else {
            return None;
        };
        if let Some(autoupgrade_job) = jobs
            .into_iter()
            .find(|job| job.source == AutoUpgrade::AUTOUPGRADE_SOURCE_NAME)
        {
            autoupgrade_job.next_tick
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SystemUpgradeState {
    DownloadStarted {
        total_mb: Option<f32>,
    },
    DownloadProgress {
        downloaded_mb: f32,
        total_mb: Option<f32>,
    },
    DownloadFinished {
        hash: Option<String>,
        total_mb: Option<f32>,
    },
    UpgradeStarted,
    Finished,
    Failed,
}

impl SystemUpgradeState {
    /// Whether a device restart must be declined in this state. Only the
    /// transient states block: an active download would be wasted and an
    /// active upgrade (flashing) is genuinely dangerous. `DownloadFinished`,
    /// `Finished`, and `Failed` are resting states that persist indefinitely,
    /// and rebooting outside an active operation is safe.
    pub(crate) fn blocks_restart(&self) -> bool {
        match self {
            SystemUpgradeState::DownloadStarted { .. }
            | SystemUpgradeState::DownloadProgress { .. }
            | SystemUpgradeState::UpgradeStarted => true,
            SystemUpgradeState::DownloadFinished { .. }
            | SystemUpgradeState::Finished
            | SystemUpgradeState::Failed => false,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub(crate) enum SystemUpgradeError {
    #[error("Failed to detect current version")]
    FailedToDetectCurrentVersion,
    #[error("Firmware image checksum mismatch. Expected {expected}, downloaded {actual}")]
    DownloadedImageHashMismatch { expected: String, actual: String },
    #[error("Failed to verify downloaded firmware")]
    VerifyFailed,
    #[error("Not enough space on disk to perform upgrade")]
    NotEnoughSpace,
    #[error("System upgrade is in progress")]
    UpgradeInProgress,
    #[error("Failed to download firmware, {0}")]
    FailedToDownload(String),
    #[error("Checking upgrade failed, {0}")]
    UnableToCheckForUpgrade(#[from] FirmwareDownloadError),
    #[error("Upgrade failed")]
    UpgradeFailed,
    #[error("Invalid firmware image")]
    InvalidImage,
    #[error("Upgrade id is unknown or already consumed")]
    UpgradeExpired,
    #[error("Package upgrade failed: {0}")]
    PackageUpgradeFailed(String),
    #[error("Failed to record the pending widget install: {0}")]
    PendingInstallWriteFailed(String),
    #[error("Cannot check for upgrade: {0}.")]
    PackageCheckFailed(PackageProbeError),
}

impl From<FirmwareUpgradeError> for SystemUpgradeError {
    fn from(err: FirmwareUpgradeError) -> Self {
        match err {
            FirmwareUpgradeError::DownloadedImageHashMismatch { expected, actual } => {
                Self::DownloadedImageHashMismatch { expected, actual }
            }
            FirmwareUpgradeError::VerifyFailed => Self::VerifyFailed,
            FirmwareUpgradeError::NotEnoughSpace(_) => Self::NotEnoughSpace,
            FirmwareUpgradeError::FailedToDownload(msg) => Self::FailedToDownload(msg),
            FirmwareUpgradeError::CheckFailed(err) => Self::UnableToCheckForUpgrade(err),
        }
    }
}

impl SystemUpgradeError {
    fn is_retriable(&self) -> bool {
        if let Self::PackageCheckFailed(err) = self {
            return err.is_transient();
        }
        // `UpgradeInProgress` and `UpgradeExpired` are transient collisions
        // with another run (e.g. a UI-driven check during the scheduled slot
        // evicting the id the autoupgrade just minted): the autoupgrade must
        // back off and retry, not wait for the next cron slot.
        matches!(
            self,
            Self::UnableToCheckForUpgrade(
                FirmwareDownloadError::IndexDownloadFailed
                    | FirmwareDownloadError::FetchUpgradeDetails
            ) | Self::UpgradeInProgress
                | Self::UpgradeExpired
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_upgrade::packages::{PackageGcError, PackageGcOutcome};
    use futures::StreamExt;

    fn test_upgrade_detail() -> UpgradeDetail {
        UpgradeDetail {
            latest_release: bmc_upgrade::firmware::UpgradeMetadata::new(
                "hash".to_owned(),
                "1.0.0".to_owned(),
                chrono::NaiveDate::default(),
                "description".to_owned(),
                "http://x".to_owned(),
                1,
            ),
            previous_releases: Vec::new(),
        }
    }

    #[tokio::test]
    async fn start_upgrade_unknown_id_expires_and_frees_gate() {
        let run_gate = Arc::new(Mutex::new(()));
        let system_upgrades = Mutex::new(HashMap::new());

        let claim = claim_upgrade(&run_gate, &system_upgrades, "unknown-id").await;

        let Err(mut stream) = claim else {
            panic!("BUG: unknown id must not claim an upgrade");
        };
        assert!(matches!(
            stream.next().await,
            Some(UpgradeRunState::Failed(SystemUpgradeError::UpgradeExpired))
        ));
        assert!(stream.next().await.is_none());
        assert!(run_gate.try_lock().is_ok());
    }

    #[tokio::test]
    async fn start_upgrade_while_gate_held_keeps_id_and_reports_in_progress() {
        let run_gate = Arc::new(Mutex::new(()));
        let system_upgrades = Mutex::new(HashMap::new());
        system_upgrades.lock().await.insert(
            "upgrade-0".to_owned(),
            AvailableSystemUpgrade::Firmware {
                detail: test_upgrade_detail(),
                install: Vec::new(),
            },
        );

        let guard = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");

        let claim = claim_upgrade(&run_gate, &system_upgrades, "upgrade-0").await;

        let Err(mut stream) = claim else {
            panic!("BUG: held gate must not claim an upgrade");
        };
        assert!(matches!(
            stream.next().await,
            Some(UpgradeRunState::Failed(
                SystemUpgradeError::UpgradeInProgress
            ))
        ));
        assert!(system_upgrades.lock().await.contains_key("upgrade-0"));

        drop(guard);
        assert!(run_gate.try_lock().is_ok());
    }

    #[tokio::test]
    async fn claimed_upgrade_id_is_single_use() {
        let run_gate = Arc::new(Mutex::new(()));
        let system_upgrades = Mutex::new(HashMap::new());
        system_upgrades.lock().await.insert(
            "upgrade-0".to_owned(),
            AvailableSystemUpgrade::Firmware {
                detail: test_upgrade_detail(),
                install: Vec::new(),
            },
        );

        let claim = claim_upgrade(&run_gate, &system_upgrades, "upgrade-0").await;
        let Ok((gate, _upgrade)) = claim else {
            panic!("BUG: a fresh id with a free gate must claim");
        };
        drop(gate);

        // The first claim consumed the id: starting the same id again
        // must expire even though the gate is free again.
        let claim = claim_upgrade(&run_gate, &system_upgrades, "upgrade-0").await;
        let Err(mut stream) = claim else {
            panic!("BUG: a consumed id must not claim again");
        };
        assert!(matches!(
            stream.next().await,
            Some(UpgradeRunState::Failed(SystemUpgradeError::UpgradeExpired))
        ));
    }

    #[tokio::test]
    async fn expired_upgrade_after_racing_claim_is_retriable() {
        let run_gate = Arc::new(Mutex::new(()));
        let system_upgrades = Mutex::new(HashMap::new());
        system_upgrades.lock().await.insert(
            "upgrade-0".to_owned(),
            AvailableSystemUpgrade::Firmware {
                detail: test_upgrade_detail(),
                install: Vec::new(),
            },
        );

        // A racing UI-driven start consumes the id the autoupgrade just
        // minted; the autoupgrade's own claim then expires.
        let winner = claim_upgrade(&run_gate, &system_upgrades, "upgrade-0").await;
        let Ok((gate, _upgrade)) = winner else {
            panic!("BUG: the racing claim must win the fresh id");
        };
        drop(gate);

        let Err(mut stream) = claim_upgrade(&run_gate, &system_upgrades, "upgrade-0").await else {
            panic!("BUG: the evicted id must not claim");
        };
        let Some(UpgradeRunState::Failed(err)) = stream.next().await else {
            panic!("BUG: the evicted id must fail with an error");
        };
        assert!(
            matches!(err, SystemUpgradeError::UpgradeExpired),
            "expected UpgradeExpired, got {err:?}"
        );
        assert!(
            err.is_retriable(),
            "a racing claim evicting the autoupgrade's id must be retriable"
        );
    }

    fn empty_merged_index() -> bmc_nix::types::MergedIndex {
        bmc_nix::types::MergedIndex {
            packages: Vec::new(),
            by_name: std::collections::BTreeMap::new(),
        }
    }

    fn test_packages_preview() -> PackagesPreview {
        PackagesPreview {
            changes: Vec::new(),
            download_size_bytes: Some(42),
            unpacked_size_bytes: Some(84),
            bmc_version: None,
            bmc_changelog: None,
        }
    }

    #[test]
    fn offer_prefers_firmware_over_packages() {
        let detail = test_upgrade_detail();

        let offer = select_offer(
            Some(&detail),
            Some(empty_merged_index()),
            Some(&test_packages_preview()),
            &[],
        )
        .expect("BUG: firmware present must mint an offer");

        assert!(
            matches!(offer, AvailableSystemUpgrade::Firmware { .. }),
            "a pending firmware upgrade must win over packages"
        );
    }

    #[test]
    fn firmware_wins_even_with_pending_install() {
        let firmware = test_upgrade_detail();
        let preview = test_packages_preview();
        let offer = select_offer(
            Some(&firmware),
            Some(empty_merged_index()),
            Some(&preview),
            &["widget-weather".to_owned()],
        );
        assert!(matches!(
            offer,
            Some(AvailableSystemUpgrade::Firmware { .. })
        ));
    }

    #[test]
    fn offer_falls_back_to_packages_without_firmware() {
        let offer = select_offer(
            None,
            Some(empty_merged_index()),
            Some(&test_packages_preview()),
            &[],
        )
        .expect("BUG: available packages must mint an offer");

        assert!(matches!(
            offer,
            AvailableSystemUpgrade::Packages {
                download_size_bytes: Some(42),
                ..
            }
        ));
        assert!(select_offer(None, None, None, &[]).is_none());
    }

    #[derive(Debug)]
    struct StubBackend;

    #[async_trait::async_trait]
    impl PackageBackend for StubBackend {
        async fn gc(&self, _request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
            Ok(PackageGcOutcome::Collected)
        }

        async fn probe(&self, _estimate: EstimateMode, _install: &[String]) -> PackageProbe {
            PackageProbe::UpToDate
        }

        async fn apply(
            &self,
            _merged: bmc_nix::types::MergedIndex,
            _install: Vec<String>,
            _progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
        ) -> Result<(), bmc_upgrade::packages::ApplyError> {
            Ok(())
        }

        async fn list_installable_widgets(
            &self,
        ) -> Result<
            Vec<bmc_upgrade::packages::InstallableWidget>,
            bmc_upgrade::packages::PackageProbeError,
        > {
            Ok(Vec::new())
        }

        fn store_free_bytes(&self) -> std::io::Result<u64> {
            Ok(u64::MAX)
        }
    }

    #[derive(Debug)]
    struct FailingBackend;

    #[async_trait::async_trait]
    impl PackageBackend for FailingBackend {
        async fn gc(&self, _request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
            Ok(PackageGcOutcome::Collected)
        }

        async fn probe(&self, _estimate: EstimateMode, _install: &[String]) -> PackageProbe {
            PackageProbe::UpToDate
        }

        async fn apply(
            &self,
            _merged: bmc_nix::types::MergedIndex,
            _install: Vec<String>,
            _progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
        ) -> Result<(), bmc_upgrade::packages::ApplyError> {
            Err(bmc_upgrade::packages::ApplyError("boom".to_owned()))
        }

        async fn list_installable_widgets(
            &self,
        ) -> Result<
            Vec<bmc_upgrade::packages::InstallableWidget>,
            bmc_upgrade::packages::PackageProbeError,
        > {
            Ok(Vec::new())
        }

        fn store_free_bytes(&self) -> std::io::Result<u64> {
            Ok(u64::MAX)
        }
    }

    #[derive(Debug, Default)]
    struct RecordingLifecycle {
        refreshed: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl WidgetLifecycle for RecordingLifecycle {
        async fn stop_all_widgets(&self) {}
        async fn restart_widgets(&self) {}
        async fn refresh_widgets(&self) {
            self.refreshed
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum BackendEvent {
        ForcedGc,
        Apply,
    }

    #[derive(Debug)]
    struct RecordingGcBackend {
        gc_results:
            std::sync::Mutex<std::collections::VecDeque<Result<PackageGcOutcome, PackageGcError>>>,
        requests: std::sync::Mutex<Vec<PackageGcRequest>>,
        events: std::sync::Mutex<Vec<BackendEvent>>,
        /// `None` fails `store_free_bytes`, simulating an unmeasurable filesystem.
        free_bytes: Option<u64>,
    }

    impl RecordingGcBackend {
        fn new(
            results: impl IntoIterator<Item = Result<PackageGcOutcome, PackageGcError>>,
        ) -> Self {
            Self::with_free_bytes(results, Some(u64::MAX))
        }

        fn with_free_bytes(
            results: impl IntoIterator<Item = Result<PackageGcOutcome, PackageGcError>>,
            free_bytes: Option<u64>,
        ) -> Self {
            Self {
                gc_results: std::sync::Mutex::new(results.into_iter().collect()),
                requests: std::sync::Mutex::new(Vec::new()),
                events: std::sync::Mutex::new(Vec::new()),
                free_bytes,
            }
        }
    }

    #[async_trait::async_trait]
    impl PackageBackend for RecordingGcBackend {
        async fn gc(&self, request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
            self.requests
                .lock()
                .expect("BUG: recording backend requests mutex poisoned")
                .push(request);
            // What "forced" now means: sweep regardless.
            if request.sweep == bmc_nix::gc::Sweep::Always {
                self.events
                    .lock()
                    .expect("BUG: recording backend events mutex poisoned")
                    .push(BackendEvent::ForcedGc);
            }
            self.gc_results
                .lock()
                .expect("BUG: recording backend results mutex poisoned")
                .pop_front()
                .unwrap_or(Ok(PackageGcOutcome::Collected))
        }

        async fn probe(&self, _estimate: EstimateMode, _install: &[String]) -> PackageProbe {
            PackageProbe::UpToDate
        }

        async fn apply(
            &self,
            _merged: bmc_nix::types::MergedIndex,
            _install: Vec<String>,
            _progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
        ) -> Result<(), bmc_upgrade::packages::ApplyError> {
            self.events
                .lock()
                .expect("BUG: recording backend events mutex poisoned")
                .push(BackendEvent::Apply);
            Ok(())
        }

        async fn list_installable_widgets(
            &self,
        ) -> Result<
            Vec<bmc_upgrade::packages::InstallableWidget>,
            bmc_upgrade::packages::PackageProbeError,
        > {
            Ok(Vec::new())
        }

        fn store_free_bytes(&self) -> std::io::Result<u64> {
            self.free_bytes
                .ok_or_else(|| std::io::Error::other("scripted statvfs failure"))
        }
    }

    #[tokio::test]
    async fn automatic_upgrade_forces_gc_before_package_dispatch() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let backend = Arc::new(RecordingGcBackend::new([]));
        let backend_dyn = Arc::clone(&backend) as Arc<dyn PackageBackend>;
        let Ok(gate) = automatic_gc_preflight(gate, &backend_dyn, Some(1_000)).await else {
            panic!("BUG: successful forced GC must preserve the gate");
        };

        let run = spawn_packages_run(
            gate,
            backend_dyn,
            Arc::new(RecordingLifecycle::default()),
            empty_merged_index(),
            Vec::new(),
            None,
            StateService::new(),
        );
        assert!(matches!(drain(run).await, Some(UpgradeRunState::Finished)));
        assert_eq!(
            *backend
                .events
                .lock()
                .expect("BUG: recording backend events mutex poisoned"),
            [BackendEvent::ForcedGc, BackendEvent::Apply]
        );
    }

    #[tokio::test]
    async fn automatic_gc_failure_still_dispatches_when_space_suffices() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let backend = Arc::new(RecordingGcBackend::new([Err(PackageGcError::Operational(
            "store failure".to_owned(),
        ))]));
        let backend_dyn = Arc::clone(&backend) as Arc<dyn PackageBackend>;

        assert!(
            automatic_gc_preflight(gate, &backend_dyn, Some(1_000))
                .await
                .is_ok(),
            "the free-space check decides the upgrade, not the collection result"
        );
    }

    #[tokio::test]
    async fn insufficient_space_fails_the_upgrade_and_releases_the_gate() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let backend = Arc::new(RecordingGcBackend::with_free_bytes([], Some(1_000)));
        let backend_dyn = Arc::clone(&backend) as Arc<dyn PackageBackend>;

        let Err(mut run) = automatic_gc_preflight(gate, &backend_dyn, Some(10_000)).await else {
            panic!("BUG: an estimate exceeding free space must stop dispatch");
        };
        assert!(matches!(
            run.next().await,
            Some(UpgradeRunState::Failed(SystemUpgradeError::NotEnoughSpace))
        ));
        assert!(
            run_gate.try_lock().is_ok(),
            "a failed space check must release the run gate"
        );
        assert!(
            backend
                .events
                .lock()
                .expect("BUG: recording backend events mutex poisoned")
                .iter()
                .all(|event| *event != BackendEvent::Apply)
        );
    }

    #[tokio::test]
    async fn space_check_requires_headroom_beyond_the_estimate() {
        let run_gate = Arc::new(Mutex::new(()));
        let backend = Arc::new(RecordingGcBackend::with_free_bytes([], Some(1_000_000)));
        let backend_dyn = Arc::clone(&backend) as Arc<dyn PackageBackend>;

        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        assert!(
            automatic_gc_preflight(gate, &backend_dyn, Some(950_000))
                .await
                .is_err(),
            "an estimate that fits only without headroom must fail"
        );

        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: the failed check released the gate");
        assert!(
            automatic_gc_preflight(gate, &backend_dyn, Some(900_000))
                .await
                .is_ok(),
            "an estimate that fits with headroom must pass"
        );
    }

    #[tokio::test]
    async fn missing_estimate_skips_the_space_check() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let backend = Arc::new(RecordingGcBackend::with_free_bytes([], Some(0)));
        let backend_dyn = Arc::clone(&backend) as Arc<dyn PackageBackend>;

        assert!(
            automatic_gc_preflight(gate, &backend_dyn, None)
                .await
                .is_ok(),
            "no estimate leaves nothing to compare; the realization fails loudly instead"
        );
    }

    #[tokio::test]
    async fn unmeasurable_free_space_does_not_block_the_upgrade() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let backend = Arc::new(RecordingGcBackend::with_free_bytes([], None));
        let backend_dyn = Arc::clone(&backend) as Arc<dyn PackageBackend>;

        assert!(
            automatic_gc_preflight(gate, &backend_dyn, Some(10_000))
                .await
                .is_ok(),
            "a statvfs failure is not a certain \"will not fit\""
        );
    }

    #[tokio::test]
    async fn manual_upgrade_does_not_run_forced_gc() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let backend = Arc::new(RecordingGcBackend::new([]));

        let run = spawn_packages_run(
            gate,
            Arc::clone(&backend) as Arc<dyn PackageBackend>,
            Arc::new(RecordingLifecycle::default()),
            empty_merged_index(),
            Vec::new(),
            None,
            StateService::new(),
        );
        assert!(matches!(drain(run).await, Some(UpgradeRunState::Finished)));
        assert_eq!(
            *backend
                .events
                .lock()
                .expect("BUG: recording backend events mutex poisoned"),
            [BackendEvent::Apply]
        );
    }

    async fn drain(mut run: UpgradeRunStream) -> Option<UpgradeRunState> {
        let mut last = None;
        while let Some(state) = run.next().await {
            last = Some(state);
        }
        last
    }

    #[tokio::test]
    async fn run_gate_is_free_after_a_packages_run_completes() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let lifecycle = Arc::new(RecordingLifecycle::default());

        let run = spawn_packages_run(
            gate,
            Arc::new(StubBackend),
            Arc::clone(&lifecycle) as Arc<dyn WidgetLifecycle>,
            empty_merged_index(),
            Vec::new(),
            None,
            StateService::new(),
        );

        let last = drain(run).await;
        assert!(
            matches!(last, Some(UpgradeRunState::Finished)),
            "run must finish successfully, got {last:?}"
        );
        // The detached run task dropped its gate when it completed; a new
        // run must be startable immediately.
        assert!(
            run_gate.try_lock().is_ok(),
            "run gate must be free after the run completes"
        );
    }

    #[tokio::test]
    async fn successful_packages_run_refreshes_widgets_once() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let lifecycle = Arc::new(RecordingLifecycle::default());

        let run = spawn_packages_run(
            gate,
            Arc::new(StubBackend),
            Arc::clone(&lifecycle) as Arc<dyn WidgetLifecycle>,
            empty_merged_index(),
            vec!["widget-flip-clock".to_owned()],
            None,
            StateService::new(),
        );

        assert!(matches!(drain(run).await, Some(UpgradeRunState::Finished)));
        // A completed install must re-scan exactly once so the newly-installed
        // widget becomes available without a restart.
        assert_eq!(
            lifecycle
                .refreshed
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[derive(Debug)]
    struct GatedLifecycle {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl WidgetLifecycle for GatedLifecycle {
        async fn stop_all_widgets(&self) {}
        async fn restart_widgets(&self) {}
        async fn refresh_widgets(&self) {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }

    #[tokio::test]
    async fn packages_run_refreshes_widgets_before_finishing() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let lifecycle = Arc::new(GatedLifecycle {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        });

        let mut run = spawn_packages_run(
            gate,
            Arc::new(StubBackend),
            Arc::clone(&lifecycle) as Arc<dyn WidgetLifecycle>,
            empty_merged_index(),
            vec!["widget-flip-clock".to_owned()],
            None,
            StateService::new(),
        );

        // apply() has returned Ok and refresh_widgets() is now in flight,
        // parked on the release gate.
        entered.notified().await;

        // Finished must not reach the stream while the refresh is still
        // running: a newly-installed widget would otherwise be reported
        // available to the FE before the registry actually knows about it.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), run.next())
                .await
                .is_err(),
            "Finished must not be observable until the widget refresh completes"
        );

        // Let the refresh finish; only then does the run signal Finished.
        release.notify_one();
        assert!(matches!(drain(run).await, Some(UpgradeRunState::Finished)));
    }

    #[tokio::test]
    async fn failed_packages_run_does_not_refresh_widgets() {
        let run_gate = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&run_gate)
            .try_lock_owned()
            .expect("BUG: fresh gate is lockable");
        let lifecycle = Arc::new(RecordingLifecycle::default());

        let run = spawn_packages_run(
            gate,
            Arc::new(FailingBackend),
            Arc::clone(&lifecycle) as Arc<dyn WidgetLifecycle>,
            empty_merged_index(),
            vec!["widget-flip-clock".to_owned()],
            None,
            StateService::new(),
        );

        assert!(matches!(
            drain(run).await,
            Some(UpgradeRunState::Failed(
                SystemUpgradeError::PackageUpgradeFailed(_)
            ))
        ));
        // A failed apply installed nothing: refreshing would only churn the
        // registry, so it must not fire.
        assert_eq!(
            lifecycle
                .refreshed
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn retriable_only_for_transient_causes() {
        assert!(
            SystemUpgradeError::UnableToCheckForUpgrade(FirmwareDownloadError::IndexDownloadFailed)
                .is_retriable()
        );
        assert!(
            SystemUpgradeError::PackageCheckFailed(
                bmc_upgrade::packages::PackageProbeError::IndexFetchFailed("timeout".to_owned())
            )
            .is_retriable()
        );
        assert!(
            !SystemUpgradeError::PackageCheckFailed(
                bmc_upgrade::packages::PackageProbeError::NoEnabledServers
            )
            .is_retriable()
        );
        assert!(
            !SystemUpgradeError::PackageCheckFailed(
                bmc_upgrade::packages::PackageProbeError::PlanFailed(
                    bmc_upgrade::packages::PackagePlanFailure::MissingSystemPackages {
                        names: vec!["nix".to_owned()],
                    }
                )
            )
            .is_retriable()
        );
        // Both collide with a concurrent run and resolve themselves once the
        // other run finishes; autoupgrade must retry them.
        assert!(SystemUpgradeError::UpgradeInProgress.is_retriable());
        assert!(SystemUpgradeError::UpgradeExpired.is_retriable());
        assert!(
            !SystemUpgradeError::PackageUpgradeFailed("realize failed".to_owned()).is_retriable()
        );
        assert!(!SystemUpgradeError::UpgradeFailed.is_retriable());
        // A rejected image is permanent: retrying the same one is futile.
        assert!(!SystemUpgradeError::InvalidImage.is_retriable());
    }

    #[test]
    fn progress_throttle_admits_first_then_only_after_interval() {
        let mut throttle = ProgressThrottle::default();
        let start = Instant::now();
        assert!(throttle.admit(start));
        assert!(!throttle.admit(start + Duration::from_millis(100)));
        assert!(throttle.admit(start + UPDATE_PROGRESS_INTERVAL));
        assert!(!throttle.admit(start + UPDATE_PROGRESS_INTERVAL + Duration::from_millis(100)));
    }

    #[test]
    fn led_maps_apply_and_activate_to_upgrade_started() {
        assert!(matches!(
            led_event(&UpgradeRunState::Phase(
                SystemUpgradePhase::FirmwareApplying
            )),
            Some(SystemUpgradeState::UpgradeStarted)
        ));
        assert!(matches!(
            led_event(&UpgradeRunState::Phase(
                SystemUpgradePhase::PackageActivating
            )),
            Some(SystemUpgradeState::UpgradeStarted)
        ));
        assert!(matches!(
            led_event(&UpgradeRunState::Finished),
            Some(SystemUpgradeState::Finished)
        ));
    }

    #[test]
    fn records_pending_install_names_to_the_handoff() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("pending.json");
        record_pending_install(&["widget-flip-clock".to_owned()], &path).expect("BUG: write");
        let read = bmc_nix::pending_install::read_pending_install(&path).expect("BUG: read");
        assert_eq!(read.install, vec!["widget-flip-clock".to_owned()]);
    }

    // DownloadFinished, Finished, and Failed are resting states the watch can sit in
    // forever (download and upgrade are two separate gRPC calls, and the watch
    // never resets to None), so they must not block restart; only the
    // transient states do.
    #[test]
    fn blocks_restart_only_during_transient_upgrade_states() {
        assert!(
            SystemUpgradeState::DownloadStarted {
                total_mb: Some(1.0)
            }
            .blocks_restart()
        );
        assert!(
            SystemUpgradeState::DownloadProgress {
                downloaded_mb: 0.5,
                total_mb: Some(1.0)
            }
            .blocks_restart()
        );
        assert!(SystemUpgradeState::UpgradeStarted.blocks_restart());
        assert!(
            !SystemUpgradeState::DownloadFinished {
                hash: Some("h".to_owned()),
                total_mb: Some(1.0)
            }
            .blocks_restart()
        );
        assert!(!SystemUpgradeState::Finished.blocks_restart());
        assert!(!SystemUpgradeState::Failed.blocks_restart());
    }
}

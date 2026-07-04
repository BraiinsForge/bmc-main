// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use anyhow::anyhow;
use bmc_scheduler::JobScheduler;
use bmc_scheduler::jobs::to_boxed;
use bmc_scheduler::scheduler::{JobConfig, Schedule, Task};
use bmc_upgrade::autoupgrade::{AutoUpgrade, AutoUpgradeConfig};
use bmc_upgrade::firmware::{FirmwareDownloadError, FirmwareIndex, UpgradeDetail};
use bmc_upgrade::upgrader::{
    DownloadState as UpgraderDownloadState, FirmwareUpgradeError, FirmwareUpgrader,
};
use chrono::{DateTime, Utc};
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

const INDEX_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Minimum spacing between forwarded intermediate download `Progress`
/// events; without it every written chunk becomes an event on the run
/// channel and the gRPC-web stream.
const UPDATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(300);

pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .build()
        .expect("BUG: static client builder failed")
});

/// Client for package-index fetches, which run under the upgrade run gate:
/// a hung index server must time out instead of wedging every check/start
/// in `UpgradeInProgress`. [`CLIENT`] stays timeout-free because it also
/// serves the long firmware image download.
static INDEX_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(INDEX_FETCH_TIMEOUT)
        .build()
        .expect("BUG: static client builder failed")
});

/// Widget lifecycle control around a disruptive firmware upgrade: stops all
/// running widget processes before the flash hand-off so GPU resources are
/// freed, and respawns them when the upgrade fails. The compositor keeps
/// running throughout so the display stays alive if the upgrade fails and
/// is retried.
#[async_trait::async_trait]
pub(crate) trait WidgetStopper: Send + Sync + std::fmt::Debug {
    async fn stop_all_widgets(&self);
    async fn restart_widgets(&self);
}

#[derive(Clone, Debug)]
pub(crate) struct NixUpgradeConfig {
    pub servers_config_path: PathBuf,
    pub profile_dir: PathBuf,
    pub hooks_dir: String,
    pub hooks_override_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
#[expect(dead_code, reason = "read when mapping the check result onto the wire")]
pub(crate) struct SystemPackageChange {
    pub name: String,
    pub version_from: Option<String>,
    pub version_to: Option<String>,
    pub category: Option<String>,
    pub changelog: Option<String>,
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
#[expect(dead_code, reason = "emitted by the unified start_upgrade run")]
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
#[expect(dead_code, reason = "consumed by start_upgrade and the wire mapping")]
pub(crate) enum AvailableSystemUpgrade {
    Firmware {
        upgrade_id: String,
        detail: UpgradeDetail,
    },
    Packages {
        upgrade_id: String,
        merged: bmc_nix::types::MergedIndex,
        changes: Vec<SystemPackageChange>,
        download_size_bytes: Option<u64>,
        bmc_version: Option<String>,
        bmc_changelog: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct PackagesPreview {
    pub changes: Vec<SystemPackageChange>,
    pub download_size_bytes: Option<u64>,
    pub bmc_version: Option<String>,
    pub bmc_changelog: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disruption {
    AppRestart,
    Reboot,
    Unspecified,
}

#[derive(Debug)]
pub(crate) struct CheckOutcome {
    pub firmware: Option<UpgradeDetail>,
    #[expect(
        dead_code,
        reason = "consumed by the wire mapping of the unified check"
    )]
    pub packages: Option<PackagesPreview>,
    #[expect(
        dead_code,
        reason = "consumed by the wire mapping of the unified check"
    )]
    pub upgrade_id: Option<String>,
    #[expect(
        dead_code,
        reason = "consumed by the wire mapping of the unified check"
    )]
    pub disruption: Disruption,
    #[expect(dead_code, reason = "consumed by autoupgrade retry classification")]
    pub package_fetch_transient: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum EstimateMode {
    Estimate,
    Skip,
}

pub(crate) enum PackageProbe {
    Available(bmc_nix::types::MergedIndex, PackagesPreview),
    /// No servers, no plan, planning error — packages simply not offered.
    Unavailable,
    /// The index fetch itself failed transiently
    /// (`FetchIndexesError::Fetch { .. }`) — still "unavailable" on the
    /// wire, but autoupgrade may retry (Task 7).
    FetchFailed(String),
}

fn arbitrate(firmware: Option<&UpgradeDetail>, packages: Option<&PackagesPreview>) -> Disruption {
    match (firmware, packages) {
        (Some(_), _) => Disruption::Reboot,
        (None, Some(_)) => Disruption::AppRestart,
        (None, None) => Disruption::Unspecified,
    }
}

async fn probe_packages(
    nix_config: &NixUpgradeConfig,
    client: &reqwest::Client,
    estimate: EstimateMode,
) -> PackageProbe {
    if let Some(parent) = nix_config.servers_config_path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        warn!(error = %err, "Failed to create the servers config directory");
    }

    let servers = match std::fs::read_to_string(&nix_config.servers_config_path) {
        Ok(contents) => match serde_json::from_str::<bmc_nix::types::ServersConfig>(&contents) {
            Ok(config) => config.servers,
            Err(err) => {
                warn!(error = %err, "Servers config is unparseable, packages unavailable");
                return PackageProbe::Unavailable;
            }
        },
        Err(err) => {
            warn!(error = %err, "Servers config is unreadable, packages unavailable");
            return PackageProbe::Unavailable;
        }
    };

    if !servers.iter().any(|server| server.enabled) {
        warn!("No enabled package servers, packages unavailable");
        return PackageProbe::Unavailable;
    }

    let merged = match bmc_nix::index::fetch_and_merge_indexes(client, &servers).await {
        Ok(merged) => merged,
        Err(err @ bmc_nix::index::FetchIndexesError::Fetch { .. }) => {
            warn!(error = %err, "Package index fetch failed");
            return PackageProbe::FetchFailed(err.to_string());
        }
        Err(err) => {
            warn!(error = %err, "Package index unusable, packages unavailable");
            return PackageProbe::Unavailable;
        }
    };

    let base = match bmc_nix::manifest::read_current_manifest(&nix_config.profile_dir) {
        Ok(manifest) => manifest,
        Err(bmc_nix::manifest::ReadManifestError::CurrentNotFound { .. }) => {
            match bmc_nix::manifest::read_latest_manifest(&nix_config.profile_dir) {
                Ok(manifest) => manifest,
                Err(err) => {
                    warn!(error = %err, "Failed to read the profile manifest, packages unavailable");
                    return PackageProbe::Unavailable;
                }
            }
        }
        Err(err) => {
            warn!(error = %err, "Failed to read the profile manifest, packages unavailable");
            return PackageProbe::Unavailable;
        }
    };

    let plan = match bmc_nix::manifest::compute_upgrade_plan(&base, Some(&merged), &[], &[]) {
        Ok(plan) => plan,
        Err(err) => {
            warn!(error = %err, "Failed to plan the package upgrade, packages unavailable");
            return PackageProbe::Unavailable;
        }
    };

    if plan.changed.is_empty() && plan.added.is_empty() && plan.removed.is_empty() {
        info!("Packages are up to date");
        return PackageProbe::Unavailable;
    }

    let download_size_bytes = match estimate {
        EstimateMode::Estimate => {
            match bmc_nix::store::estimate_realization(
                &bmc_nix::store::TokioCommandRunner,
                &plan.packages,
            )
            .await
            {
                Ok(realize_estimate) => Some(realize_estimate.download_bytes),
                Err(err) => {
                    warn!(error = %err, "Failed to estimate the package download size");
                    None
                }
            }
        }
        EstimateMode::Skip => None,
    };

    PackageProbe::Available(merged, build_packages_preview(&plan, download_size_bytes))
}

fn build_packages_preview(
    plan: &bmc_nix::types::UpgradePlan,
    download_size_bytes: Option<u64>,
) -> PackagesPreview {
    let resolved_by_name: HashMap<&str, &bmc_nix::types::ResolvedPackage> = plan
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let mut changes = Vec::new();
    for change in &plan.changed {
        let resolved = resolved_by_name.get(change.name.as_str());
        changes.push(SystemPackageChange {
            name: change.name.clone(),
            version_from: Some(change.from_version.clone()),
            version_to: Some(change.to_version.clone()),
            category: resolved.and_then(|package| package.category.clone()),
            changelog: resolved.and_then(|package| package.metadata.get("changelog").cloned()),
        });
    }
    for added in &plan.added {
        let resolved = resolved_by_name.get(added.name.as_str());
        changes.push(SystemPackageChange {
            name: added.name.clone(),
            version_from: None,
            version_to: Some(added.version.clone()),
            category: resolved.and_then(|package| package.category.clone()),
            changelog: resolved.and_then(|package| package.metadata.get("changelog").cloned()),
        });
    }
    for removed in &plan.removed {
        changes.push(SystemPackageChange {
            name: removed.name.clone(),
            version_from: Some(removed.version.clone()),
            version_to: None,
            category: None,
            changelog: None,
        });
    }

    let core = resolved_by_name.get("core");
    let bmc_version = core.and_then(|package| package.metadata.get("bmc_version").cloned());
    let bmc_changelog = core.and_then(|package| package.metadata.get("changelog").cloned());

    PackagesPreview {
        changes,
        download_size_bytes,
        bmc_version,
        bmc_changelog,
    }
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
    throttle: std::sync::Mutex<ProgressThrottle>,
}

impl ChannelUpgradeProgress {
    fn new(sender: tokio::sync::mpsc::UnboundedSender<UpgradeRunState>) -> Self {
        Self {
            sender,
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

    fn on_realization_finished(&self) {}

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

fn spawn_packages_run(
    gate: tokio::sync::OwnedMutexGuard<()>,
    nix_config: NixUpgradeConfig,
    merged: bmc_nix::types::MergedIndex,
) -> UpgradeRunStream {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    task::spawn(async move {
        let _gate = gate;
        let adapter = ChannelUpgradeProgress::new(tx.clone());
        let result = bmc_nix::upgrade::apply_profile_change(
            &nix_config.profile_dir,
            None, // base manifest is re-read under the profile lock
            Some(&merged),
            &[],
            &[],
            bmc_nix::upgrade::ActivationMode::Activate,
            None, // GC is disabled on the packages path
            Some(&adapter),
            &nix_config.hooks_dir,
            nix_config.hooks_override_path.as_deref(),
        )
        .await;
        match result {
            Ok(_) => {
                info!("Package upgrade finished");
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
struct AvailableUpgrade {
    url: String,
    file_size: u64,
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
    available_upgrades: Arc<Mutex<HashMap<String, AvailableUpgrade>>>,
    bmc_manager: Arc<U>,
    scheduler: JobScheduler,
    autoupgrade: Arc<AutoUpgrade>,
    run_gate: Arc<Mutex<()>>,
    upgrade_id_seq: Arc<AtomicUsize>,
    system_upgrades: Arc<Mutex<HashMap<String, AvailableSystemUpgrade>>>,
    nix_config: NixUpgradeConfig,
    widget_stopper: Arc<dyn WidgetStopper>,
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
            available_upgrades: self.available_upgrades.clone(),
            bmc_manager: self.bmc_manager.clone(),
            scheduler: self.scheduler.clone(),
            autoupgrade: self.autoupgrade.clone(),
            run_gate: self.run_gate.clone(),
            upgrade_id_seq: self.upgrade_id_seq.clone(),
            system_upgrades: self.system_upgrades.clone(),
            nix_config: self.nix_config.clone(),
            widget_stopper: self.widget_stopper.clone(),
        }
    }
}

impl<T: FirmwareIndex, U: BmcManager> SystemUpgradeService<T, U> {
    pub(crate) fn new(
        firmware_index: T,
        upgrade_image_path: &PathBuf,
        bmc_manager: Arc<U>,
        state_service: StateService,
        scheduler: JobScheduler,
        nix_config: NixUpgradeConfig,
        widget_stopper: Arc<dyn WidgetStopper>,
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
            available_upgrades: Arc::new(Mutex::new(HashMap::new())),
            bmc_manager,
            scheduler,
            autoupgrade: Arc::new(autoupgrade),
            run_gate: Arc::new(Mutex::new(())),
            upgrade_id_seq: Arc::new(AtomicUsize::new(0)),
            system_upgrades: Arc::new(Mutex::new(HashMap::new())),
            nix_config,
            widget_stopper,
        }
    }

    pub(crate) async fn check_for_upgrade(&self) -> Result<CheckOutcome, SystemUpgradeError> {
        let _gate = self
            .run_gate
            .try_lock()
            .map_err(|_| SystemUpgradeError::UpgradeInProgress)?;

        self.system_upgrades.lock().await.clear();

        let firmware = self.probe_firmware().await?;

        let probe = probe_packages(
            &self.nix_config,
            &INDEX_CLIENT,
            if firmware.is_some() {
                EstimateMode::Skip
            } else {
                EstimateMode::Estimate
            },
        )
        .await;

        let mut package_fetch_transient = false;
        let packages = match probe {
            PackageProbe::Available(merged, preview) => Some((merged, preview)),
            PackageProbe::Unavailable => None,
            PackageProbe::FetchFailed(message) => {
                debug!(message, "Package index fetch failed, not offering packages");
                package_fetch_transient = true;
                None
            }
        };

        let disruption = arbitrate(firmware.as_ref(), packages.as_ref().map(|(_, p)| p));

        let (merged, packages) = match packages {
            Some((merged, preview)) => (Some(merged), Some(preview)),
            None => (None, None),
        };

        let upgrade_id = if firmware.is_some() || packages.is_some() {
            let upgrade_id = format!(
                "upgrade-{}",
                self.upgrade_id_seq.fetch_add(1, Ordering::Relaxed)
            );
            let upgrade = if let Some(detail) = &firmware {
                AvailableSystemUpgrade::Firmware {
                    upgrade_id: upgrade_id.clone(),
                    detail: detail.clone(),
                }
            } else {
                let merged = merged.expect("BUG: upgrade id minted without firmware or packages");
                let preview = packages
                    .as_ref()
                    .expect("BUG: merged index present without a preview");
                AvailableSystemUpgrade::Packages {
                    upgrade_id: upgrade_id.clone(),
                    merged,
                    changes: preview.changes.clone(),
                    download_size_bytes: preview.download_size_bytes,
                    bmc_version: preview.bmc_version.clone(),
                    bmc_changelog: preview.bmc_changelog.clone(),
                }
            };
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
            package_fetch_transient,
        })
    }

    pub(crate) async fn check_for_firmware_upgrade(
        &self,
    ) -> Result<Option<UpgradeDetail>, SystemUpgradeError> {
        Ok(self.check_for_upgrade().await?.firmware)
    }

    #[expect(dead_code, reason = "consumed by the wire mapping of start_upgrade")]
    pub(crate) async fn start_upgrade(&self, upgrade_id: String) -> UpgradeRunStream {
        let (gate, upgrade) =
            match claim_upgrade(&self.run_gate, &self.system_upgrades, &upgrade_id).await {
                Ok(claimed) => claimed,
                Err(stream) => return stream,
            };

        match upgrade {
            AvailableSystemUpgrade::Firmware { detail, .. } => {
                self.spawn_firmware_run(gate, detail)
            }
            AvailableSystemUpgrade::Packages { merged, .. } => {
                spawn_packages_run(gate, self.nix_config.clone(), merged)
            }
        }
    }

    fn spawn_firmware_run(
        &self,
        gate: tokio::sync::OwnedMutexGuard<()>,
        detail: UpgradeDetail,
    ) -> UpgradeRunStream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let firmware_upgrader = Arc::clone(&self.firmware_upgrader);
        let bmc_manager = Arc::clone(&self.bmc_manager);
        let widget_stopper = Arc::clone(&self.widget_stopper);
        task::spawn(async move {
            let _gate = gate;
            let upgrader = firmware_upgrader.lock().await;
            let release = &detail.latest_release;
            let total_bytes = release.file_size as u64;

            _ = tx.send(UpgradeRunState::Phase(
                SystemUpgradePhase::FirmwareDownloading,
            ));
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
                    UpgraderDownloadState::Finished { .. } => download_finished = true,
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

            widget_stopper.stop_all_widgets().await;

            let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let adapter = ChannelUpgradeProgress::new(tx.clone());
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
                    // Ok means the handoff was accepted; sysupgrade terminates
                    // this process only in stage2, so the event still reaches
                    // the client and the stream then ends by process death.
                    info!("Firmware upgrade handoff accepted");
                    _ = tx.send(UpgradeRunState::Phase(SystemUpgradePhase::FirmwareApplying));
                    // The process is now committed to die under sysupgrade;
                    // keep the worker alive so the run gate stays held (no
                    // second upgrade can start during teardown) and the
                    // stream ends by process death, not by completing.
                    std::future::pending::<()>().await;
                }
                Err(err) => {
                    error!(error = %err, "Firmware upgrade failed");
                    widget_stopper.restart_widgets().await;
                    _ = tx.send(UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed));
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

        let available_upgrade = AvailableUpgrade {
            file_size: release_info.latest_release.file_size as u64,
            url: release_info.latest_release.url.clone(),
        };

        self.available_upgrades
            .lock()
            .await
            .insert(release_info.latest_release.hash.clone(), available_upgrade);

        info!(
            hash = %release_info.latest_release.hash,
            version = %release_info.latest_release.version,
            file_size = release_info.latest_release.file_size,
            "Firmware upgrade available"
        );

        Ok(Some(release_info))
    }

    pub fn download_firmware(&self, hash: String) -> UnboundedReceiver<DownloadState> {
        let (progress_sender, progress_receiver) = tokio::sync::mpsc::unbounded_channel();
        let available_upgrades = self.available_upgrades.clone();
        let firmware_upgrader = self.firmware_upgrader.clone();
        let state_service = self.state_service.clone();
        let run_gate = self.run_gate.clone();

        task::spawn(async move {
            info!(hash = %hash, "Starting firmware download");

            let Ok(_gate) = run_gate.try_lock_owned() else {
                warn!("Upgrade already in progress");
                _ = progress_sender
                    .send(DownloadState::Failed(SystemUpgradeError::UpgradeInProgress));
                return;
            };

            let Ok(upgrader) = firmware_upgrader.try_lock() else {
                warn!("Upgrade already in progress");
                _ = progress_sender
                    .send(DownloadState::Failed(SystemUpgradeError::UpgradeInProgress));
                return;
            };

            let firmware_info = {
                let available_upgrades: tokio::sync::MutexGuard<
                    '_,
                    HashMap<String, AvailableUpgrade>,
                > = available_upgrades.lock().await;
                available_upgrades.get(&hash).cloned()
            };

            let Some(firmware_info) = firmware_info else {
                warn!(hash = %hash, "Firmware with requested hash not found");
                _ = progress_sender
                    .send(DownloadState::Failed(SystemUpgradeError::NoImageWithHash));
                return;
            };

            let mut rx = upgrader.download_firmware(
                firmware_info.url.clone(),
                hash.clone(),
                firmware_info.file_size,
            );

            // We need to track total_mb for StateService notifications and throttling
            #[expect(clippy::cast_precision_loss)]
            let total_mb = firmware_info.file_size as f32 / 1_000_000.0;
            let mut progress_updated_at = Instant::now();

            state_service.notify(SystemUpgradeState::DownloadStarted { total_mb });
            while let Some(event) = rx.recv().await {
                match event {
                    UpgraderDownloadState::Progress {
                        downloaded_mb,
                        total_mb,
                    } => {
                        if progress_updated_at.elapsed() < UPDATE_PROGRESS_INTERVAL {
                            continue;
                        }
                        progress_updated_at = Instant::now();

                        state_service.notify(SystemUpgradeState::DownloadProgress {
                            downloaded_mb,
                            total_mb,
                        });

                        _ = progress_sender.send(DownloadState::Progress {
                            downloaded_mb,
                            total_mb,
                        });
                    }
                    UpgraderDownloadState::Finished {
                        hash: finished_hash,
                    } => {
                        info!(hash = %finished_hash, total_mb, "Firmware download finished successfully");
                        state_service.notify(SystemUpgradeState::DownloadFinished {
                            hash: finished_hash.clone(),
                            total_mb,
                        });
                        _ = progress_sender.send(DownloadState::Finished {
                            hash: finished_hash,
                        });
                    }
                    UpgraderDownloadState::Failed(err) => {
                        warn!(error = %err, "Failed to download firmware");
                        state_service.notify(SystemUpgradeState::Failed);
                        _ = progress_sender.send(DownloadState::Failed(err.into()));
                    }
                }
            }
        });

        progress_receiver
    }

    pub async fn verify_and_upgrade(&self, hash: &str) -> Result<(), SystemUpgradeError> {
        info!(hash, "Starting firmware verification and upgrade");

        let _gate = self
            .run_gate
            .try_lock()
            .map_err(|_| SystemUpgradeError::UpgradeInProgress)?;

        let upgrader = self
            .firmware_upgrader
            .try_lock()
            .map_err(|_| SystemUpgradeError::UpgradeInProgress)?;

        self.state_service
            .notify(SystemUpgradeState::UpgradeStarted);

        info!(hash, "Verifying firmware hash");
        upgrader.verify_firmware(hash).await.map_err(|err| {
            warn!(hash, error = %err, "Failed to verify downloaded firmware");
            self.state_service.notify(SystemUpgradeState::Failed);
            SystemUpgradeError::from(err)
        })?;

        info!(hash, "Firmware verification successful, starting upgrade");
        self.bmc_manager
            .upgrade(true, upgrader.upgrade_image_path(), None)
            .await
            .map_err(|err| {
                warn!(error = %err, "Upgrade failed");
                self.state_service.notify(SystemUpgradeState::Failed);
                SystemUpgradeError::UpgradeFailed
            })?;

        drop(upgrader);

        info!("Firmware upgrade completed successfully");
        Ok(())
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
        if let Some(upgrade) = self.check_for_firmware_upgrade().await? {
            info!(hash = %upgrade.latest_release.hash, "Upgrade available");
            let download_stream = self.download_firmware(upgrade.latest_release.hash.clone());

            if let Some(upgrade_hash) = Self::wait_for_download(download_stream).await {
                info!(hash = %upgrade_hash, "Firmware download completed");
                self.verify_and_upgrade(&upgrade.latest_release.hash).await
            } else {
                Err(SystemUpgradeError::FailedToDownload(
                    "Failed to download firmware".to_owned(),
                ))
            }
        } else {
            debug!("No upgrade available");
            Ok(())
        }
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

    async fn wait_for_download(mut receiver: UnboundedReceiver<DownloadState>) -> Option<String> {
        while let Some(res) = receiver.recv().await {
            match res {
                DownloadState::Progress {
                    downloaded_mb,
                    total_mb,
                } => {
                    debug!(downloaded_mb, total_mb, "Firmware download progress");
                }
                DownloadState::Finished { hash } => {
                    return Some(hash);
                }
                DownloadState::Failed(err) => {
                    error!(error = ?err, "Failed to download firmware");
                    return None;
                }
            }
        }
        None
    }
}

pub(crate) enum DownloadState {
    Progress { downloaded_mb: f32, total_mb: f32 },
    Finished { hash: String },
    Failed(SystemUpgradeError),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SystemUpgradeState {
    DownloadStarted { total_mb: f32 },
    DownloadProgress { downloaded_mb: f32, total_mb: f32 },
    DownloadFinished { hash: String, total_mb: f32 },
    UpgradeStarted,
    Failed,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub(crate) enum SystemUpgradeError {
    #[error("Failed to detect current version")]
    FailedToDetectCurrentVersion,
    #[error("Firmware with a given hash does not exist")]
    NoImageWithHash,
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
    #[error("Upgrade id is unknown or already consumed")]
    UpgradeExpired,
    #[error("Package upgrade failed: {0}")]
    PackageUpgradeFailed(String),
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
        matches!(
            self,
            Self::UnableToCheckForUpgrade(
                FirmwareDownloadError::IndexDownloadFailed
                    | FirmwareDownloadError::FetchUpgradeDetails
            )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::path::Path;

    fn test_nix_config(root: &Path, servers: &Path) -> NixUpgradeConfig {
        NixUpgradeConfig {
            servers_config_path: servers.to_path_buf(),
            profile_dir: root.join("profile"),
            hooks_dir: "hooks".to_owned(),
            hooks_override_path: None,
        }
    }

    #[tokio::test]
    async fn probe_reports_unavailable_when_no_enabled_servers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        std::fs::write(
            &path,
            r#"{"factory":{"id":"factory","base_url":"http://x","known_public_key":"k","priority":0,"enabled":false},"servers":[]}"#,
        )
        .expect("BUG: write servers.json");
        let config = test_nix_config(dir.path(), &path);
        assert!(matches!(
            probe_packages(&config, &INDEX_CLIENT, EstimateMode::Skip).await,
            PackageProbe::Unavailable
        ));
    }

    #[tokio::test]
    async fn probe_reports_unavailable_when_servers_json_missing() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let config = test_nix_config(dir.path(), &dir.path().join("absent.json"));
        assert!(matches!(
            probe_packages(&config, &INDEX_CLIENT, EstimateMode::Skip).await,
            PackageProbe::Unavailable
        ));
    }

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

    fn test_packages_preview() -> PackagesPreview {
        PackagesPreview {
            changes: Vec::new(),
            download_size_bytes: None,
            bmc_version: None,
            bmc_changelog: None,
        }
    }

    #[test]
    fn arbitrate_firmware_wins_over_packages() {
        let firmware = test_upgrade_detail();
        let packages = test_packages_preview();
        assert_eq!(
            arbitrate(Some(&firmware), Some(&packages)),
            Disruption::Reboot
        );
    }

    #[test]
    fn arbitrate_packages_only_is_app_restart() {
        let packages = test_packages_preview();
        assert_eq!(arbitrate(None, Some(&packages)), Disruption::AppRestart);
    }

    #[test]
    fn arbitrate_nothing_available_is_unspecified() {
        assert_eq!(arbitrate(None, None), Disruption::Unspecified);
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
                upgrade_id: "upgrade-0".to_owned(),
                detail: test_upgrade_detail(),
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

    #[test]
    fn progress_throttle_admits_first_then_only_after_interval() {
        let mut throttle = ProgressThrottle::default();
        let start = Instant::now();
        assert!(throttle.admit(start));
        assert!(!throttle.admit(start + Duration::from_millis(100)));
        assert!(throttle.admit(start + UPDATE_PROGRESS_INTERVAL));
        assert!(!throttle.admit(start + UPDATE_PROGRESS_INTERVAL + Duration::from_millis(100)));
    }
}

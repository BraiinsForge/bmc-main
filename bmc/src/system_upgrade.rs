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
use std::time::{Duration, Instant};
use std::{collections::HashMap, sync::LazyLock};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::watch::{self, Receiver};
use tokio::sync::{Mutex, Notify};
use tokio::task;
use tracing::{debug, error, info, warn};

const UPDATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(300);
const AUTOUPGRADE_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(30);
const AUTOUPGRADE_RETRY_MAX_ATTEMPTS: u32 = 5;
const AUTOUPGRADE_RETRY_DELAY_COEFF: u32 = 2;

pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .build()
        .expect("BUG: static client builder failed")
});

#[derive(Clone, Debug)]
#[expect(dead_code, reason = "consumed in the package-check path")]
pub(crate) struct NixUpgradeConfig {
    pub servers_config_path: PathBuf,
    pub profile_dir: PathBuf,
    pub hooks_dir: String,
    pub hooks_override_path: Option<PathBuf>,
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
        }
    }

    pub(crate) async fn check_for_upgrade(
        &self,
    ) -> Result<Option<UpgradeDetail>, SystemUpgradeError> {
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

        task::spawn(async move {
            info!(hash = %hash, "Starting firmware download");

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
            .upgrade(true, upgrader.upgrade_image_path())
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
        if let Some(upgrade) = self.check_for_upgrade().await? {
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

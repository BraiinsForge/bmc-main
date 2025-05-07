// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_upgrade::{
    downloader::FileDownloader,
    firmware::{
        DownloadEvent, FirmwareDownloadError, FirmwareIndex, FirmwareResolver, UpgradeDetail,
    },
};
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use std::{collections::HashMap, sync::LazyLock};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::watch::{self, Receiver};
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use tokio::time::MissedTickBehavior;
use tokio::{task, time};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{BmcManager, storage_checker::StorageChecker};

const UPDATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(300);

pub static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .build()
        .expect("BUG: static client builder failed")
});

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
    firmware_resolver: Arc<Mutex<Arc<FirmwareResolver<T>>>>,
    available_upgrades: Arc<Mutex<HashMap<String, AvailableUpgrade>>>,
    upgrade_image_path: PathBuf,
    bmc_manager: Arc<U>,
    client: Client,
}

impl<T, U> Clone for SystemUpgradeService<T, U>
where
    T: FirmwareIndex,
    U: BmcManager,
{
    fn clone(&self) -> Self {
        Self {
            state_service: self.state_service.clone(),
            firmware_resolver: self.firmware_resolver.clone(),
            available_upgrades: self.available_upgrades.clone(),
            upgrade_image_path: self.upgrade_image_path.clone(),
            bmc_manager: self.bmc_manager.clone(),
            client: self.client.clone(),
        }
    }
}

impl<T: FirmwareIndex, U: BmcManager> SystemUpgradeService<T, U> {
    pub(crate) fn new(
        firmware_resolver: FirmwareResolver<T>,
        upgrade_image_path: &PathBuf,
        bmc_manager: Arc<U>,
        state_service: StateService,
    ) -> Self {
        Self {
            state_service,
            firmware_resolver: Arc::new(Mutex::new(Arc::new(firmware_resolver))),
            available_upgrades: Arc::new(Mutex::new(HashMap::new())),
            upgrade_image_path: upgrade_image_path.to_owned(),
            bmc_manager,
            client: CLIENT.clone(),
        }
    }

    pub(crate) async fn init(&self) {
        let is_after_upgrade = self.bmc_manager.check_and_remove_upgrade_marker().await;

        if is_after_upgrade {
            self.state_service
                .notify(SystemUpgradeState::UpgradeFinished);
        }
    }

    pub(crate) async fn check_for_upgrade(
        &self,
    ) -> Result<Option<UpgradeDetail>, SystemUpgradeError> {
        let Ok(firmware_handle) = self.firmware_resolver.try_lock() else {
            return Err(SystemUpgradeError::UpgradeInProgress);
        };

        let platform = self.bmc_manager.platform();
        let version = self.bmc_manager.version();

        let Some(release_info) = firmware_handle
            .check_for_upgrade(&self.client, platform, version)
            .await
            .map_err(SystemUpgradeError::UnableToCheckForUpgrade)?
        else {
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

        Ok(Some(release_info))
    }

    pub fn download_firmware(&self, hash: String) -> UnboundedReceiver<DownloadState> {
        let (progress_sender, progress_receiver) = tokio::sync::mpsc::unbounded_channel();
        let path = self.upgrade_image_path.clone();
        let available_upgrades = self.available_upgrades.clone();
        let firmware_handle = self.firmware_resolver.clone();
        let state_service = self.state_service.clone();
        let client = self.client.clone();

        task::spawn(async move {
            let Ok(firmware_handle_guard) = firmware_handle.try_lock() else {
                warn!("Upgrade is in progress");
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
                warn!("No firmware with hash");
                _ = progress_sender
                    .send(DownloadState::Failed(SystemUpgradeError::NoImageWithHash));
                return;
            };

            if let Err(e) = StorageChecker::check_disk_space(&path, firmware_info.file_size) {
                warn!("Error while checking disk space for upgrade: {}", e);
                _ = progress_sender.send(DownloadState::Failed(SystemUpgradeError::NotEnoughSpace));
                return;
            }

            let Ok(file_downloader) = FileDownloader::init(&path).await else {
                warn!("Error creating file to download firmware");
                _ = progress_sender.send(DownloadState::Failed(
                    SystemUpgradeError::FailedToDownload(
                        "Unable to create file to download data".to_owned(),
                    ),
                ));
                return;
            };

            let mut rx = firmware_handle_guard.download_firmware(
                &client,
                &firmware_info.url,
                file_downloader,
            );

            let downloaded_bytes = Arc::new(AtomicUsize::new(0));

            #[expect(clippy::cast_precision_loss)]
            let total_mb = firmware_info.file_size as f32 / 1_000_000.0;

            let mut firmware_checksum = String::new();
            let cancellation_token = CancellationToken::new();

            let handle = tokio::spawn(Self::send_download_progress(
                downloaded_bytes.clone(),
                total_mb,
                state_service.clone(),
                progress_sender.clone(),
                cancellation_token.clone(),
            ));

            state_service.notify(SystemUpgradeState::DownloadStarted);
            while let Some(data) = rx.recv().await {
                match data {
                    Ok(event) => match event {
                        DownloadEvent::BytesWritten(bytes) => {
                            downloaded_bytes.fetch_add(bytes, Ordering::AcqRel);
                        }
                        DownloadEvent::Finished { checksum } => firmware_checksum = checksum,
                    },
                    Err(err) => {
                        warn!("Error while downloading upgrade firmware: {}", &err);

                        state_service.notify(SystemUpgradeState::Failed);

                        _ = progress_sender.send(DownloadState::Failed(
                            SystemUpgradeError::FailedToDownload(err.to_string()),
                        ));

                        cancellation_token.cancel();
                        return;
                    }
                }
            }

            cancellation_token.cancel();
            _ = handle.await;

            if hash.to_lowercase() == firmware_checksum.to_lowercase() {
                state_service.notify(SystemUpgradeState::DownloadFinished(
                    firmware_checksum.clone(),
                ));
                _ = progress_sender.send(DownloadState::Finished {
                    hash: firmware_checksum,
                });
            } else {
                state_service.notify(SystemUpgradeState::Failed);
                _ = progress_sender.send(DownloadState::Failed(
                    SystemUpgradeError::DownloadedImageHashMismatch {
                        expected: hash,
                        actual: firmware_checksum,
                    },
                ));
            }
        });

        progress_receiver
    }

    async fn send_download_progress(
        downloaded_bytes: Arc<AtomicUsize>,
        total_mb: f32,
        state_service: StateService,
        download_progress_sender: UnboundedSender<DownloadState>,
        cancellation_token: CancellationToken,
    ) {
        let mut interval = time::interval(UPDATE_PROGRESS_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        while !cancellation_token.is_cancelled() {
            interval.tick().await;

            #[expect(clippy::cast_precision_loss)]
            let downloaded_mb = downloaded_bytes.load(Ordering::Acquire) as f32 / 1_000_000.0;

            state_service.notify(SystemUpgradeState::DownloadProgress {
                downloaded_mb,
                total_mb,
            });

            _ = download_progress_sender.send(DownloadState::Progress {
                downloaded_mb,
                total_mb,
            });
        }
    }

    pub async fn verify_and_upgrade(&self, hash: &str) -> Result<(), SystemUpgradeError> {
        let firmware_guard = self
            .firmware_resolver
            .try_lock()
            .map_err(|_| SystemUpgradeError::UpgradeInProgress)?;

        self.state_service
            .notify(SystemUpgradeState::UpgradeStarted);

        FileDownloader::verify_hash(&self.upgrade_image_path, hash)
            .await
            .map_err(|e| {
                warn!("Error when verifying downloaded firmware. {}", e);
                self.state_service.notify(SystemUpgradeState::Failed);
                match e {
                    bmc_upgrade::downloader::DownloaderError::HashMismatch { expected, actual } => {
                        SystemUpgradeError::DownloadedImageHashMismatch { expected, actual }
                    }
                    bmc_upgrade::downloader::DownloaderError::FailedToReadFile(_) => {
                        SystemUpgradeError::VerifyFailed
                    }
                }
            })?;

        self.bmc_manager
            .upgrade(true, &self.upgrade_image_path)
            .await
            .map_err(|e| {
                warn!("Upgrade was not successful. {}", e);
                self.state_service.notify(SystemUpgradeState::Failed);
                SystemUpgradeError::UpgradeFailed
            })?;

        drop(firmware_guard);

        Ok(())
    }
}

pub(crate) enum DownloadState {
    Progress { downloaded_mb: f32, total_mb: f32 },
    Finished { hash: String },
    Failed(SystemUpgradeError),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SystemUpgradeState {
    DownloadStarted,
    DownloadProgress { downloaded_mb: f32, total_mb: f32 },
    DownloadFinished(String),
    UpgradeStarted,
    UpgradeFinished,
    Failed,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub(crate) enum SystemUpgradeError {
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

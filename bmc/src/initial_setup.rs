// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bmc_shared_time::time::{DateFormat, TimeSystem, Timezone};
use thiserror::Error;
use tokio::sync::watch::{self, Receiver};
use tracing::{info, warn};

use crate::{
    BmcManager,
    manager::{InitialSetupError, WifiNetworkConfig},
    utils::NumberFormat,
};

const REBOOT_SLEEP_DURATION: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub(crate) struct StateService {
    sender: Arc<watch::Sender<Option<InitSetupState>>>,
}
impl StateService {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(None);

        Self {
            sender: Arc::new(sender),
        }
    }

    fn notify(&self, value: InitSetupState) {
        let value = Some(value);

        self.sender.send_if_modified(|current| {
            if *current != value {
                *current = value;
                return true;
            }
            false
        });
    }

    pub(crate) fn subscribe(&self) -> Receiver<Option<InitSetupState>> {
        self.sender.subscribe()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InitialSetup<T: BmcManager> {
    manager: Arc<T>,
    in_progress: Arc<AtomicBool>,
    state_service: StateService,
}

impl<T: BmcManager> InitialSetup<T> {
    pub(crate) fn new(manager: Arc<T>, in_progress: Arc<AtomicBool>) -> Self {
        Self {
            manager,
            in_progress,
            state_service: StateService::new(),
        }
    }

    pub(crate) fn connect_to_wifi(&self, config: WifiNetworkConfig) -> Result<(), SetupError> {
        self.in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .map_err(|_| SetupError::InProgress)?;

        // Once dropped, progress is set to false
        let progress_guard = ProgressGuard {
            in_progress: self.in_progress.clone(),
        };

        let state_service = self.state_service.clone();
        let manager = self.manager.clone();

        tokio::task::spawn(async move {
            let _guard = progress_guard;

            state_service.notify(InitSetupState::ConnectingToWifi {
                wifi_ssid: config.ssid.clone(),
            });

            match manager.wifi_initial_setup(config).await {
                Ok(()) => {
                    state_service.notify(InitSetupState::WifiConnectionSuccess);
                }
                Err(InitialSetupError::NotSupported) => {
                    warn!("Initial setup is not supported");
                    state_service.notify(InitSetupState::UnexpectedError);
                }
                Err(InitialSetupError::UnexpectedFailure(e)) => {
                    warn!(
                        "Unexpected failure during initial setup. Rebooting device. Error: {}",
                        e
                    );
                    Self::notify_failure_and_reboot(manager, &state_service).await;
                }
                Err(InitialSetupError::WifiConnectionFailure(e)) => {
                    warn!("Failed to connect to wifi: {}", e);
                    state_service.notify(InitSetupState::WifiConnectionFailed);

                    // Revert wifi settings
                    if let Err(e) = manager.revert_to_initial_setup().await {
                        warn!("Failed to revert back to initial setup: {}", e);
                        Self::notify_failure_and_reboot(manager, &state_service).await;
                    }
                }
            }
        });

        Ok(())
    }

    async fn notify_failure_and_reboot(manager: Arc<T>, state_service: &StateService) {
        state_service.notify(InitSetupState::UnexpectedError);
        tokio::time::sleep(REBOOT_SLEEP_DURATION).await;
        _ = manager.reboot().await;
    }

    pub(crate) fn subscribe(&self) -> Receiver<Option<InitSetupState>> {
        self.state_service.subscribe()
    }

    pub(crate) async fn setup_device(&self, config: DeviceSetupConfig) -> anyhow::Result<()> {
        self.manager.set_timezone(config.timezone).await?;

        if config.system_password.is_some() {
            self.manager.set_password(config.system_password).await?;
        }

        // TODO: save number_format, date_format, data_collection to config

        info!(
            "data collection: {}, time system: {}, number format: {}, date format: {}",
            config.data_collection,
            config.time_system,
            config.number_format.format_number(1234567.89),
            config.date_format.format_string()
        );

        self.manager.update_device_state().await?;

        self.state_service
            .notify(InitSetupState::DeviceSetupSuccess);

        Ok(())
    }
}

#[derive(Error, Debug)]
pub(crate) enum SetupError {
    #[error("Initial setup is in progress")]
    InProgress,
}

struct ProgressGuard {
    in_progress: Arc<AtomicBool>,
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        self.in_progress.store(false, Ordering::Release);
    }
}

#[derive(PartialEq, Debug, Clone)]
pub enum InitSetupState {
    ConnectingToWifi { wifi_ssid: String },
    WifiConnectionSuccess,
    WifiConnectionFailed,
    UnexpectedError,
    DeviceSetupSuccess,
}

#[derive(Debug)]
pub(crate) struct DeviceSetupConfig {
    pub(crate) timezone: Timezone,
    pub(crate) system_password: Option<String>,
    pub(crate) time_system: TimeSystem,
    pub(crate) date_format: DateFormat,
    pub(crate) number_format: NumberFormat,
    pub(crate) data_collection: bool,
}

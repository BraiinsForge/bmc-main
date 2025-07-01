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
use tokio::sync::{
    RwLock,
    watch::{self, Receiver},
};
use tracing::warn;

use crate::{
    BmcManager,
    config::DisplayConfigHandle,
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
    config_handle: Arc<RwLock<DisplayConfigHandle>>,
}

impl<T: BmcManager> InitialSetup<T> {
    pub(crate) fn new(
        manager: Arc<T>,
        in_progress: Arc<AtomicBool>,
        config_handle: Arc<RwLock<DisplayConfigHandle>>,
    ) -> Self {
        Self {
            manager,
            in_progress,
            state_service: StateService::new(),
            config_handle,
        }
    }

    pub(crate) fn connect_to_wifi(&self, config: WifiNetworkConfig) -> Result<(), WifiSetupError> {
        self.in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .map_err(|_| WifiSetupError::InProgress)?;

        let in_progress = self.in_progress.clone();
        let state_service = self.state_service.clone();
        let manager = self.manager.clone();

        tokio::task::spawn(async move {
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

            in_progress.store(false, Ordering::Release);
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

    pub(crate) async fn setup_device(
        &self,
        config: DeviceSetupConfig,
    ) -> Result<(), DeviceSetupError> {
        let mut config_guard = self
            .config_handle
            .try_write()
            .map_err(|_| DeviceSetupError::InProgress)?;

        self.manager
            .set_timezone(config.timezone)
            .await
            .map_err(DeviceSetupError::SetTimezone)?;

        if config.system_password.is_some() {
            self.manager
                .set_password(config.system_password)
                .await
                .map_err(|_| DeviceSetupError::SetPassword)?;
        }

        config_guard.set_date_format(config.date_format);
        config_guard.set_number_format(config.number_format);
        config_guard.set_time_system(config.time_system);
        config_guard.set_data_collection(config.data_collection);
        config_guard
            .sync_to_storage()
            .await
            .map_err(DeviceSetupError::SyncConfigData)?;

        self.manager
            .update_device_state()
            .await
            .map_err(DeviceSetupError::UpdateDeviceState)?;

        self.state_service
            .notify(InitSetupState::DeviceSetupSuccess);

        Ok(())
    }
}

#[derive(Error, Debug)]
pub(crate) enum WifiSetupError {
    #[error("WiFi setup is in progress")]
    InProgress,
}

#[derive(Error, Debug)]
pub(crate) enum DeviceSetupError {
    #[error("Device setup is in progress")]
    InProgress,
    #[error("Failed to set timezone, error: {0}")]
    SetTimezone(#[source] anyhow::Error),
    #[error("Failed to set password")]
    SetPassword,
    #[error("Failed to save data to config, error: {0}")]
    SyncConfigData(#[source] anyhow::Error),
    #[error("Failed to update device state, error: {0}")]
    UpdateDeviceState(#[source] anyhow::Error),
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

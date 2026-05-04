// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::system_upgrade::SystemUpgradeService;
use crate::{
    BmcManager,
    config::{ConfigHandle, UnitSystem},
    manager::{InitialSetupError, WifiNetworkConfig},
};
use bmc_shared_time::time::{DateFormat, TimeSystem, Timezone};
use bmc_shared_utils::{number_format::NumberFormat, temperature::TemperatureUnit};
use bmc_upgrade::autoupgrade::{
    AutoUpgradeConfig, AutoUpgradeFrequency, SECONDS_DEVICE_SETUP_DELAY,
};
use bmc_upgrade::firmware::FirmwareIndex;
use chrono::{TimeDelta, Utc};
use std::ops::Add;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tokio::sync::{
    RwLock,
    watch::{self, Receiver},
};
use tracing::{info, warn};

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
pub(crate) struct InitialSetup<T: BmcManager, F: FirmwareIndex> {
    manager: Arc<T>,
    in_progress: Arc<AtomicBool>,
    state_service: StateService,
    config_handle: Arc<RwLock<ConfigHandle>>,
    system_upgrade_service: SystemUpgradeService<F, T>,
}

impl<T: BmcManager, F: FirmwareIndex> InitialSetup<T, F> {
    pub(crate) fn new(
        manager: Arc<T>,
        in_progress: Arc<AtomicBool>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        system_upgrade_service: SystemUpgradeService<F, T>,
    ) -> Self {
        Self {
            manager,
            in_progress,
            state_service: StateService::new(),
            config_handle,
            system_upgrade_service,
        }
    }

    pub(crate) fn connect_to_wifi(
        &self,
        config: WifiNetworkConfig,
        is_reconfig: bool,
    ) -> Result<(), WifiSetupError> {
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

            if is_reconfig {
                Self::handle_wifi_reconfig(manager, config, &state_service).await;
            } else {
                Self::handle_wifi_initial_setup(manager, config, &state_service).await;
            }

            in_progress.store(false, Ordering::Release);
        });

        Ok(())
    }

    async fn handle_wifi_initial_setup(
        manager: Arc<T>,
        config: WifiNetworkConfig,
        state_service: &StateService,
    ) {
        match manager.wifi_initial_setup(config).await {
            Ok(()) => {
                state_service.notify(InitSetupState::WifiConnectionSuccess);
                info!("WiFi initial setup completed successfully");
            }
            Err(InitialSetupError::NotSupported) => {
                warn!("WiFi initial setup not supported");
                state_service.notify(InitSetupState::UnexpectedError);
            }
            Err(InitialSetupError::UnexpectedFailure(err)) => {
                warn!(
                    error = %err,
                    "Unexpected failure during WiFi initial setup, rebooting device"
                );
                Self::notify_failure_and_reboot(manager, state_service).await;
            }
            Err(InitialSetupError::WifiConnectionFailure(err)) => {
                warn!(error = %err, "Failed to connect to WiFi");
                state_service.notify(InitSetupState::WifiConnectionFailed);

                // Revert wifi settings
                if let Err(err) = manager.revert_to_initial_setup().await {
                    warn!(error = %err, "Failed to revert to initial setup");
                    Self::notify_failure_and_reboot(manager, state_service).await;
                }
            }
        }
    }

    async fn handle_wifi_reconfig(
        manager: Arc<T>,
        config: WifiNetworkConfig,
        state_service: &StateService,
    ) {
        // For reconfiguration, we connect and then exit reconfig mode (return to Operational)
        let ssid = config.ssid.clone();
        match manager
            .wifi_save_and_connect(config.ssid, config.password, config.encryption)
            .await
        {
            Ok(()) => {
                info!(ssid = %ssid, "WiFi reconfiguration connection successful");
                // Exit reconfiguration mode (disables captive portal, removes flag)
                if let Err(err) = manager.exit_wifi_reconfiguration().await {
                    warn!(error = %err, "Failed to exit wifi reconfiguration mode");
                    state_service.notify(InitSetupState::UnexpectedError);
                    return;
                }
                state_service.notify(InitSetupState::WifiReconfigSuccess);
                info!("WiFi reconfiguration completed successfully");
            }
            Err(err) => {
                warn!(error = %err, ssid = %ssid, "Failed to connect to WiFi during reconfiguration");
                state_service.notify(InitSetupState::WifiConnectionFailed);

                // Re-enable AP so user can try again
                if let Err(err) = manager.enter_wifi_reconfig().await {
                    warn!(error = %err, "Failed to re-enable WiFi AP after failed connection");
                }
            }
        }
    }

    async fn notify_failure_and_reboot(manager: Arc<T>, state_service: &StateService) {
        state_service.notify(InitSetupState::UnexpectedError);
        tokio::time::sleep(REBOOT_SLEEP_DURATION).await;
        _ = manager.reboot().await;
    }

    #[expect(dead_code, reason = "consumed by future display-overlay channel")]
    pub(crate) fn subscribe(&self) -> Receiver<Option<InitSetupState>> {
        self.state_service.subscribe()
    }

    pub(crate) async fn setup_device(
        &self,
        config: DeviceSetupConfig,
    ) -> Result<(), DeviceSetupError> {
        let timezone = config.timezone;

        let mut config_guard = self
            .config_handle
            .try_write()
            .map_err(|_| DeviceSetupError::InProgress)?;

        self.manager
            .set_timezone(timezone.clone())
            .await
            .map_err(DeviceSetupError::SetTimezone)?;

        info!(timezone = %timezone, "Device timezone configured");

        if config.system_password.is_some() {
            self.manager
                .set_password(config.system_password)
                .await
                .map_err(|_| DeviceSetupError::SetPassword)?;

            info!("Device system password configured");
        }
        let time_of_day = Utc::now()
            .time()
            .add(TimeDelta::seconds(SECONDS_DEVICE_SETUP_DELAY));
        let autoupgrade_config = AutoUpgradeConfig::new(
            true,
            AutoUpgradeFrequency::default(),
            Some(time_of_day),
            timezone.chrono_offset(),
        );

        let date_format = config.date_format;
        let number_format = config.number_format;
        let time_system = config.time_system;
        let data_collection = config.data_collection;
        let temperature_unit = config.temperature_unit;
        let unit_system = config.unit_system;

        config_guard.set_date_format(date_format);
        config_guard.set_number_format(number_format);
        config_guard.set_time_system(time_system);
        config_guard.set_data_collection(data_collection);
        config_guard.set_temperature_unit(temperature_unit);
        config_guard.set_unit_system(unit_system.clone());
        config_guard.set_autoupgrade(autoupgrade_config.clone());
        config_guard
            .save()
            .await
            .map_err(DeviceSetupError::SyncConfigData)?;

        info!(
            date_format = ?date_format,
            number_format = ?number_format,
            time_system = ?time_system,
            data_collection = data_collection,
            temperature_unit = ?temperature_unit,
            unit_system = ?unit_system,
            "Device configuration saved"
        );

        self.manager
            .update_device_state()
            .await
            .map_err(DeviceSetupError::UpdateDeviceState)?;

        self.state_service
            .notify(InitSetupState::DeviceSetupSuccess);

        self.system_upgrade_service
            .autoupgrade_reschedule(autoupgrade_config)
            .await
            .map_err(DeviceSetupError::EnableAutoUpgrade)?;

        info!("Device setup completed successfully");

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
    #[error("Failed to enable AutoUpgrade, error: {0}")]
    EnableAutoUpgrade(#[source] anyhow::Error),
}

#[derive(PartialEq, Debug, Clone)]
pub enum InitSetupState {
    ConnectingToWifi { wifi_ssid: String },
    WifiConnectionSuccess,
    WifiConnectionFailed,
    WifiReconfigSuccess,
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
    pub(crate) temperature_unit: TemperatureUnit,
    pub(crate) unit_system: UnitSystem,
}

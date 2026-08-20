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

use crate::system_upgrade::SystemUpgradeService;
use crate::{
    BmcManager,
    config::ConfigHandle,
    manager::{InitialSetupError, WifiNetworkConfig},
};
use bmc_shared_time::time::{DateFormat, TimeSystem, Timezone};
use bmc_shared_utils::{
    number_format::NumberFormat, temperature::TemperatureUnit, unit_system::UnitSystem,
};
use bmc_upgrade::firmware::FirmwareIndex;
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
        let network = manager.network_manager();
        let Some(wifi) = network.wifi() else {
            warn!("WiFi initial setup not supported");
            state_service.notify(InitSetupState::UnexpectedError { restarting: false });
            return;
        };
        match wifi.wifi_initial_setup(config).await {
            Ok(()) => {
                state_service.notify(InitSetupState::WifiConnectionSuccess);
                info!("WiFi initial setup completed successfully");
            }
            Err(InitialSetupError::NotSupported) => {
                warn!("WiFi initial setup not supported");
                state_service.notify(InitSetupState::UnexpectedError { restarting: false });
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
                if let Err(err) = wifi.revert_to_initial_setup().await {
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
        let network = manager.network_manager();
        let Some(wifi) = network.wifi() else {
            warn!("WiFi reconfiguration not supported");
            state_service.notify(InitSetupState::UnexpectedError { restarting: false });
            return;
        };
        match wifi
            .wifi_save_and_connect(config.ssid, config.password, config.encryption)
            .await
        {
            Ok(()) => {
                info!(ssid = %ssid, "WiFi reconfiguration connection successful");
                // Exit reconfiguration mode (disables captive portal, removes flag)
                if let Err(err) = wifi.exit_wifi_reconfiguration().await {
                    warn!(error = %err, "Failed to exit wifi reconfiguration mode");
                    state_service.notify(InitSetupState::UnexpectedError { restarting: false });
                    return;
                }
                state_service.notify(InitSetupState::WifiReconfigSuccess);
                info!("WiFi reconfiguration completed successfully");
            }
            Err(err) => {
                warn!(error = %err, ssid = %ssid, "Failed to connect to WiFi during reconfiguration");
                state_service.notify(InitSetupState::WifiConnectionFailed);

                // Re-enable AP so user can try again
                if let Err(err) = wifi.enter_wifi_reconfiguration().await {
                    warn!(error = %err, "Failed to re-enable WiFi AP after failed connection");
                }
            }
        }
    }

    async fn notify_failure_and_reboot(manager: Arc<T>, state_service: &StateService) {
        state_service.notify(InitSetupState::UnexpectedError { restarting: true });
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
        let previous_autoupgrade_config = config_guard.autoupgrade();
        let autoupgrade_config = self
            .system_upgrade_service
            .create_autoupgrade_config(true)
            .map_err(DeviceSetupError::EnableAutoUpgrade)?;

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
        config_guard.set_unit_system(unit_system);
        config_guard.set_autoupgrade(autoupgrade_config);
        if let Err(err) = config_guard.save().await {
            config_guard.set_autoupgrade(previous_autoupgrade_config);
            return Err(DeviceSetupError::SyncConfigData(err));
        }

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
            .network_manager()
            .provisioning()
            .advance()
            .await
            .map_err(DeviceSetupError::UpdateDeviceState)?;

        // The device already left SetupPending above, so erroring out here
        // would fail a setup no client can retry; the saved config lets the
        // next boot's autoupgrade_init recover the schedule.
        if let Err(err) = self.system_upgrade_service.apply_autoupgrade(true).await {
            warn!(
                ?err,
                "Failed to schedule automatic upgrade checks; deferring to the next boot"
            );
        }

        self.state_service
            .notify(InitSetupState::DeviceSetupSuccess);

        // A fresh device should not wait up to two hours for its first check;
        // the setup flow ending implies the device is up and attended.
        self.system_upgrade_service.autoupgrade_check_now();

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
    ConnectingToWifi {
        wifi_ssid: String,
    },
    WifiConnectionSuccess,
    WifiConnectionFailed,
    WifiReconfigSuccess,
    /// Setup cannot continue. `restarting` says whether bmc resolves it
    /// by restarting or resetting the device, which is what decides
    /// whether the screen waits it out or asks the user to act.
    UnexpectedError {
        restarting: bool,
    },
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

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
    manager::{InitialSetupError, NetworkProtocolConfig, WifiNetworkConfig},
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
use tracing::{error, info, warn};

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
            state_service.notify(InitSetupState::SwitchingUplink {
                uplink: Uplink::Wifi {
                    ssid: config.ssid.clone(),
                },
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

    async fn apply_device_settings(
        &self,
        config: DeviceSetupConfig,
    ) -> Result<(), DeviceSetupError> {
        let timezone = config.timezone;

        // NOTE: wait for the lock, never try_write: some task may be holding
        // the read lock at this moment and try_write would fail against it.
        let mut config_guard = self.config_handle.write().await;

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

        config_guard.set_date_format(config.date_format);
        config_guard.set_number_format(config.number_format);
        config_guard.set_time_system(config.time_system);
        config_guard.set_data_collection(config.data_collection);
        config_guard.set_temperature_unit(config.temperature_unit);
        config_guard.set_unit_system(config.unit_system);
        config_guard
            .save()
            .await
            .map_err(DeviceSetupError::SyncConfigData)?;

        info!(
            date_format = ?config.date_format,
            number_format = ?config.number_format,
            time_system = ?config.time_system,
            data_collection = config.data_collection,
            temperature_unit = ?config.temperature_unit,
            unit_system = ?config.unit_system,
            "Device configuration saved"
        );

        Ok(())
    }

    async fn enable_autoupgrade(&self) -> Result<(), DeviceSetupError> {
        // NOTE: wait for the lock, never try_write: some task may be holding
        // the read lock at this moment and try_write would fail against it.
        let mut config_guard = self.config_handle.write().await;
        let previous = config_guard.autoupgrade();
        let autoupgrade_config = self
            .system_upgrade_service
            .create_autoupgrade_config(true)
            .map_err(DeviceSetupError::EnableAutoUpgrade)?;
        config_guard.set_autoupgrade(autoupgrade_config);
        if let Err(err) = config_guard.save().await {
            config_guard.set_autoupgrade(previous);
            return Err(DeviceSetupError::SyncConfigData(err));
        }
        Ok(())
    }

    /// Finish setup once the device has advanced to Operational: schedule
    /// auto-upgrade, notify listeners, and kick off the first upgrade check.
    async fn finalize_setup(&self) {
        // The device already left SetupPending, so erroring out here would fail
        // a setup no client can retry; the saved config lets the next boot's
        // autoupgrade_init recover the schedule.
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
    }

    pub(crate) async fn setup_device(
        &self,
        config: DeviceSetupConfig,
    ) -> Result<(), DeviceSetupError> {
        self.apply_device_settings(config).await?;
        self.enable_autoupgrade().await?;

        self.manager
            .network_manager()
            .provisioning()
            .advance()
            .await
            .map_err(DeviceSetupError::UpdateDeviceState)?;

        self.finalize_setup().await;
        info!("Device setup completed successfully");

        Ok(())
    }

    /// Advance past WiFi setup without connecting, for a miner reachable over
    /// ethernet. Errors when ethernet is not connected.
    pub(crate) async fn skip_wifi(&self) -> Result<(), SkipWifiError> {
        if self
            .manager
            .network_manager()
            .ethernet_ipv4()
            .await
            .is_none()
        {
            return Err(SkipWifiError::NoEthernet);
        }
        self.in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .map_err(|_| SkipWifiError::InProgress)?;

        self.state_service.notify(InitSetupState::SwitchingUplink {
            uplink: Uplink::Ethernet,
        });

        let in_progress = self.in_progress.clone();
        let state_service = self.state_service.clone();
        let manager = self.manager.clone();
        tokio::spawn(async move {
            let network = manager.network_manager();
            let stopped = match network.wifi() {
                Some(wifi) => wifi.stop_wifi_ap().await,
                None => Ok(()),
            };
            let advanced = match stopped {
                Ok(()) => network.provisioning().advance().await,
                Err(err) => Err(err),
            };
            match advanced {
                Ok(()) => info!("Skipped WiFi setup; device reachable over ethernet"),
                Err(err) => {
                    error!(?err, "Failed to skip WiFi setup, rebooting device");
                    Self::notify_failure_and_reboot(manager, &state_service).await;
                }
            }
            in_progress.store(false, Ordering::Release);
        });
        Ok(())
    }

    pub(crate) async fn setup_miner(
        &self,
        params: MinerSetupParams,
    ) -> Result<(), DeviceSetupError> {
        self.apply_device_settings(params.device).await?;

        write_custom_installation_config(&params.pool)
            .map_err(DeviceSetupError::WritePoolConfig)?;
        info!("Pool configuration written for first boot");

        self.manager
            .network_manager()
            .apply_network_settings(params.network, params.hostname)
            .await
            .map_err(DeviceSetupError::ApplyNetwork)?;

        self.manager
            .network_manager()
            .provisioning()
            .advance()
            .await
            .map_err(DeviceSetupError::UpdateDeviceState)?;

        for service in ["boser", "bosminer"] {
            if let Err(err) = self.manager.control_service(service, &["start"]).await {
                error!(
                    ?err,
                    service, "Failed to start mining service, rebooting device"
                );
                let manager = self.manager.clone();
                let state_service = self.state_service.clone();
                tokio::spawn(async move {
                    Self::notify_failure_and_reboot(manager, &state_service).await;
                });
                return Err(DeviceSetupError::StartServices(err));
            }
        }

        self.state_service
            .notify(InitSetupState::DeviceSetupSuccess);

        info!("Miner setup completed successfully");

        Ok(())
    }
}

#[derive(Error, Debug)]
pub(crate) enum WifiSetupError {
    #[error("WiFi setup is in progress")]
    InProgress,
}

#[derive(Error, Debug)]
pub(crate) enum SkipWifiError {
    #[error("Ethernet is not connected; WiFi setup is required")]
    NoEthernet,
    #[error("WiFi setup is in progress")]
    InProgress,
}

#[derive(Error, Debug)]
pub(crate) enum DeviceSetupError {
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
    #[error("Failed to apply network configuration, error: {0}")]
    ApplyNetwork(#[source] anyhow::Error),
    #[error("Failed to write pool configuration, error: {0}")]
    WritePoolConfig(#[source] anyhow::Error),
    #[error("Failed to start mining services, error: {0}")]
    StartServices(#[source] anyhow::Error),
}

#[derive(PartialEq, Debug, Clone)]
pub enum InitSetupState {
    SwitchingUplink {
        uplink: Uplink,
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

/// The uplink the device keeps once the setup AP is gone.
#[derive(PartialEq, Debug, Clone)]
pub enum Uplink {
    Ethernet,
    Wifi { ssid: String },
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

/// A single mining pool collected during miner setup.
#[derive(Debug)]
pub(crate) struct PoolEntry {
    pub(crate) url: String,
    pub(crate) user: String,
    pub(crate) password: Option<String>,
}

/// Step-2 configuration for a miner (BMM101): the shared device settings
/// (localization + password) plus network and the mining pool.
#[derive(Debug)]
pub(crate) struct MinerSetupParams {
    pub(crate) device: DeviceSetupConfig,
    pub(crate) network: Option<NetworkProtocolConfig>,
    pub(crate) hostname: Option<String>,
    pub(crate) pool: PoolEntry,
}

/// Path boser reads on first boot to seed pools into `/etc/bosminer.toml`.
const CUSTOM_INSTALLATION_CONFIG_PATH: &str = "/tmp/post_install/custom_config.json";

#[derive(serde::Serialize)]
struct CustomInstallationConfig {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pools: Vec<CustomInstallationPool>,
}

#[derive(serde::Serialize)]
struct CustomInstallationPool {
    url: String,
    user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

/// Write the pool into the custom-installation config file boser consumes.
fn write_custom_installation_config(pool: &PoolEntry) -> anyhow::Result<()> {
    let config = CustomInstallationConfig {
        pools: vec![CustomInstallationPool {
            url: pool.url.clone(),
            user: pool.user.clone(),
            password: pool.password.clone(),
        }],
    };
    let path = std::path::Path::new(CUSTOM_INSTALLATION_CONFIG_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

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

use super::{GrpcError, system::into_grpc_timezone};
use crate::initial_setup::{DeviceSetupError, WifiSetupError};
use crate::web::grpc::network::{scan_wifi_response, try_into_wifi_network_config};
use crate::web::grpc::shared::try_from_number_format;
use crate::{
    BmcManager,
    initial_setup::{DeviceSetupConfig, InitialSetup},
    manager::BmcState,
};
use bmc_grpc::web::{
    DateFormat, NumberFormat, ScanWifiResponse, SetWifiRequest, SettingsDataResponse,
    SettingsRequest, TemperatureUnit, TimeFormat, UnitSystem,
    initial_setup_service_server::InitialSetupService as GrpcInitialSetupService,
};
use bmc_shared_time::time::{TimeSystem, Timezone};
use bmc_shared_utils::temperature::TemperatureUnit as ConfigTemperatureUnit;
use bmc_shared_utils::unit_system::UnitSystem as ConfigUnitSystem;
use bmc_upgrade::firmware::FirmwareIndex;
use std::{str::FromStr, sync::Arc};
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, FieldViolation, StatusExt};
use tracing::warn;

#[derive(Clone)]
pub(crate) struct InitialSetupService<T, F>
where
    T: BmcManager,
    F: FirmwareIndex,
{
    manager: Arc<T>,
    initial_setup: InitialSetup<T, F>,
}

impl<T, F> InitialSetupService<T, F>
where
    T: BmcManager,
    F: FirmwareIndex,
{
    pub(crate) fn new(manager: Arc<T>, initial_setup: InitialSetup<T, F>) -> Self {
        Self {
            manager,
            initial_setup,
        }
    }

    async fn check_precondition(&self, state: BmcState) -> Result<(), Status> {
        let current_state = self
            .manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        if current_state != state {
            return Err(Status::failed_precondition(format!(
                "Function is only available when the device is in '{state}' state. Current state is '{current_state}'.",
            )));
        }
        Ok(())
    }

    async fn check_wifi_setup_precondition(&self) -> Result<BmcState, Status> {
        let current_state = self
            .manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        if current_state != BmcState::FactoryDefault
            && current_state != BmcState::WifiReconfiguration
        {
            return Err(Status::failed_precondition(format!(
                "Function is only available when the device is in 'factory default' or 'wifi reconfiguration' state. Current state is '{current_state}'.",
            )));
        }
        Ok(current_state)
    }
}

#[async_trait::async_trait]
impl<T, F> GrpcInitialSetupService for InitialSetupService<T, F>
where
    T: BmcManager,
    F: FirmwareIndex,
{
    async fn set_wifi(&self, request: Request<SetWifiRequest>) -> Result<Response<()>, Status> {
        let state = self.check_wifi_setup_precondition().await?;

        let request = request.into_inner();
        let config = try_into_wifi_network_config(request)?;
        let is_reconfig = state == BmcState::WifiReconfiguration;

        match self.initial_setup.connect_to_wifi(config, is_reconfig) {
            Ok(()) => Ok(Response::new(())),
            Err(e) => match e {
                WifiSetupError::InProgress => {
                    Err(Status::failed_precondition("WiFi setup is in progress"))
                }
            },
        }
    }

    async fn scan_wifi(&self, _request: Request<()>) -> Result<Response<ScanWifiResponse>, Status> {
        self.check_wifi_setup_precondition().await?;

        Ok(Response::new(
            scan_wifi_response(self.manager.clone()).await?,
        ))
    }

    async fn get_settings_data(
        &self,
        _request: Request<()>,
    ) -> Result<Response<SettingsDataResponse>, Status> {
        self.check_precondition(BmcState::SetupPending).await?;

        let timezones = Timezone::list().iter().map(into_grpc_timezone).collect();

        Ok(Response::new(SettingsDataResponse {
            timezones,
            timezone_id: Timezone::default().iana().to_owned(),
            data_collection: Some(true),
            time_format: TimeFormat::TimeFormat24Hour.into(),
            date_format: DateFormat::DdMmYyyyDot.into(),
            number_format: NumberFormat::SpaceGroupCommaDecimal.into(),
            temperature_unit: TemperatureUnit::Celsius.into(),
            unit_system: UnitSystem::Metric.into(),
        }))
    }

    async fn setup_device(
        &self,
        request: Request<SettingsRequest>,
    ) -> Result<Response<()>, Status> {
        self.check_precondition(BmcState::SetupPending).await?;

        let request = request.into_inner();

        let config = request.try_into()?;

        self.initial_setup
            .setup_device(config)
            .await
            .inspect_err(|e| warn!("Error while setting device, {}", e))
            .map_err(|e| match e {
                DeviceSetupError::InProgress => {
                    Status::failed_precondition("Device setup is in progress")
                }
                DeviceSetupError::SetTimezone(..)
                | DeviceSetupError::SetPassword
                | DeviceSetupError::SyncConfigData(..)
                | DeviceSetupError::UpdateDeviceState(..)
                | DeviceSetupError::EnableAutoUpgrade(..) => {
                    Status::internal("Error while saving device settings")
                }
            })?;

        Ok(Response::new(()))
    }
}

impl TryFrom<SettingsRequest> for DeviceSetupConfig {
    type Error = Status;

    fn try_from(value: SettingsRequest) -> Result<Self, Self::Error> {
        let mut field_violations = vec![];

        let timezone = Timezone::from_str(&value.timezone_id).inspect_err(|_| {
            field_violations.push(FieldViolation::new("timezone", "invalid timezone variant"));
        });

        let time_system = try_from_time_format(value.time_format())
            .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

        let date_format = try_from_date_time(value.date_format())
            .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

        let number_format = try_from_number_format(value.number_format())
            .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

        let temperature_unit = try_from_temperature_unit(value.temperature_unit())
            .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

        let unit_system = try_from_unit_system(value.unit_system())
            .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

        let err = Status::with_error_details(
            Code::InvalidArgument,
            GrpcError::BadRequest.to_string(),
            ErrorDetails::with_bad_request(field_violations),
        );

        let timezone = timezone.map_err(|_| err.clone())?;
        let time_system = time_system.map_err(|_| err.clone())?;
        let date_format = date_format.map_err(|_| err.clone())?;
        let number_format = number_format.map_err(|_| err.clone())?;
        let temperature_unit = temperature_unit.map_err(|_| err.clone())?;
        let unit_system = unit_system.map_err(|_| err)?;

        Ok(DeviceSetupConfig {
            timezone,
            system_password: value.password,
            time_system,
            number_format,
            date_format,
            data_collection: value.data_collection,
            temperature_unit,
            unit_system,
        })
    }
}

pub(crate) fn try_from_time_format(
    value: TimeFormat,
) -> Result<bmc_shared_time::time::TimeSystem, FieldViolation> {
    match value {
        TimeFormat::Unspecified => Err(FieldViolation::new(
            "time_format",
            "time_format cannot be unspecified",
        )),
        TimeFormat::TimeFormat12Hour => Ok(TimeSystem::Hour12),
        TimeFormat::TimeFormat24Hour => Ok(TimeSystem::Hour24),
    }
}

pub(crate) fn try_from_date_time(
    value: DateFormat,
) -> Result<bmc_shared_time::time::DateFormat, FieldViolation> {
    match value {
        DateFormat::Unspecified => Err(FieldViolation::new(
            "date_format",
            "date_format cannot be unspecified",
        )),
        DateFormat::DdMmYyyyDot => Ok(bmc_shared_time::time::DateFormat::DdMmYyyyDot),
        DateFormat::DdMmYyyySlash => Ok(bmc_shared_time::time::DateFormat::DdMmYyyySlash),
        DateFormat::DMYyyySlash => Ok(bmc_shared_time::time::DateFormat::DMYyyySlash),
        DateFormat::MDYyyySlash => Ok(bmc_shared_time::time::DateFormat::MDYyyySlash),
        DateFormat::DdMmYyyyDash => Ok(bmc_shared_time::time::DateFormat::DdMmYyyyDash),
        DateFormat::YyyyMDSlash => Ok(bmc_shared_time::time::DateFormat::YyyyMDSlash),
        DateFormat::YyyyMmDdDot => Ok(bmc_shared_time::time::DateFormat::YyyyMmDdDot),
        DateFormat::YyyyMmDdDash => Ok(bmc_shared_time::time::DateFormat::YyyyMmDdDash),
    }
}

fn try_from_temperature_unit(
    value: TemperatureUnit,
) -> Result<ConfigTemperatureUnit, FieldViolation> {
    match value {
        TemperatureUnit::Unspecified => Err(FieldViolation::new(
            "temperature_unit",
            "temperature_unit cannot be unspecified",
        )),
        TemperatureUnit::Celsius => Ok(ConfigTemperatureUnit::Celsius),
        TemperatureUnit::Fahrenheit => Ok(ConfigTemperatureUnit::Fahrenheit),
    }
}

fn try_from_unit_system(value: UnitSystem) -> Result<ConfigUnitSystem, FieldViolation> {
    match value {
        UnitSystem::Unspecified => Err(FieldViolation::new(
            "unit_system",
            "unit_system cannot be unspecified",
        )),
        UnitSystem::Metric => Ok(ConfigUnitSystem::Metric),
        UnitSystem::Imperial => Ok(ConfigUnitSystem::Imperial),
    }
}

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

use super::system::into_grpc_timezone;
use crate::initial_setup::{DeviceSetupError, WifiSetupError};
use crate::web::grpc::network::{
    bad_request, into_network_config, parse_network_config, scan_wifi_response,
    try_into_wifi_network_config,
};
use crate::web::grpc::shared::{try_from_number_format, violation};
use crate::{
    BmcManager,
    initial_setup::{DeviceSetupConfig, InitialSetup, MinerSetupParams, PoolEntry, SkipWifiError},
    manager::{BmcState, WifiNetworkConfig},
};
use bmc_grpc::google::rpc::bad_request::FieldViolation;
use bmc_grpc::web::{
    DateFormat, InitialSetupService as GrpcInitialSetupService, NumberFormat, PoolConfig,
    ScanWifiResponse, SetWifiRequest, SettingsDataResponse, SettingsRequest, TemperatureUnit,
    TimeFormat, UnitSystem,
};
use bmc_platform::HardwareProfile;
use buffa::EnumValue;
use buffa_types::google::protobuf::{BoolValue, Empty};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

const SOLO_POOL_URL: &str = "stratum+tcp://solo.stratum.braiins.com:3333";
use bmc_shared_time::time::{TimeSystem, Timezone};
use bmc_shared_utils::temperature::TemperatureUnit as ConfigTemperatureUnit;
use bmc_shared_utils::unit_system::UnitSystem as ConfigUnitSystem;
use bmc_upgrade::firmware::FirmwareIndex;
use std::{str::FromStr, sync::Arc};
use tracing::warn;

#[derive(Clone)]
pub(crate) struct InitialSetupService<T>
where
    T: BmcManager,
{
    manager: Arc<T>,
    product_setup: Arc<dyn ProductSetup>,
}

impl<T> InitialSetupService<T>
where
    T: BmcManager,
{
    pub(crate) fn new<F: FirmwareIndex>(
        manager: Arc<T>,
        initial_setup: InitialSetup<T, F>,
    ) -> Self {
        let mining_supported = HardwareProfile::for_product(manager.platform().product())
            .capabilities()
            .mining_supported;
        let product_setup: Arc<dyn ProductSetup> = if mining_supported {
            Arc::new(MinerSetup::new(initial_setup))
        } else {
            Arc::new(DisplaySetup::new(initial_setup))
        };
        Self {
            manager,
            product_setup,
        }
    }

    async fn check_precondition(&self, state: BmcState) -> Result<(), ConnectError> {
        let current_state = self
            .manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        if current_state != state {
            return Err(ConnectError::failed_precondition(format!(
                "Function is only available when the device is in '{state}' state. Current state is '{current_state}'.",
            )));
        }
        Ok(())
    }

    async fn check_wifi_setup_precondition(&self) -> Result<BmcState, ConnectError> {
        let current_state = self
            .manager
            .network_manager()
            .provisioning()
            .device_state()
            .await;
        if current_state != BmcState::FactoryDefault
            && current_state != BmcState::WifiReconfiguration
        {
            return Err(ConnectError::failed_precondition(format!(
                "Function is only available when the device is in 'factory default' or 'wifi reconfiguration' state. Current state is '{current_state}'.",
            )));
        }
        Ok(current_state)
    }
}

impl<T> GrpcInitialSetupService for InitialSetupService<T>
where
    T: BmcManager,
{
    async fn set_wifi(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetWifiRequest>,
    ) -> ServiceResult<Empty> {
        let state = self.check_wifi_setup_precondition().await?;

        if request.ssid.trim().is_empty() {
            return Err(bad_request(vec![violation("ssid", "Missing value!")]));
        }

        let config = try_into_wifi_network_config(request.to_owned_message())?;
        let is_reconfig = state == BmcState::WifiReconfiguration;

        match self.product_setup.connect_wifi(config, is_reconfig) {
            Ok(()) => Response::ok(Empty::default()),
            Err(e) => match e {
                WifiSetupError::InProgress => Err(ConnectError::failed_precondition(
                    "Wi-Fi setup is in progress",
                )),
            },
        }
    }

    async fn skip_wifi(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.check_wifi_setup_precondition().await?;
        self.product_setup.skip_wifi().await?;
        Response::ok(Empty::default())
    }

    async fn scan_wifi(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ScanWifiResponse> {
        self.check_wifi_setup_precondition().await?;

        Response::ok(scan_wifi_response(self.manager.clone()).await?)
    }

    async fn get_settings_data(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<SettingsDataResponse> {
        self.check_precondition(BmcState::SetupPending).await?;

        let timezones = Timezone::list().iter().map(into_grpc_timezone).collect();
        let hostname = self.manager.network_manager().hostname().await;
        let network = self
            .manager
            .network_manager()
            .network_config()
            .await
            .as_ref()
            .map(into_network_config);

        let mining_supported = HardwareProfile::for_product(self.manager.platform().product())
            .capabilities()
            .mining_supported;
        let pool = mining_supported.then(|| PoolConfig {
            url: SOLO_POOL_URL.to_owned(),
            user: String::new(),
            password: None,
        });

        Response::ok(SettingsDataResponse {
            timezones,
            timezone_id: Timezone::default().iana().to_owned(),
            data_collection: BoolValue {
                value: true,
                ..Default::default()
            }
            .into(),
            time_format: TimeFormat::TimeFormat24Hour.into(),
            date_format: DateFormat::DdMmYyyyDot.into(),
            number_format: NumberFormat::SpaceGroupCommaDecimal.into(),
            temperature_unit: TemperatureUnit::Celsius.into(),
            unit_system: UnitSystem::Metric.into(),
            pool: pool.into(),
            hostname,
            network: network.into(),
        })
    }

    async fn setup_device(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SettingsRequest>,
    ) -> ServiceResult<Empty> {
        self.check_precondition(BmcState::SetupPending).await?;

        self.product_setup
            .setup_device(request.to_owned_message())
            .await?;

        Response::ok(Empty::default())
    }
}

/// Per-product initial-setup process. The wire format is shared; each product
/// implements the steps it actually runs.
#[async_trait::async_trait]
pub(crate) trait ProductSetup: Send + Sync {
    fn connect_wifi(
        &self,
        config: WifiNetworkConfig,
        is_reconfig: bool,
    ) -> Result<(), WifiSetupError>;
    async fn skip_wifi(&self) -> Result<(), ConnectError>;
    async fn setup_device(&self, request: SettingsRequest) -> Result<(), ConnectError>;
}

/// Display device (Deck)
pub(crate) struct DisplaySetup<T: BmcManager, F: FirmwareIndex> {
    inner: InitialSetup<T, F>,
}

impl<T: BmcManager, F: FirmwareIndex> DisplaySetup<T, F> {
    fn new(inner: InitialSetup<T, F>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl<T: BmcManager, F: FirmwareIndex> ProductSetup for DisplaySetup<T, F> {
    fn connect_wifi(
        &self,
        config: WifiNetworkConfig,
        is_reconfig: bool,
    ) -> Result<(), WifiSetupError> {
        self.inner.connect_to_wifi(config, is_reconfig)
    }

    async fn skip_wifi(&self) -> Result<(), ConnectError> {
        Err(ConnectError::failed_precondition(
            "WiFi setup is required on this device",
        ))
    }

    async fn setup_device(&self, request: SettingsRequest) -> Result<(), ConnectError> {
        let config: DeviceSetupConfig = request.try_into()?;
        self.inner
            .setup_device(config)
            .await
            .inspect_err(|e| warn!("Error while setting device, {}", e))
            .map_err(|e| map_device_setup_error(&e))
    }
}

pub(crate) struct MinerSetup<T: BmcManager, F: FirmwareIndex> {
    inner: InitialSetup<T, F>,
}

impl<T: BmcManager, F: FirmwareIndex> MinerSetup<T, F> {
    fn new(inner: InitialSetup<T, F>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl<T: BmcManager, F: FirmwareIndex> ProductSetup for MinerSetup<T, F> {
    fn connect_wifi(
        &self,
        config: WifiNetworkConfig,
        is_reconfig: bool,
    ) -> Result<(), WifiSetupError> {
        self.inner.connect_to_wifi(config, is_reconfig)
    }

    async fn skip_wifi(&self) -> Result<(), ConnectError> {
        self.inner.skip_wifi().await.map_err(|e| match e {
            SkipWifiError::NoEthernet | SkipWifiError::InProgress => {
                ConnectError::failed_precondition(e.to_string())
            }
        })
    }

    async fn setup_device(&self, request: SettingsRequest) -> Result<(), ConnectError> {
        // NOTE: every field is checked before anything is written, so a rejected
        // request leaves the device untouched and names each offending field.
        let mut field_violations = vec![];

        let pool = request.pool.clone().into_option();
        match &pool {
            None => field_violations.push(violation("pool", "Missing value!")),
            Some(pool) => {
                if url::Url::parse(&pool.url).is_err() {
                    field_violations.push(violation("pool.url", "Invalid URL!"));
                }
                if pool.user.trim().is_empty() {
                    field_violations.push(violation("pool.user", "Missing value!"));
                }
            }
        }

        let hostname = request.hostname.clone();
        if let Some(hostname) = hostname.as_deref()
            && let Err(err) = bmc_net::validate_hostname(hostname)
        {
            field_violations.push(violation("hostname", err.to_string()));
        }

        let network = match request.network.as_option() {
            None => None,
            Some(config) => {
                match parse_network_config(config, "network.protocol", "network.static.") {
                    Ok(config) => Some(config),
                    Err(violations) => {
                        field_violations.extend(violations);
                        None
                    }
                }
            }
        };

        let device = match parse_device_setup(request) {
            Ok(device) => Some(device),
            Err(violations) => {
                field_violations.extend(violations);
                None
            }
        };

        if !field_violations.is_empty() {
            return Err(bad_request(field_violations));
        }
        let pool = pool.expect("BUG: a missing pool is reported as a field violation");
        let device = device.expect("BUG: an invalid device setup is reported as field violations");

        let params = MinerSetupParams {
            device,
            network,
            hostname,
            pool: PoolEntry {
                url: pool.url,
                user: pool.user,
                password: pool.password,
            },
        };

        self.inner
            .setup_miner(params)
            .await
            .inspect_err(|e| warn!("Error while setting up miner, {}", e))
            .map_err(|e| map_device_setup_error(&e))
    }
}

fn map_device_setup_error(e: &DeviceSetupError) -> ConnectError {
    match e {
        DeviceSetupError::SetTimezone(..)
        | DeviceSetupError::SetPassword
        | DeviceSetupError::SyncConfigData(..)
        | DeviceSetupError::UpdateDeviceState(..)
        | DeviceSetupError::EnableAutoUpgrade(..)
        | DeviceSetupError::ApplyNetwork(..)
        | DeviceSetupError::WritePoolConfig(..)
        | DeviceSetupError::StartServices(..) => {
            ConnectError::internal("Error while saving device settings")
        }
    }
}

impl TryFrom<SettingsRequest> for DeviceSetupConfig {
    type Error = ConnectError;

    fn try_from(value: SettingsRequest) -> Result<Self, Self::Error> {
        parse_device_setup(value).map_err(bad_request)
    }
}

fn parse_device_setup(value: SettingsRequest) -> Result<DeviceSetupConfig, Vec<FieldViolation>> {
    const REPORTED: &str = "BUG: an invalid field is reported as a field violation";
    let mut field_violations = vec![];

    let timezone = Timezone::from_str(&value.timezone_id).inspect_err(|_| {
        field_violations.push(violation("timezone_id", "invalid timezone variant"));
    });

    let time_system = try_from_time_format(value.time_format)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let date_format = try_from_date_time(value.date_format)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let number_format = try_from_number_format(value.number_format)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let temperature_unit = try_from_temperature_unit(value.temperature_unit)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let unit_system = try_from_unit_system(value.unit_system)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    if !field_violations.is_empty() {
        return Err(field_violations);
    }
    let timezone = timezone.expect(REPORTED);
    let time_system = time_system.expect(REPORTED);
    let date_format = date_format.expect(REPORTED);
    let number_format = number_format.expect(REPORTED);
    let temperature_unit = temperature_unit.expect(REPORTED);
    let unit_system = unit_system.expect(REPORTED);

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

pub(crate) fn try_from_time_format(
    value: EnumValue<TimeFormat>,
) -> Result<bmc_shared_time::time::TimeSystem, FieldViolation> {
    match value
        .as_known()
        .unwrap_or(TimeFormat::TimeFormatUnspecified)
    {
        TimeFormat::TimeFormatUnspecified => Err(violation(
            "time_format",
            "time_format cannot be unspecified",
        )),
        TimeFormat::TimeFormat12Hour => Ok(TimeSystem::Hour12),
        TimeFormat::TimeFormat24Hour => Ok(TimeSystem::Hour24),
    }
}

pub(crate) fn try_from_date_time(
    value: EnumValue<DateFormat>,
) -> Result<bmc_shared_time::time::DateFormat, FieldViolation> {
    match value.as_known().unwrap_or(DateFormat::Unspecified) {
        DateFormat::Unspecified => Err(violation(
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
    value: EnumValue<TemperatureUnit>,
) -> Result<ConfigTemperatureUnit, FieldViolation> {
    match value.as_known().unwrap_or(TemperatureUnit::Unspecified) {
        TemperatureUnit::Unspecified => Err(violation(
            "temperature_unit",
            "temperature_unit cannot be unspecified",
        )),
        TemperatureUnit::Celsius => Ok(ConfigTemperatureUnit::Celsius),
        TemperatureUnit::Fahrenheit => Ok(ConfigTemperatureUnit::Fahrenheit),
    }
}

fn try_from_unit_system(value: EnumValue<UnitSystem>) -> Result<ConfigUnitSystem, FieldViolation> {
    match value.as_known().unwrap_or(UnitSystem::Unspecified) {
        UnitSystem::Unspecified => Err(violation(
            "unit_system",
            "unit_system cannot be unspecified",
        )),
        UnitSystem::Metric => Ok(ConfigUnitSystem::Metric),
        UnitSystem::Imperial => Ok(ConfigUnitSystem::Imperial),
    }
}

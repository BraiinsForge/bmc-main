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

use super::intl::intl;
use super::language::save_language;
use super::system::into_grpc_timezone;
use crate::config::ConfigHandle;
use crate::initial_setup::{DeviceSetupError, WifiSetupError};
use crate::web::grpc::network::{
    bad_request, into_network_config, parse_network_config, scan_wifi_response,
    try_into_wifi_network_config,
};
use crate::web::grpc::shared::{missing_value, require_state, try_from_number_format, violation};
use crate::{
    BmcManager,
    initial_setup::{DeviceSetupConfig, InitialSetup, MinerSetupParams, PoolEntry, SkipWifiError},
    manager::{BmcState, WifiNetworkConfig},
};
use bmc_grpc::google::rpc::bad_request::FieldViolation;
use bmc_grpc::web::{
    DateFormat, InitialSetupService as GrpcInitialSetupService, NumberFormat, PoolConfig,
    ScanWifiResponse, SetLanguageRequest, SetWifiRequest, SettingsDataResponse, SettingsRequest,
    TemperatureUnit, TimeFormat, UnitSystem,
};
use bmc_intl::{Intl, format_message};
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
use tokio::sync::RwLock;
use tracing::warn;

#[derive(Clone)]
pub(crate) struct InitialSetupService<T>
where
    T: BmcManager,
{
    manager: Arc<T>,
    product_setup: Arc<dyn ProductSetup>,
    config_handle: Arc<RwLock<ConfigHandle>>,
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
        let config_handle = initial_setup.config_handle();
        let product_setup: Arc<dyn ProductSetup> = if mining_supported {
            Arc::new(MinerSetup::new(initial_setup))
        } else {
            Arc::new(DisplaySetup::new(initial_setup))
        };
        Self {
            manager,
            product_setup,
            config_handle,
        }
    }

    async fn require_device_settings_step(&self, intl: &Intl) -> Result<BmcState, ConnectError> {
        require_state(
            &*self.manager,
            |state| match state {
                BmcState::SetupPending => true,
                BmcState::FactoryDefault
                | BmcState::WifiReconfiguration
                | BmcState::Operational
                | BmcState::Unsupported => false,
            },
            || {
                format_message!(
                    intl,
                    default_message: "This is only available at the device settings step of the setup.",
                    description: "Error when the setup's device settings form is used while the device is at another step",
                )
            },
        )
        .await
    }

    async fn require_wifi_setup(&self, intl: &Intl) -> Result<BmcState, ConnectError> {
        require_state(
            &*self.manager,
            |state| match state {
                BmcState::FactoryDefault | BmcState::WifiReconfiguration => true,
                BmcState::SetupPending | BmcState::Operational | BmcState::Unsupported => false,
            },
            || {
                format_message!(
                    intl,
                    default_message: "This is only available while the device's Wi-Fi is being set up.",
                    description: "Error when the setup's Wi-Fi step is used while the device is not waiting for Wi-Fi settings",
                )
            },
        )
        .await
    }

    async fn require_setup(&self, intl: &Intl) -> Result<BmcState, ConnectError> {
        require_state(
            &*self.manager,
            |state| match state {
                BmcState::FactoryDefault | BmcState::SetupPending | BmcState::WifiReconfiguration => {
                    true
                }
                BmcState::Operational | BmcState::Unsupported => false,
            },
            || {
                format_message!(
                    intl,
                    default_message: "This is only available while the device is being set up.",
                    description: "Error when the setup's language picker is used on a device that has finished its setup",
                )
            },
        )
        .await
    }
}

impl<T> GrpcInitialSetupService for InitialSetupService<T>
where
    T: BmcManager,
{
    async fn set_wifi(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetWifiRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let state = self.require_wifi_setup(&intl).await?;

        if request.ssid.trim().is_empty() {
            return Err(bad_request(
                &intl,
                vec![violation("ssid", missing_value(&intl))],
            ));
        }

        let config = try_into_wifi_network_config(&intl, request.to_owned_message())?;
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
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.require_wifi_setup(&intl(&ctx)).await?;
        self.product_setup.skip_wifi().await?;
        Response::ok(Empty::default())
    }

    async fn scan_wifi(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ScanWifiResponse> {
        self.require_wifi_setup(&intl(&ctx)).await?;

        Response::ok(scan_wifi_response(self.manager.clone()).await?)
    }

    async fn get_settings_data(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<SettingsDataResponse> {
        self.require_device_settings_step(&intl(&ctx)).await?;

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
        ctx: RequestContext,
        request: ServiceRequest<'_, SettingsRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        self.require_device_settings_step(&intl).await?;

        self.product_setup
            .setup_device(&intl, request.to_owned_message())
            .await?;

        Response::ok(Empty::default())
    }

    async fn set_language(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetLanguageRequest>,
    ) -> ServiceResult<Empty> {
        self.require_setup(&intl(&ctx)).await?;
        save_language(&self.config_handle, request.language).await?;
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
    async fn setup_device(&self, intl: &Intl, request: SettingsRequest)
    -> Result<(), ConnectError>;
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

    async fn setup_device(
        &self,
        intl: &Intl,
        request: SettingsRequest,
    ) -> Result<(), ConnectError> {
        let config = parse_device_setup(intl, request).map_err(|v| bad_request(intl, v))?;
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

    async fn setup_device(
        &self,
        intl: &Intl,
        request: SettingsRequest,
    ) -> Result<(), ConnectError> {
        // NOTE: every field is checked before anything is written, so a rejected
        // request leaves the device untouched and names each offending field.
        let mut field_violations = vec![];

        let pool = request.pool.clone().into_option();
        match &pool {
            None => field_violations.push(violation("pool", missing_value(intl))),
            Some(pool) => {
                if url::Url::parse(&pool.url).is_err() {
                    let description = format_message!(
                        intl,
                        default_message: "Invalid URL!",
                        description: "Error under the mining pool's address field",
                    );
                    field_violations.push(violation("pool.url", description));
                }
                if pool.user.trim().is_empty() {
                    field_violations.push(violation("pool.user", missing_value(intl)));
                }
            }
        }

        let hostname = request.hostname.clone();
        if let Some(hostname) = hostname.as_deref()
            && let Err(err) = bmc_net::validate_hostname(hostname)
        {
            warn!("Refused the hostname: {err}");
            let description = format_message!(
                intl,
                default_message: "Use only letters, digits and hyphens, separated by dots",
                description: "Error under the device's hostname field",
            );
            field_violations.push(violation("hostname", description));
        }

        let network = match request.network.as_option() {
            None => None,
            Some(config) => {
                match parse_network_config(intl, config, "network.protocol", "network.static.") {
                    Ok(config) => Some(config),
                    Err(violations) => {
                        field_violations.extend(violations);
                        None
                    }
                }
            }
        };

        let device = match parse_device_setup(intl, request) {
            Ok(device) => Some(device),
            Err(violations) => {
                field_violations.extend(violations);
                None
            }
        };

        if !field_violations.is_empty() {
            return Err(bad_request(intl, field_violations));
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

fn parse_device_setup(
    intl: &Intl,
    value: SettingsRequest,
) -> Result<DeviceSetupConfig, Vec<FieldViolation>> {
    const REPORTED: &str = "BUG: an invalid field is reported as a field violation";
    let mut field_violations = vec![];

    let timezone = Timezone::from_str(&value.timezone_id).inspect_err(|_| {
        let description = format_message!(
            intl,
            default_message: "Choose a timezone",
            description: "Error under the timezone field when nothing valid is chosen",
        );
        field_violations.push(violation("timezone_id", description));
    });

    let time_system = try_from_time_format(intl, value.time_format)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let date_format = try_from_date_time(intl, value.date_format)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let number_format = try_from_number_format(intl, value.number_format)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let temperature_unit = try_from_temperature_unit(intl, value.temperature_unit)
        .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

    let unit_system = try_from_unit_system(intl, value.unit_system)
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
    intl: &Intl,
    value: EnumValue<TimeFormat>,
) -> Result<bmc_shared_time::time::TimeSystem, FieldViolation> {
    match value
        .as_known()
        .unwrap_or(TimeFormat::TimeFormatUnspecified)
    {
        TimeFormat::TimeFormatUnspecified => Err(violation(
            "time_format",
            format_message!(
                intl,
                default_message: "Choose a time format",
                description: "Error under the 12 or 24 hour clock field when nothing is chosen",
            ),
        )),
        TimeFormat::TimeFormat12Hour => Ok(TimeSystem::Hour12),
        TimeFormat::TimeFormat24Hour => Ok(TimeSystem::Hour24),
    }
}

pub(crate) fn try_from_date_time(
    intl: &Intl,
    value: EnumValue<DateFormat>,
) -> Result<bmc_shared_time::time::DateFormat, FieldViolation> {
    match value.as_known().unwrap_or(DateFormat::Unspecified) {
        DateFormat::Unspecified => Err(violation(
            "date_format",
            format_message!(
                intl,
                default_message: "Choose a date format",
                description: "Error under the date format field when nothing is chosen",
            ),
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
    intl: &Intl,
    value: EnumValue<TemperatureUnit>,
) -> Result<ConfigTemperatureUnit, FieldViolation> {
    match value.as_known().unwrap_or(TemperatureUnit::Unspecified) {
        TemperatureUnit::Unspecified => Err(violation(
            "temperature_unit",
            format_message!(
                intl,
                default_message: "Choose a temperature unit",
                description: "Error under the temperature unit field when nothing is chosen",
            ),
        )),
        TemperatureUnit::Celsius => Ok(ConfigTemperatureUnit::Celsius),
        TemperatureUnit::Fahrenheit => Ok(ConfigTemperatureUnit::Fahrenheit),
    }
}

fn try_from_unit_system(
    intl: &Intl,
    value: EnumValue<UnitSystem>,
) -> Result<ConfigUnitSystem, FieldViolation> {
    match value.as_known().unwrap_or(UnitSystem::Unspecified) {
        UnitSystem::Unspecified => Err(violation(
            "unit_system",
            format_message!(
                intl,
                default_message: "Choose a unit system",
                description: "Error under the metric or imperial field when nothing is chosen",
            ),
        )),
        UnitSystem::Metric => Ok(ConfigUnitSystem::Metric),
        UnitSystem::Imperial => Ok(ConfigUnitSystem::Imperial),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::grpc::tests::violation_fields;
    use bmc_grpc::web::{NetworkConfig, NetworkConfigStatic, network_config::Protocol};
    use bmc_intl::Language;

    fn english() -> Arc<Intl> {
        bmc_intl::shared_formatter(Language::EN)
    }

    #[test]
    fn an_empty_form_names_every_unchosen_field_in_english() {
        let violations = parse_device_setup(&english(), SettingsRequest::default())
            .expect_err("BUG: an empty form leaves every field unchosen");

        let found: Vec<(&str, &str)> = violations
            .iter()
            .map(|v| (v.field.as_str(), v.description.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("timezone_id", "Choose a timezone"),
                ("time_format", "Choose a time format"),
                ("date_format", "Choose a date format"),
                ("number_format", "Choose a number format"),
                ("temperature_unit", "Choose a temperature unit"),
                ("unit_system", "Choose a unit system"),
            ]
        );
    }

    #[test]
    fn a_request_asking_for_chinese_has_every_unchosen_field_named_in_it() {
        let violations = parse_device_setup(
            &crate::web::grpc::intl::asking_for("zh"),
            SettingsRequest::default(),
        )
        .expect_err("BUG: an empty form leaves every field unchosen");
        let english = parse_device_setup(&english(), SettingsRequest::default())
            .expect_err("BUG: an empty form leaves every field unchosen");

        assert_eq!(violations.len(), english.len());
        for (worded, english) in violations.iter().zip(&english) {
            assert_eq!(worded.field, english.field);
            assert_ne!(worded.description, english.description, "worded in English");
        }
    }

    #[test]
    fn a_wrong_address_quotes_what_was_typed() {
        let config = NetworkConfig {
            protocol: Some(Protocol::Static(NetworkConfigStatic {
                address: "not-an-address".to_owned(),
                netmask: "255.255.255.0".to_owned(),
                gateway: "192.168.1.1".to_owned(),
                dns_servers: vec![],
            })),
        };

        let violations = parse_network_config(&english(), &config, "protocol", "")
            .expect_err("BUG: the address does not parse");

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "address");
        assert_eq!(
            violations[0].description,
            "not-an-address is not a valid IPv4."
        );
    }

    #[test]
    fn a_refused_form_says_so_above_its_fields() {
        let error = bad_request(
            &english(),
            vec![violation("ssid", missing_value(&english()))],
        );

        assert_eq!(
            error.message.as_deref(),
            Some("Some of the fields are invalid.")
        );
        assert_eq!(violation_fields(&error), ["ssid"]);
    }
}

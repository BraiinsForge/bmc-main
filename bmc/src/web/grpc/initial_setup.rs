// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    DateFormat, EncryptionType as GrpcEncryptionType, NumberFormat, ScanWifiResponse,
    SetWifiRequest, SettingsDataResponse, SettingsRequest, SignalStrength as GrpcSignalStrength,
    TimeFormat, WifiNetwork,
    initial_setup_service_server::InitialSetupService as GrpcInitialSetupService,
};
use bmc_shared_ii_net::wifi::{EncryptionType, SignalStrength};
use bmc_shared_time::time::{TimeSystem, Timezone};
use std::{str::FromStr, sync::Arc};
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, FieldViolation, StatusExt};
use tracing::warn;

use super::{GrpcError, system::into_grpc_timezone};
use crate::{
    BmcManager,
    initial_setup::{DeviceSetupConfig, InitialSetup, SetupError},
    manager::{BmcState, WifiNetworkConfig},
};

#[derive(Clone)]
pub(crate) struct InitialSetupService<T>
where
    T: BmcManager,
{
    manager: Arc<T>,
    initial_setup: InitialSetup<T>,
}

impl<T> InitialSetupService<T>
where
    T: BmcManager,
{
    pub(crate) fn new(manager: Arc<T>, initial_setup: InitialSetup<T>) -> Self {
        Self {
            manager,
            initial_setup,
        }
    }

    async fn check_precondition(&self, state: BmcState) -> Result<(), Status> {
        if self.manager.device_state().await != state {
            return Err(Status::failed_precondition(format!(
                "Function is only available when the device is in '{state}' state.",
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<T> GrpcInitialSetupService for InitialSetupService<T>
where
    T: BmcManager,
{
    async fn set_wifi(&self, request: Request<SetWifiRequest>) -> Result<Response<()>, Status> {
        self.check_precondition(BmcState::FactoryDefault).await?;

        let request = request.into_inner();

        let config = try_into_network_config(request)?;

        match self.initial_setup.connect_to_wifi(config) {
            Ok(()) => Ok(Response::new(())),
            Err(e) => match e {
                WifiSetupError::InProgress => {
                    Err(Status::failed_precondition("WiFi setup is in progress"))
                }
            },
        }
    }

    async fn scan_wifi(&self, _request: Request<()>) -> Result<Response<ScanWifiResponse>, Status> {
        self.check_precondition(BmcState::FactoryDefault).await?;

        let available_wifi = self.manager.wifi_scan().await.map_err(|e| {
            warn!("Failed to scan WiFi networks: {}", e);
            Status::internal("Failed to scan WiFi networks")
        })?;

        Ok(Response::new(ScanWifiResponse {
            networks: available_wifi
                .into_iter()
                .filter(|wifi| wifi.signal_strength() != SignalStrength::Offline)
                .map(|wifi| WifiNetwork {
                    signal_strength: into_grpc_signal_strength(wifi.signal_strength()) as i32,
                    ssid: wifi.ssid,
                    encryption_type: into_encryption_type(wifi.encryption_type) as i32,
                })
                .collect(),
        }))
    }

    async fn get_settings_data(
        &self,
        _request: Request<()>,
    ) -> Result<Response<SettingsDataResponse>, Status> {
        self.check_precondition(BmcState::SetupPending).await?;

        let timezones = self
            .manager
            .timezone_list()
            .map(|tz| into_grpc_timezone(&tz))
            .collect();

        Ok(Response::new(SettingsDataResponse {
            timezones,
            timezone_id: Timezone::default().iana.to_owned(),
            data_collection: Some(true),
            time_format: TimeFormat::TimeFormat24Hour.into(),
            date_format: DateFormat::DdMmYyyyDot.into(),
            number_format: NumberFormat::SpaceGroupCommaDecimal.into(),
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
                | DeviceSetupError::UpdateDeviceState(..) => {
                    Status::internal("Error while saving device settings")
                }
            })?;

        Ok(Response::new(()))
    }
}

fn try_into_network_config(request: SetWifiRequest) -> Result<WifiNetworkConfig, Status> {
    let encryption = try_into_encryption_type(request.encryption_type())?;

    Ok(WifiNetworkConfig {
        ssid: request.ssid,
        password: request.password,
        encryption,
    })
}

fn try_into_encryption_type(value: GrpcEncryptionType) -> Result<EncryptionType, Status> {
    match value {
        GrpcEncryptionType::Unspecified => Err(Status::with_error_details(
            Code::InvalidArgument,
            GrpcError::BadRequest.to_string(),
            ErrorDetails::with_bad_request_violation(
                "encryption_type",
                "Encryption type must be specified and cannot be UNSPECIFIED.",
            ),
        )),
        GrpcEncryptionType::None => Ok(EncryptionType::None),
        GrpcEncryptionType::Wep => Ok(EncryptionType::Wep),
        GrpcEncryptionType::WepShared => Ok(EncryptionType::WepShared),
        GrpcEncryptionType::Wpa => Ok(EncryptionType::Wpa),
        GrpcEncryptionType::Wpa12 => Ok(EncryptionType::Wpa1_2),
        GrpcEncryptionType::Wpa2 => Ok(EncryptionType::Wpa2),
        GrpcEncryptionType::Wpa23 => Ok(EncryptionType::Wpa2_3),
        GrpcEncryptionType::Wpa3 => Ok(EncryptionType::Wpa3),
    }
}

fn into_encryption_type(value: EncryptionType) -> GrpcEncryptionType {
    match value {
        EncryptionType::None => GrpcEncryptionType::None,
        EncryptionType::Wep => GrpcEncryptionType::Wep,
        EncryptionType::WepShared => GrpcEncryptionType::WepShared,
        EncryptionType::Wpa => GrpcEncryptionType::Wpa,
        EncryptionType::Wpa1_2 => GrpcEncryptionType::Wpa12,
        EncryptionType::Wpa2 => GrpcEncryptionType::Wpa2,
        EncryptionType::Wpa2_3 => GrpcEncryptionType::Wpa23,
        EncryptionType::Wpa3 => GrpcEncryptionType::Wpa3,
    }
}

fn into_grpc_signal_strength(value: SignalStrength) -> GrpcSignalStrength {
    match value {
        SignalStrength::Offline => GrpcSignalStrength::Unspecified, // Offline WiFi is filtered out
        SignalStrength::Low => GrpcSignalStrength::Weak,
        SignalStrength::Fair => GrpcSignalStrength::Moderate,
        SignalStrength::Excellent => GrpcSignalStrength::Strong,
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

        let number_format = value
            .number_format()
            .try_into()
            .inspect_err(|e: &FieldViolation| field_violations.push(e.clone()));

        let err = Status::with_error_details(
            Code::InvalidArgument,
            GrpcError::BadRequest.to_string(),
            ErrorDetails::with_bad_request(field_violations),
        );

        let timezone = timezone.map_err(|_| err.clone())?;
        let time_system = time_system.map_err(|_| err.clone())?;
        let date_format = date_format.map_err(|_| err.clone())?;
        let number_format = number_format.map_err(|_| err)?;

        Ok(DeviceSetupConfig {
            timezone,
            system_password: value.password,
            time_system,
            number_format,
            date_format,
            data_collection: value.data_collection,
        })
    }
}

fn try_from_time_format(
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

fn try_from_date_time(
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

impl TryFrom<NumberFormat> for crate::utils::NumberFormat {
    type Error = FieldViolation;

    fn try_from(value: NumberFormat) -> Result<Self, Self::Error> {
        match value {
            NumberFormat::Unspecified => Err(FieldViolation::new(
                "number_format",
                "number_format cannot be unspecified",
            )),
            NumberFormat::SpaceGroupCommaDecimal => {
                Ok(crate::utils::NumberFormat::SpaceGroupCommaDecimal)
            }
            NumberFormat::CommaGroupDotDecimal => {
                Ok(crate::utils::NumberFormat::CommaGroupDotDecimal)
            }
            NumberFormat::DotGroupCommaDecimal => {
                Ok(crate::utils::NumberFormat::DotGroupCommaDecimal)
            }
            NumberFormat::SpaceGroupDotDecimal => {
                Ok(crate::utils::NumberFormat::SpaceGroupDotDecimal)
            }
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    EncryptionType as GrpcEncryptionType, ScanWifiResponse, SetWifiRequest,
    SignalStrength as GrpcSignalStrength, WifiNetwork,
    initial_setup_service_server::InitialSetupService as GrpcInitialSetupService,
};
use std::sync::Arc;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::warn;

use super::GrpcError;
use crate::{
    BmcManager,
    initial_setup::{InitialSetup, SetupError},
    manager::{BmcState, EncryptionType, SignalStrength, WifiNetworkConfig},
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
                SetupError::InProgress => {
                    Err(Status::failed_precondition("Initial setup is in progress"))
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

    async fn pending_setup(&self, _request: Request<()>) -> Result<Response<bool>, Status> {
        let pending_setup = match self.manager.device_state().await {
            BmcState::FactoryDefault | crate::manager::BmcState::Operational => false,
            BmcState::SetupPending => true,
        };

        Ok(Response::new(pending_setup))
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

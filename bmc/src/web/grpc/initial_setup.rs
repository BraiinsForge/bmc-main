// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    EncryptionType as GrpcEncryptionType, SetWifiRequest,
    initial_setup_service_server::InitialSetupService as GrpcInitialSetupService,
};
use std::sync::Arc;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};

use super::GrpcError;
use crate::{
    BmcManager,
    initial_setup::{InitialSetup, SetupError},
    manager::{EncryptionType, WifiNetworkConfig},
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
}

#[async_trait::async_trait]
impl<T> GrpcInitialSetupService for InitialSetupService<T>
where
    T: BmcManager,
{
    async fn set_wifi(&self, request: Request<SetWifiRequest>) -> Result<Response<()>, Status> {
        if !self.manager.is_factory_default().await {
            return Err(Status::failed_precondition(
                "Initial setup is only available when the device is in its factory default state.",
            ));
        }

        let request = request.into_inner();

        let config = try_into_network_config(request)?;

        match self.initial_setup.connect_to_wifi(config) {
            Ok(()) => Ok(Response::new(())),
            Err(e) => match e {
                SetupError::InProgress => Err(Status::failed_precondition(
                    "Initial setup is already being set up",
                )),
            },
        }
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

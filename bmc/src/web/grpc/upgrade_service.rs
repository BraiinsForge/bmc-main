// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    CheckForUpgradeResponse, DownloadFinished, DownloadFirmwareRequest, DownloadFirmwareResponse,
    UpgradeRequest, download_firmware_response,
    upgrade_service_server::UpgradeService as GrpcUpgradeService,
};
use bmc_upgrade::firmware::{FirmwareIndex, ReleaseInfo, UpgradeDetail, UpgradeMetadata};
use chrono::NaiveTime;
use futures::stream::{BoxStream, StreamExt};
use prost_types::Timestamp;
use tonic::{Code, Request, Status};
use tonic_types::{ErrorDetails, StatusExt};

use crate::{
    BmcManager,
    system_upgrade::{DownloadState, SystemUpgradeError},
};
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::{GrpcError, SystemUpgradeService};

pub(crate) struct UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    system_upgrade_service: SystemUpgradeService<U, T>,
}

impl<T, U> UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    pub(crate) fn new(system_upgrade_service: SystemUpgradeService<U, T>) -> Self {
        Self {
            system_upgrade_service,
        }
    }
}

#[tonic::async_trait]
impl<T, U> GrpcUpgradeService for UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    type DownloadFirmwareStream =
        BoxStream<'static, Result<DownloadFirmwareResponse, tonic::Status>>;

    async fn check_for_upgrade(
        &self,
        _request: Request<()>,
    ) -> Result<tonic::Response<CheckForUpgradeResponse>, tonic::Status> {
        let available_releases = self
            .system_upgrade_service
            .check_for_upgrade()
            .await
            .map_err(Into::<tonic::Status>::into)?;

        let result = map_available_releases(available_releases);

        Ok(tonic::Response::new(result))
    }

    async fn download_firmware(
        &self,
        request: Request<DownloadFirmwareRequest>,
    ) -> Result<tonic::Response<Self::DownloadFirmwareStream>, tonic::Status> {
        let request = request.into_inner();

        let rx = self.system_upgrade_service.download_firmware(request.hash);

        let stream = UnboundedReceiverStream::new(rx).map(map_download_state);

        Ok(tonic::Response::new(stream.boxed()))
    }

    async fn upgrade(
        &self,
        request: Request<UpgradeRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let request = request.into_inner();

        self.system_upgrade_service
            .verify_and_upgrade(&request.hash)
            .await
            .map_err(Into::<tonic::Status>::into)?;

        Ok(tonic::Response::new(()))
    }
}

fn map_available_releases(upgrade_detail: Option<UpgradeDetail>) -> CheckForUpgradeResponse {
    upgrade_detail
        .map(|upgrade_detail| CheckForUpgradeResponse {
            latest_release: Some(map_upgrade_metadata(upgrade_detail.latest_release)),
            previous_releases: upgrade_detail
                .previous_releases
                .into_iter()
                .map(map_release_info)
                .collect(),
        })
        .unwrap_or_default()
}

fn map_upgrade_metadata(value: UpgradeMetadata) -> bmc_grpc::web::UpgradeMetadata {
    let release_date = value.release_date.and_time(NaiveTime::MIN).and_utc();

    bmc_grpc::web::UpgradeMetadata {
        hash: value.hash,
        version: value.version,
        release_date: Some(Timestamp {
            seconds: release_date.timestamp(),
            nanos: 0,
        }),
        description: value.description,
    }
}

fn map_release_info(value: ReleaseInfo) -> bmc_grpc::web::ReleaseInfo {
    bmc_grpc::web::ReleaseInfo {
        version: value.version,
        description: value.description,
    }
}

fn map_download_state(state: DownloadState) -> Result<DownloadFirmwareResponse, tonic::Status> {
    match state {
        DownloadState::Progress {
            downloaded_mb,
            total_mb,
        } => Ok(DownloadFirmwareResponse {
            state: Some(download_firmware_response::State::DownloadProgress(
                bmc_grpc::web::DownloadProgress {
                    downloaded_mb,
                    total_mb,
                },
            )),
        }),
        DownloadState::Finished { hash } => Ok(DownloadFirmwareResponse {
            state: Some(download_firmware_response::State::DownloadFinished(
                DownloadFinished { hash },
            )),
        }),
        DownloadState::Failed(error) => Err(error.into()),
    }
}

impl From<SystemUpgradeError> for Status {
    fn from(value: SystemUpgradeError) -> Self {
        match value {
            SystemUpgradeError::NoImageWithHash => Status::with_error_details(
                Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request_violation("hash", value.to_string()),
            ),
            SystemUpgradeError::DownloadedImageHashMismatch { .. }
            | SystemUpgradeError::VerifyFailed
            | SystemUpgradeError::UpgradeInProgress
            | SystemUpgradeError::FailedToDownload(_)
            | SystemUpgradeError::UnableToCheckForUpgrade(_)
            | SystemUpgradeError::UpgradeFailed => Status::internal(value.to_string()),
            SystemUpgradeError::NotEnoughSpace => Status::failed_precondition(value.to_string()),
        }
    }
}

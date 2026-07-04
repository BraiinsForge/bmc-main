// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    AutoUpgradeFrequency as GrpcAutoUpgradeFrequency, CheckForUpgradeResponse, DownloadFinished,
    DownloadFirmwareRequest, DownloadFirmwareResponse, GetAutoUpgradeResponse,
    SetAutoUpgradeRequest, UpgradeRequest, download_firmware_response,
    upgrade_service_server::UpgradeService as GrpcUpgradeService,
};
use bmc_upgrade::firmware::{FirmwareIndex, ReleaseInfo, UpgradeDetail, UpgradeMetadata};
use chrono::{NaiveTime, TimeDelta, Timelike};
use futures::stream::{BoxStream, StreamExt};
use prost_types::Timestamp;
use std::ops::Add;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Code, Request, Status};
use tonic_types::{ErrorDetails, StatusExt};

use super::{GrpcError, SystemUpgradeService};
use crate::config::ConfigHandle;
use crate::{
    BmcManager,
    system_upgrade::{DownloadState, SystemUpgradeError},
};
use bmc_upgrade::autoupgrade::{AutoUpgradeConfig, AutoUpgradeFrequency};
use tokio_stream::wrappers::UnboundedReceiverStream;

pub(crate) struct UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    manager: Arc<T>,
    system_upgrade: SystemUpgradeService<U, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
}

impl<T, U> UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    pub(crate) fn new(
        manager: Arc<T>,
        system_upgrade: SystemUpgradeService<U, T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) -> Self {
        Self {
            manager,
            system_upgrade,
            config_handle,
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
            .system_upgrade
            .check_for_firmware_upgrade()
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

        let rx = self.system_upgrade.download_firmware(request.hash);

        let stream = UnboundedReceiverStream::new(rx).map(map_download_state);

        Ok(tonic::Response::new(stream.boxed()))
    }

    async fn upgrade(
        &self,
        request: Request<UpgradeRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let request = request.into_inner();

        self.system_upgrade
            .verify_and_upgrade(&request.hash)
            .await
            .map_err(Into::<tonic::Status>::into)?;

        Ok(tonic::Response::new(()))
    }

    async fn set_auto_upgrade(
        &self,
        request: Request<SetAutoUpgradeRequest>,
    ) -> Result<tonic::Response<()>, Status> {
        let req = request.get_ref();
        let frequency = req.frequency().into();
        let timezone = self.manager.timezone();
        let (hour, minute) = (req.hour, req.minute);
        let time_of_day = map_hour_minute_to_naive_time(hour, minute).into();
        let config = AutoUpgradeConfig::new(
            req.enabled,
            frequency,
            time_of_day,
            timezone.chrono_offset(),
        );

        self.system_upgrade
            .autoupgrade_reschedule(config.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        self.config_handle.write().await.set_autoupgrade(config);

        Ok(tonic::Response::new(()))
    }

    async fn get_auto_upgrade(
        &self,
        _request: Request<()>,
    ) -> Result<tonic::Response<GetAutoUpgradeResponse>, Status> {
        let autoupgrade_config = self.config_handle.read().await.autoupgrade();
        let (frequency, hour_minute) = if let Some(cron) = autoupgrade_config.cron {
            (
                GrpcAutoUpgradeFrequency::from(AutoUpgradeFrequency::from(&cron)).into(),
                self.system_upgrade
                    .get_autoupgrade_next_run()
                    .await
                    .map(map_datetime_to_hour_minute),
            )
        } else {
            (None, None)
        };
        let (hour, minute) = hour_minute.unwrap_or((None, None));
        let response = GetAutoUpgradeResponse {
            enabled: autoupgrade_config.enabled,
            frequency: frequency.map(Into::into),
            hour,
            minute,
        };

        Ok(tonic::Response::new(response))
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
            SystemUpgradeError::FailedToDetectCurrentVersion
            | SystemUpgradeError::DownloadedImageHashMismatch { .. }
            | SystemUpgradeError::VerifyFailed
            | SystemUpgradeError::FailedToDownload(_)
            | SystemUpgradeError::UnableToCheckForUpgrade(_)
            | SystemUpgradeError::UpgradeFailed
            | SystemUpgradeError::PackageUpgradeFailed(_)
            | SystemUpgradeError::PackageIndexFetchFailed(_) => Status::internal(value.to_string()),
            // Another run holds the gate; the condition clears by itself,
            // so the client sees "try again", not a server fault.
            SystemUpgradeError::UpgradeInProgress => Status::unavailable(value.to_string()),
            SystemUpgradeError::NotEnoughSpace | SystemUpgradeError::UpgradeExpired => {
                Status::failed_precondition(value.to_string())
            }
        }
    }
}

fn map_hour_minute_to_naive_time(hour: Option<u32>, minute: Option<u32>) -> NaiveTime {
    NaiveTime::default()
        .add(TimeDelta::hours(i64::from(hour.unwrap_or_default())))
        .add(TimeDelta::minutes(i64::from(minute.unwrap_or_default())))
}

fn map_datetime_to_hour_minute(value: chrono::DateTime<chrono::Utc>) -> (Option<u32>, Option<u32>) {
    let naive_time = value.naive_utc();
    let hour = naive_time.hour().into();
    let minute = naive_time.minute().into();
    (hour, minute)
}

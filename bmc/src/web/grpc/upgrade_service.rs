// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::{
    AutoUpgradeFrequency as GrpcAutoUpgradeFrequency, CheckForUpgradeRequest,
    CheckForUpgradeResponse, FirmwareUpgrade, FirmwareUpgradePhase, GetAutoUpgradeResponse,
    GetInstallableWidgetsResponse, InstallablePreview, InstallableWidget, PackageChange,
    PackageUpgradePhase, PackageUpgradePlan, SetAutoUpgradeRequest, StartUpgradeRequest,
    UpgradeDisruption, UpgradeDownloadProgress, UpgradeProgress, upgrade_progress,
    upgrade_service_server::UpgradeService as GrpcUpgradeService,
};
use bmc_upgrade::firmware::{FirmwareIndex, ReleaseInfo, UpgradeDetail};
use chrono::{NaiveTime, TimeDelta, Timelike};
use futures::stream::{BoxStream, StreamExt};
use prost_types::Timestamp;
use std::ops::Add;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Status};

use super::SystemUpgradeService;
use crate::config::ConfigHandle;
use crate::{
    BmcManager,
    system_upgrade::{
        CheckOutcome, Disruption, PackagesPreview, SystemPackageChange, SystemUpgradeError,
        SystemUpgradePhase, UpgradeRunState,
    },
};
use bmc_upgrade::autoupgrade::{AutoUpgradeConfig, AutoUpgradeFrequency};

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
    type StartUpgradeStream = BoxStream<'static, Result<UpgradeProgress, tonic::Status>>;

    async fn check_for_upgrade(
        &self,
        request: Request<CheckForUpgradeRequest>,
    ) -> Result<tonic::Response<CheckForUpgradeResponse>, tonic::Status> {
        let install = request.into_inner().install_packages;
        let outcome = self
            .system_upgrade
            .check_for_upgrade(install)
            .await
            .map_err(Into::<tonic::Status>::into)?;

        Ok(tonic::Response::new(outcome_to_response(outcome)))
    }

    async fn get_installable_widgets(
        &self,
        _request: Request<()>,
    ) -> Result<tonic::Response<GetInstallableWidgetsResponse>, tonic::Status> {
        let widgets = self
            .system_upgrade
            .list_installable_widgets()
            .await
            .map_err(Into::<tonic::Status>::into)?;
        Ok(tonic::Response::new(GetInstallableWidgetsResponse {
            widgets: widgets.into_iter().map(map_installable_widget).collect(),
        }))
    }

    async fn start_upgrade(
        &self,
        request: Request<StartUpgradeRequest>,
    ) -> Result<tonic::Response<Self::StartUpgradeStream>, tonic::Status> {
        let request = request.into_inner();

        let run = self.system_upgrade.start_upgrade(request.upgrade_id).await;

        let stream = run.map(run_state_to_progress);

        Ok(tonic::Response::new(stream.boxed()))
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

fn outcome_to_response(outcome: CheckOutcome) -> CheckForUpgradeResponse {
    let disruption = match outcome.disruption {
        Disruption::AppRestart => UpgradeDisruption::AppRestart,
        Disruption::Reboot => UpgradeDisruption::Reboot,
        Disruption::Unspecified => UpgradeDisruption::Unspecified,
    };

    CheckForUpgradeResponse {
        upgrade_id: outcome.upgrade_id,
        firmware: outcome.firmware.map(map_firmware_upgrade),
        packages: outcome.packages.map(map_package_upgrade_plan),
        disruption: disruption.into(),
    }
}

fn map_firmware_upgrade(detail: UpgradeDetail) -> FirmwareUpgrade {
    let release = detail.latest_release;
    let release_date = release.release_date.and_time(NaiveTime::MIN).and_utc();

    FirmwareUpgrade {
        hash: release.hash,
        version: release.version,
        release_date: Some(Timestamp {
            seconds: release_date.timestamp(),
            nanos: 0,
        }),
        description: release.description,
        file_size_bytes: release.file_size as u64,
        previous_releases: detail
            .previous_releases
            .into_iter()
            .map(map_release_info)
            .collect(),
    }
}

fn map_release_info(value: ReleaseInfo) -> bmc_grpc::web::ReleaseInfo {
    bmc_grpc::web::ReleaseInfo {
        version: value.version,
        description: value.description,
    }
}

fn map_package_upgrade_plan(preview: PackagesPreview) -> PackageUpgradePlan {
    PackageUpgradePlan {
        changes: preview
            .changes
            .into_iter()
            .map(map_package_change)
            .collect(),
        download_size_bytes: preview.download_size_bytes,
        bmc_version: preview.bmc_version,
        bmc_changelog: preview.bmc_changelog,
    }
}

fn installable_category_to_proto(c: bmc_upgrade::packages::InstallableCategory) -> i32 {
    use bmc_upgrade::packages::InstallableCategory as Cat;
    let proto = match c {
        Cat::Known(k) => super::scene_management::category_to_proto(k),
        Cat::Unknown => bmc_grpc::web::WidgetCategory::Unspecified,
    };
    proto as i32
}

fn map_installable_widget(w: bmc_upgrade::packages::InstallableWidget) -> InstallableWidget {
    InstallableWidget {
        package_name: w.package_name,
        uid: w.uid,
        version: w.version,
        display_name: w.display_name,
        subname: w.subname,
        category: installable_category_to_proto(w.category),
        description: w.description,
        icon: w.icon,
        previews: w
            .previews
            .into_iter()
            .map(|p| InstallablePreview {
                image: p.image,
                size: p.size,
            })
            .collect(),
    }
}

fn map_package_change(change: SystemPackageChange) -> PackageChange {
    PackageChange {
        name: change.name,
        version_from: change.version_from,
        version_to: change.version_to,
        category: change.category,
        changelog: change.changelog,
    }
}

fn run_state_to_progress(state: UpgradeRunState) -> Result<UpgradeProgress, Status> {
    let event = match state {
        UpgradeRunState::Phase(phase) => match phase {
            SystemUpgradePhase::FirmwareDownloading => {
                upgrade_progress::Event::FirmwarePhase(FirmwareUpgradePhase::Downloading.into())
            }
            SystemUpgradePhase::FirmwareVerifying => {
                upgrade_progress::Event::FirmwarePhase(FirmwareUpgradePhase::Verifying.into())
            }
            SystemUpgradePhase::FirmwareApplying => {
                upgrade_progress::Event::FirmwarePhase(FirmwareUpgradePhase::Applying.into())
            }
            SystemUpgradePhase::PackageRealizing => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Realizing.into())
            }
            SystemUpgradePhase::PackageVerifying => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Verifying.into())
            }
            SystemUpgradePhase::PackageBuilding => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Building.into())
            }
            SystemUpgradePhase::PackageActivating => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Activating.into())
            }
        },
        UpgradeRunState::Progress {
            downloaded_bytes,
            total_bytes,
        } => upgrade_progress::Event::Download(UpgradeDownloadProgress {
            downloaded_bytes,
            total_bytes,
        }),
        UpgradeRunState::Finished => upgrade_progress::Event::Finished(()),
        UpgradeRunState::Failed(err) => return Err(err.into()),
    };

    Ok(UpgradeProgress { event: Some(event) })
}

impl From<SystemUpgradeError> for Status {
    fn from(value: SystemUpgradeError) -> Self {
        match value {
            SystemUpgradeError::FailedToDetectCurrentVersion
            | SystemUpgradeError::DownloadedImageHashMismatch { .. }
            | SystemUpgradeError::VerifyFailed
            | SystemUpgradeError::FailedToDownload(_)
            | SystemUpgradeError::UnableToCheckForUpgrade(_)
            | SystemUpgradeError::UpgradeFailed
            | SystemUpgradeError::PackageUpgradeFailed(_)
            | SystemUpgradeError::PendingInstallWriteFailed(_) => {
                Status::internal(value.to_string())
            }
            // Configuration/manifest/index-shape/plan failures cannot proceed
            // until the package source or profile is corrected; a transient
            // fetch is a server-side transport fault.
            SystemUpgradeError::PackageCheckFailed(ref err) => {
                if err.is_transient() {
                    Status::internal(value.to_string())
                } else {
                    Status::failed_precondition(value.to_string())
                }
            }
            // Another run holds the gate; the condition clears by itself,
            // so the client sees "try again", not a server fault.
            SystemUpgradeError::UpgradeInProgress => Status::unavailable(value.to_string()),
            // The image is incompatible/unsigned/wrong-key: retrying the
            // same image will not help, so signal a precondition failure
            // the client can surface as "pick a different image".
            SystemUpgradeError::NotEnoughSpace
            | SystemUpgradeError::UpgradeExpired
            | SystemUpgradeError::InvalidImage => Status::failed_precondition(value.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_state_maps_to_wire_events() {
        let phase =
            run_state_to_progress(UpgradeRunState::Phase(SystemUpgradePhase::PackageRealizing))
                .expect("BUG: phase maps");
        assert!(matches!(
            phase.event,
            Some(upgrade_progress::Event::PackagePhase(p))
                if p == PackageUpgradePhase::Realizing as i32
        ));
        let finished =
            run_state_to_progress(UpgradeRunState::Finished).expect("BUG: finished maps");
        assert!(matches!(
            finished.event,
            Some(upgrade_progress::Event::Finished(()))
        ));
        assert!(
            run_state_to_progress(UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed))
                .is_err()
        );
    }

    #[test]
    fn up_to_date_outcome_maps_to_empty_response() {
        let response = outcome_to_response(CheckOutcome {
            firmware: None,
            packages: None,
            upgrade_id: None,
            disruption: Disruption::Unspecified,
        });
        assert!(response.upgrade_id.is_none());
        assert!(response.firmware.is_none());
        assert!(response.packages.is_none());
        assert_eq!(response.disruption(), UpgradeDisruption::Unspecified);
    }

    #[test]
    fn package_plan_failure_maps_to_failed_precondition_with_message() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::PlanFailed(
                bmc_upgrade::packages::PackagePlanFailure::MissingSystemPackages {
                    names: vec!["nix".to_owned()],
                },
            ),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            status.message(),
            "Cannot check for upgrade: package source is incomplete; \
             required system package \"nix\" is missing."
        );
    }

    #[test]
    fn no_enabled_servers_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::NoEnabledServers,
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            status.message(),
            "Cannot check for upgrade: no enabled package servers are configured."
        );
    }

    #[test]
    fn transient_index_fetch_maps_to_internal() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::IndexFetchFailed("boom".to_owned()),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn manifest_read_failure_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::ManifestReadFailed("io".to_owned()),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn unusable_index_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::IndexUnusable("bad shape".to_owned()),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}

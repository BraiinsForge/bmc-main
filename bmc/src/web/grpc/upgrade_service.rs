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

use bmc_grpc::web::{
    CheckForUpgradeRequest, CheckForUpgradeResponse, FirmwareUpgrade, FirmwareUpgradePhase,
    GetAutoUpgradeResponse, GetInstallableWidgetsResponse, InstallablePreview, InstallableWidget,
    PackageChange, PackageUpgradePhase, PackageUpgradePlan, SetAutoUpgradeRequest,
    StartUpgradeRequest, UpgradeDisruption, UpgradeDownloadProgress, UpgradeProgress,
    upgrade_progress, upgrade_service_server::UpgradeService as GrpcUpgradeService,
};
use bmc_platform::HardwareCapabilities;
use bmc_upgrade::autoupgrade::AutoUpgradeConfig;
use bmc_upgrade::firmware::{FirmwareIndex, ReleaseInfo, UpgradeDetail};
use chrono::NaiveTime;
use futures::stream::{BoxStream, StreamExt};
use prost_types::Timestamp;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tonic::{Request, Status};
use tracing::error;

use super::SystemUpgradeService;
use super::scene_management::{PlatformDescriptor, supported_sizes_for_constraints};
use crate::BmcManager;
use crate::compositor::UpgradePhase;
use crate::config::ConfigHandle;
use crate::system_upgrade::{
    CheckOutcome, Disruption, PackagesPreview, SystemPackageChange, SystemUpgradeError,
    UpgradeRunState,
};
pub(crate) struct UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    system_upgrade: SystemUpgradeService<U, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    platform: PlatformDescriptor,
    /// Serializes the save-apply-rollback transition in `set_auto_upgrade`;
    /// the config lock alone cannot, since it is released before the
    /// scheduler call.
    autoupgrade_transition: Mutex<()>,
}

impl<T, U> UpgradeService<T, U>
where
    T: BmcManager,
    U: FirmwareIndex,
{
    pub(crate) fn new(
        system_upgrade: SystemUpgradeService<U, T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        hardware_capabilities: &HardwareCapabilities,
    ) -> Self {
        Self {
            system_upgrade,
            config_handle,
            platform: PlatformDescriptor::from(hardware_capabilities),
            autoupgrade_transition: Mutex::new(()),
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
            widgets: widgets
                .into_iter()
                .map(|widget| map_installable_widget(&self.platform, widget))
                .collect(),
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
        let enabled = request.get_ref().enabled;
        let _transition = self.autoupgrade_transition.lock().await;
        let config = self
            .system_upgrade
            .create_autoupgrade_config(enabled)
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut guard = self.config_handle.write().await;
        let previous_config = guard.autoupgrade();
        guard.set_autoupgrade(config);
        if let Err(err) = guard.save().await {
            guard.set_autoupgrade(previous_config);
            return Err(Status::internal(err.to_string()));
        }
        // Released before the scheduler call below so config readers do not
        // block on it.
        drop(guard);

        if let Err(err) = self.system_upgrade.apply_autoupgrade(enabled).await {
            // A failed apply leaves the runtime disabled (gate closed, job
            // cancelled) regardless of the previous config, so persist that
            // state rather than the previous one — a re-enable failure must
            // not write back `enabled: true`.
            let mut guard = self.config_handle.write().await;
            guard.set_autoupgrade(AutoUpgradeConfig {
                enabled: false,
                cron: None,
            });
            if let Err(save_err) = guard.save().await {
                error!(
                    ?save_err,
                    "Failed to persist the disabled auto-upgrade state after a scheduling failure"
                );
            }
            return Err(Status::internal(err.to_string()));
        }

        Ok(tonic::Response::new(()))
    }

    async fn get_auto_upgrade(
        &self,
        _request: Request<()>,
    ) -> Result<tonic::Response<GetAutoUpgradeResponse>, Status> {
        let enabled = self.config_handle.read().await.autoupgrade().enabled;
        Ok(tonic::Response::new(GetAutoUpgradeResponse { enabled }))
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

fn map_installable_widget(
    platform: &PlatformDescriptor,
    w: bmc_upgrade::packages::InstallableWidget,
) -> InstallableWidget {
    let supported_sizes = supported_sizes_for_constraints(platform, &w.supported_viewports)
        .into_iter()
        .map(Into::into)
        .collect();
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
        supported_sizes,
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
            UpgradePhase::FirmwareDownloading => {
                upgrade_progress::Event::FirmwarePhase(FirmwareUpgradePhase::Downloading.into())
            }
            UpgradePhase::FirmwareVerifying => {
                upgrade_progress::Event::FirmwarePhase(FirmwareUpgradePhase::Verifying.into())
            }
            UpgradePhase::FirmwareApplying => {
                upgrade_progress::Event::FirmwarePhase(FirmwareUpgradePhase::Applying.into())
            }
            UpgradePhase::PackageRealizing => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Realizing.into())
            }
            UpgradePhase::PackageVerifying => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Verifying.into())
            }
            UpgradePhase::PackageBuilding => {
                upgrade_progress::Event::PackagePhase(PackageUpgradePhase::Building.into())
            }
            UpgradePhase::PackageActivating => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installable_supported_sizes_are_calculated_from_constraints() {
        let capabilities =
            bmc_platform::HardwareProfile::for_product(bmc_platform::Product::Bmc100)
                .capabilities();
        let platform = super::super::scene_management::PlatformDescriptor::from(&capabilities);
        let widget = bmc_upgrade::packages::InstallableWidget {
            package_name: "widget-fullscreen".to_owned(),
            uid: "uid-fullscreen".to_owned(),
            version: "1.0.0".to_owned(),
            display_name: "Fullscreen".to_owned(),
            subname: None,
            category: bmc_upgrade::packages::InstallableCategory::Unknown,
            description: None,
            icon: None,
            previews: Vec::new(),
            supported_viewports: vec![bmc_widget_manifest::WidgetViewportConstraint {
                viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
                min_width: Some(1280),
                max_width: Some(1280),
                min_height: Some(480),
                max_height: Some(480),
                min_dpi: None,
                max_dpi: None,
            }],
        };

        let mapped = map_installable_widget(&platform, widget);

        assert_eq!(
            mapped.supported_sizes,
            vec![bmc_grpc::web::WidgetSize::Full as i32]
        );
    }

    #[test]
    fn run_state_maps_to_wire_events() {
        let phase = run_state_to_progress(UpgradeRunState::Phase(UpgradePhase::PackageRealizing))
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

    #[test]
    fn install_target_unavailable_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::InstallTargetUnavailable(
                "package 'widget-nope' not found in any index".to_owned(),
            ),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            status.message(),
            "Cannot check for upgrade: requested package to install is unavailable: \
             package 'widget-nope' not found in any index."
        );
    }

    #[test]
    fn servers_config_unavailable_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::ServersConfigUnavailable("io".to_owned()),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn unrealizable_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::PackageCheckFailed(
            bmc_upgrade::packages::PackageProbeError::Unrealizable("/nix/store/x".to_owned()),
        )
        .into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn upgrade_in_progress_maps_to_unavailable() {
        let status: Status = SystemUpgradeError::UpgradeInProgress.into();
        assert_eq!(status.code(), tonic::Code::Unavailable);
    }

    #[test]
    fn not_enough_space_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::NotEnoughSpace.into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn invalid_image_maps_to_failed_precondition() {
        let status: Status = SystemUpgradeError::InvalidImage.into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}

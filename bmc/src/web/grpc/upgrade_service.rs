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
    UpgradeService as GrpcUpgradeService, upgrade_progress,
};
use bmc_intl::{Intl, format_message};
use bmc_platform::HardwareCapabilities;
use bmc_upgrade::autoupgrade::AutoUpgradeConfig;
use bmc_upgrade::firmware::{FirmwareDownloadError, FirmwareIndex, ReleaseInfo, UpgradeDetail};
use bmc_upgrade::packages::{PackagePlanFailure, PackageProbeError};
use buffa_types::google::protobuf::{Empty, Timestamp};
use chrono::NaiveTime;
use connectrpc::{
    ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
};
use futures::stream::StreamExt;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, warn};

use super::SystemUpgradeService;
use super::intl::intl;
use super::scene_management::{PlatformDescriptor, supported_sizes_for_constraints};
use super::shared::{StatusExt as _, ToStatus, internal_error, version_undetected};
use crate::BmcManager;
use crate::compositor::UpgradePhase;
use crate::config::ConfigHandle;
use crate::installable_widgets;
use crate::session::{Handle as _, Manager as SessionManager};
use crate::system_upgrade::boser::UpgradeRoute;
use crate::system_upgrade::{
    CheckOutcome, Disruption, PackagesPreview, SystemPackageChange, SystemUpgradeError,
    UpgradeRunState,
};
use crate::web::session::extract_session;

mod managed;

pub(crate) struct UpgradeService<T, U, S>
where
    T: BmcManager,
    U: FirmwareIndex,
    S: SessionManager,
{
    system_upgrade: SystemUpgradeService<U, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    platform: PlatformDescriptor,
    route: UpgradeRoute,
    /// Serializes the save-apply-rollback transition in `set_auto_upgrade`;
    /// the config lock alone cannot, since it is released before the
    /// scheduler call.
    autoupgrade_transition: Mutex<()>,
    session_manager: PhantomData<fn() -> S>,
}

impl<T, U, S> UpgradeService<T, U, S>
where
    T: BmcManager,
    U: FirmwareIndex,
    S: SessionManager,
{
    pub(crate) fn new(
        system_upgrade: SystemUpgradeService<U, T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        hardware_capabilities: &HardwareCapabilities,
        route: UpgradeRoute,
    ) -> Self {
        Self {
            system_upgrade,
            config_handle,
            platform: PlatformDescriptor::from(hardware_capabilities),
            route,
            autoupgrade_transition: Mutex::new(()),
            session_manager: PhantomData,
        }
    }
}

impl<T, U, S> GrpcUpgradeService for UpgradeService<T, U, S>
where
    T: BmcManager,
    U: FirmwareIndex,
    S: SessionManager,
{
    async fn check_for_upgrade(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckForUpgradeRequest>,
    ) -> ServiceResult<CheckForUpgradeResponse> {
        let intl = intl(&ctx);
        let install = request.to_owned_message().install_packages;
        match &self.route {
            UpgradeRoute::Local => {
                let outcome = self
                    .system_upgrade
                    .check_for_upgrade(install)
                    .await
                    .or_status(&intl)?;

                Response::ok(outcome_to_response(outcome))
            }
            UpgradeRoute::Boser(boser) => {
                let session_id = extract_session::<S>(ctx.headers(), ctx.extensions())?.id();
                let response = boser.check(&session_id, install).await.or_status(&intl)?;
                Response::ok(managed::check_response(response))
            }
            UpgradeRoute::BoserUnavailable => Err(managed::boser_unavailable(&intl)),
        }
    }

    async fn get_installable_widgets(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetInstallableWidgetsResponse> {
        let intl = intl(&ctx);
        let widgets = match &self.route {
            UpgradeRoute::Local => self
                .system_upgrade
                .list_installable_widgets()
                .await
                .or_status(&intl)?,
            UpgradeRoute::Boser(boser) => {
                let session_id = extract_session::<S>(ctx.headers(), ctx.extensions())?.id();
                let packages = boser.installable(&session_id).await.or_status(&intl)?;
                installable_widgets::from_packages(packages)
            }
            UpgradeRoute::BoserUnavailable => return Err(managed::boser_unavailable(&intl)),
        };
        Response::ok(GetInstallableWidgetsResponse {
            widgets: widgets
                .into_iter()
                .map(|widget| map_installable_widget(&self.platform, widget))
                .collect(),
        })
    }

    async fn start_upgrade(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartUpgradeRequest>,
    ) -> ServiceResult<ServiceStream<UpgradeProgress>> {
        let intl = intl(&ctx);
        let upgrade_id = request.upgrade_id.to_owned();
        let stream = match &self.route {
            UpgradeRoute::Local => {
                let run = self.system_upgrade.start_upgrade(upgrade_id).await;
                run.map(move |state| run_state_to_progress(&intl, state))
                    .boxed()
            }
            UpgradeRoute::Boser(boser) => {
                let session_id = extract_session::<S>(ctx.headers(), ctx.extensions())?.id();
                managed::start(intl, boser, &session_id, &upgrade_id).await
            }
            UpgradeRoute::BoserUnavailable => managed::only(managed::boser_unavailable(&intl)),
        };

        Ok(Response::stream(stream))
    }

    async fn set_auto_upgrade(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetAutoUpgradeRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let enabled = request.enabled;
        let _transition = self.autoupgrade_transition.lock().await;
        let config = self
            .system_upgrade
            .create_autoupgrade_config(enabled)
            .map_err(|err| automatic_updates_failed(&intl, &err))?;

        let mut guard = self.config_handle.write().await;
        let previous_config = guard.autoupgrade();
        guard.set_autoupgrade(config);
        if let Err(err) = guard.save().await {
            guard.set_autoupgrade(previous_config);
            return Err(automatic_updates_failed(&intl, &err));
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
            return Err(automatic_updates_failed(&intl, &err));
        }

        Response::ok(Empty::default())
    }

    async fn get_auto_upgrade(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GetAutoUpgradeResponse> {
        let enabled = self.config_handle.read().await.autoupgrade().enabled;
        Response::ok(GetAutoUpgradeResponse { enabled })
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
        firmware: outcome.firmware.map(map_firmware_upgrade).into(),
        packages: outcome.packages.map(map_package_upgrade_plan).into(),
        disruption: disruption.into(),
    }
}

fn map_firmware_upgrade(detail: UpgradeDetail) -> FirmwareUpgrade {
    let release = detail.latest_release;
    let release_date = release.release_date.and_time(NaiveTime::MIN).and_utc();

    FirmwareUpgrade {
        hash: release.hash,
        version: release.version,
        release_date: Timestamp {
            seconds: release_date.timestamp(),
            nanos: 0,
            ..Default::default()
        }
        .into(),
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

fn installable_category_to_proto(
    c: crate::installable_widgets::InstallableCategory,
) -> bmc_grpc::web::WidgetCategory {
    use crate::installable_widgets::InstallableCategory as Cat;
    match c {
        Cat::Known(k) => super::scene_management::category_to_proto(k),
        Cat::Unknown => bmc_grpc::web::WidgetCategory::Unspecified,
    }
}

fn map_installable_widget(
    platform: &PlatformDescriptor,
    w: crate::installable_widgets::InstallableWidget,
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
        category: installable_category_to_proto(w.category).into(),
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

fn run_state_to_progress(
    intl: &Intl,
    state: UpgradeRunState,
) -> Result<UpgradeProgress, ConnectError> {
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
        UpgradeRunState::Finished => upgrade_progress::Event::Finished(Empty::default()),
        UpgradeRunState::Failed(error) => return Err(error.to_status(intl)),
    };

    Ok(UpgradeProgress { event: Some(event) })
}

pub(super) fn upgrade_failed(intl: &Intl) -> String {
    format_message!(
        intl,
        default_message: "Upgrade failed!",
        description: "Error when a system upgrade that had started did not finish",
    )
}

pub(super) fn upgrade_in_progress(intl: &Intl) -> String {
    format_message!(
        intl,
        default_message: "System upgrade is in progress.",
        description: "Error when an upgrade is asked for while another is running",
    )
}

pub(super) fn upgrade_expired(intl: &Intl) -> String {
    format_message!(
        intl,
        default_message: "Upgrade id is unknown or already consumed.",
        description: "Error when starting an upgrade offered earlier that is no longer on offer; id is short for identifier",
    )
}

pub(super) fn not_enough_space(intl: &Intl) -> String {
    format_message!(
        intl,
        default_message: "Not enough space on disk to perform upgrade!",
        description: "Error when the device has too little free storage for an upgrade",
    )
}

impl ToStatus for SystemUpgradeError {
    fn to_status(&self, intl: &Intl) -> ConnectError {
        match self {
            SystemUpgradeError::FailedToDetectCurrentVersion => version_undetected(intl),
            SystemUpgradeError::DownloadedImageHashMismatch { .. } => {
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "Firmware image checksum mismatch!",
                    description: "Error when a downloaded firmware image is not the one the upgrade offered",
                ))
            }
            SystemUpgradeError::VerifyFailed => ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to verify downloaded firmware!",
                description: "Error when a downloaded firmware image cannot be checked against its expected hash",
            )),
            SystemUpgradeError::FailedToDownload(_) => ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to download firmware!",
                description: "Error when downloading a firmware image for an upgrade fails",
            )),
            SystemUpgradeError::UnableToCheckForUpgrade(download) => {
                ConnectError::internal(firmware_check_failed(intl, download))
            }
            SystemUpgradeError::UpgradeFailed => ConnectError::internal(upgrade_failed(intl)),
            SystemUpgradeError::PackageUpgradeFailed(_) => ConnectError::internal(format_message!(
                intl,
                default_message: "Package upgrade failed!",
                description: "Error when an upgrade of the device's software packages did not finish",
            )),
            SystemUpgradeError::PendingInstallWriteFailed(_) => {
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "Failed to record the pending widget install!",
                    description: "Error when the device could not save which widget it is about to install",
                ))
            }
            // Configuration/manifest/index-shape/plan failures cannot proceed
            // until the package source or profile is corrected; a transient
            // fetch is a server-side transport fault.
            SystemUpgradeError::PackageCheckFailed(probe) => {
                let message = package_check_failed(intl, probe);
                if probe.is_transient() {
                    ConnectError::internal(message)
                } else {
                    ConnectError::failed_precondition(message)
                }
            }
            // Another run holds the gate; the condition clears by itself,
            // so the client sees "try again", not a server fault.
            SystemUpgradeError::UpgradeInProgress => {
                ConnectError::unavailable(upgrade_in_progress(intl))
            }
            // The image is incompatible/unsigned/wrong-key: retrying the
            // same image will not help, so signal a precondition failure
            // the client can surface as "pick a different image".
            SystemUpgradeError::InvalidImage => ConnectError::failed_precondition(format_message!(
                intl,
                default_message: "Invalid firmware image!",
                description: "Error when a firmware image is incompatible with the device or wrongly signed",
            )),
            SystemUpgradeError::NotEnoughSpace => {
                ConnectError::failed_precondition(not_enough_space(intl))
            }
            SystemUpgradeError::UpgradeExpired => {
                ConnectError::failed_precondition(upgrade_expired(intl))
            }
            SystemUpgradeError::WidgetPackageRejected(_)
            | SystemUpgradeError::WidgetPackageTooLarge
            | SystemUpgradeError::WidgetDowngradeRefused(_)
            | SystemUpgradeError::WidgetFolderTaken(_)
            | SystemUpgradeError::WidgetUidTaken(_)
            | SystemUpgradeError::WidgetUploadStalled
            | SystemUpgradeError::WidgetUploadAborted
            | SystemUpgradeError::WidgetInstallBusy
            | SystemUpgradeError::WidgetInstallNoSpace
            | SystemUpgradeError::WidgetInstallFailed => {
                internal_error(intl, "a widget install error reached an upgrade call")
            }
        }
    }
}

fn firmware_check_failed(intl: &Intl, download: &FirmwareDownloadError) -> String {
    match download {
        FirmwareDownloadError::IndexDownloadFailed => format_message!(
            intl,
            default_message: "Cannot check for upgrade: the firmware index cannot be downloaded.",
            description: "Error when checking for an upgrade and the list of firmware versions cannot be downloaded",
        ),
        FirmwareDownloadError::FetchUpgradeDetails => format_message!(
            intl,
            default_message: "Cannot check for upgrade: the firmware index is unusable for this system version.",
            description: "Error when checking for an upgrade and the downloaded firmware index lists a version twice or offers only a downgrade across a major version",
        ),
        FirmwareDownloadError::InvalidVersion => format_message!(
            intl,
            default_message: "Cannot check for upgrade: the current system version is not recognized.",
            description: "Error when checking for an upgrade and the device's own firmware version number cannot be read",
        ),
        FirmwareDownloadError::UnsupportedPlatform => format_message!(
            intl,
            default_message: "Cannot check for upgrade: no firmware is published for this device.",
            description: "Error when checking for an upgrade on hardware no firmware image is published for",
        ),
    }
}

fn package_check_failed(intl: &Intl, probe: &PackageProbeError) -> String {
    match probe {
        PackageProbeError::NoEnabledServers => format_message!(
            intl,
            default_message: "Cannot check for upgrade: no enabled package servers are configured.",
            description: "Error when checking for an upgrade and the device has no package server turned on",
        ),
        PackageProbeError::ServersConfigUnavailable(_) => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package server configuration is unavailable.",
            description: "Error when checking for an upgrade and the list of package servers cannot be read",
        ),
        PackageProbeError::IndexFetchFailed(_) => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package index fetch failed.",
            description: "Error when checking for an upgrade and the list of available packages cannot be downloaded",
        ),
        PackageProbeError::IndexUnusable(_) => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package index is unusable.",
            description: "Error when checking for an upgrade and the downloaded list of available packages cannot be used",
        ),
        PackageProbeError::NoFeedEntry { firmware } => format_message!(
            intl,
            default_message: "Cannot check for upgrade: no packages are published for firmware {firmware} yet.",
            description: "Error when checking for an upgrade before the packages for a firmware version are released; firmware is a version such as 26.10",
            values: { firmware: firmware.as_str() },
        ),
        PackageProbeError::ManifestReadFailed(_) => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package manifest could not be read.",
            description: "Error when checking for an upgrade and the description of a package cannot be read",
        ),
        PackageProbeError::PlanFailed(PackagePlanFailure::MissingSystemPackages { names }) => {
            let quoted: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
            format_message!(
                intl,
                default_message: "{count, plural, one {Cannot check for upgrade: package source is incomplete; required system package {names} is missing.} other {Cannot check for upgrade: package source is incomplete; required system packages {names} are missing.}}",
                description: "Error when checking for an upgrade; count is how many packages are missing; names is a comma-separated list of their quoted names",
                values: {
                    count: u64::try_from(names.len()).expect("BUG: a list's length fits in u64"),
                    names: quoted.join(", "),
                },
            )
        }
        PackageProbeError::PlanFailed(PackagePlanFailure::Other(_)) => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package upgrade plan failed.",
            description: "Error when checking for an upgrade and the device cannot work out which packages to change",
        ),
        PackageProbeError::Unrealizable(_) => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package upgrade cannot be realized.",
            description: "Error when checking for an upgrade and no package server offers some of the packages it needs",
        ),
        PackageProbeError::InstallTargetUnavailable { package } => format_message!(
            intl,
            default_message: "Cannot check for upgrade: no package server offers \"{package}\".",
            description: "Error when checking for an upgrade that was to install a package no package server offers; package is its name",
            values: { package: package.as_str() },
        ),
        PackageProbeError::InstallTargetAmbiguous { package } => format_message!(
            intl,
            default_message: "Cannot check for upgrade: package servers with the same priority offer different builds of \"{package}\". Check the package server configuration.",
            description: "Error when checking for an upgrade that was to install a package which several package servers of the same priority offer in different builds of the same version; package is its name",
            values: { package: package.as_str() },
        ),
    }
}

fn automatic_updates_failed(intl: &Intl, cause: &dyn std::fmt::Display) -> ConnectError {
    warn!("Failed to change automatic updates: {cause:#}");
    ConnectError::internal(format_message!(
        intl,
        default_message: "Failed to change automatic updates!",
        description: "Error when turning automatic updates on or off fails",
    ))
}

#[cfg(test)]
mod tests {
    use buffa::EnumValue;
    use connectrpc::ErrorCode;

    use super::*;
    use crate::web::grpc::tests::{assert_worded, status_text};

    fn status(error: &SystemUpgradeError) -> ConnectError {
        error.to_status(&bmc_intl::shared_formatter(bmc_intl::Language::EN))
    }

    fn run_state_to_progress(state: UpgradeRunState) -> Result<UpgradeProgress, ConnectError> {
        super::run_state_to_progress(&bmc_intl::shared_formatter(bmc_intl::Language::EN), state)
    }

    #[test]
    fn every_upgrade_failure_is_worded() {
        let text = |value: &str| value.to_owned();
        let probes = [
            PackageProbeError::NoEnabledServers,
            PackageProbeError::ServersConfigUnavailable(text("1")),
            PackageProbeError::IndexFetchFailed(text("1")),
            PackageProbeError::IndexUnusable(text("1")),
            PackageProbeError::NoFeedEntry {
                firmware: text("1"),
            },
            PackageProbeError::ManifestReadFailed(text("1")),
            PackageProbeError::PlanFailed(PackagePlanFailure::MissingSystemPackages {
                names: vec![text("1"), text("2")],
            }),
            PackageProbeError::PlanFailed(PackagePlanFailure::Other(text("1"))),
            PackageProbeError::Unrealizable(text("1")),
            PackageProbeError::InstallTargetUnavailable { package: text("1") },
            PackageProbeError::InstallTargetAmbiguous { package: text("1") },
        ];
        assert_worded(|intl| {
            let mut refusals: Vec<String> = [
                SystemUpgradeError::FailedToDetectCurrentVersion,
                SystemUpgradeError::DownloadedImageHashMismatch {
                    expected: text("1"),
                    actual: text("2"),
                },
                SystemUpgradeError::VerifyFailed,
                SystemUpgradeError::FailedToDownload(text("1")),
                SystemUpgradeError::PackageUpgradeFailed(text("1")),
                SystemUpgradeError::PendingInstallWriteFailed(text("1")),
                SystemUpgradeError::UpgradeInProgress,
                SystemUpgradeError::InvalidImage,
                SystemUpgradeError::NotEnoughSpace,
                SystemUpgradeError::UpgradeExpired,
                SystemUpgradeError::WidgetInstallFailed,
            ]
            .iter()
            .map(|error| status_text(&error.to_status(intl)))
            .collect();
            refusals.extend(probes.iter().map(|probe| package_check_failed(intl, probe)));
            refusals.extend(
                [
                    FirmwareDownloadError::IndexDownloadFailed,
                    FirmwareDownloadError::FetchUpgradeDetails,
                    FirmwareDownloadError::InvalidVersion,
                    FirmwareDownloadError::UnsupportedPlatform,
                ]
                .iter()
                .map(|download| firmware_check_failed(intl, download)),
            );
            let failed_run = super::run_state_to_progress(
                intl,
                UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed),
            )
            .expect_err("BUG: a failed run is the stream's error");
            refusals.push(status_text(&failed_run));
            refusals.push(status_text(&automatic_updates_failed(intl, &"1")));
            refusals
        });
    }

    #[test]
    fn a_failure_keeps_its_detail_out_of_the_status() {
        const DETAIL: &str = "detail-for-the-log";
        let detail = || DETAIL.to_owned();
        for error in [
            SystemUpgradeError::DownloadedImageHashMismatch {
                expected: detail(),
                actual: detail(),
            },
            SystemUpgradeError::FailedToDownload(detail()),
            SystemUpgradeError::PackageUpgradeFailed(detail()),
            SystemUpgradeError::PendingInstallWriteFailed(detail()),
            SystemUpgradeError::PackageCheckFailed(PackageProbeError::ServersConfigUnavailable(
                detail(),
            )),
            SystemUpgradeError::PackageCheckFailed(PackageProbeError::IndexFetchFailed(detail())),
            SystemUpgradeError::PackageCheckFailed(PackageProbeError::IndexUnusable(detail())),
            SystemUpgradeError::PackageCheckFailed(PackageProbeError::ManifestReadFailed(detail())),
            SystemUpgradeError::PackageCheckFailed(PackageProbeError::PlanFailed(
                PackagePlanFailure::Other(detail()),
            )),
            SystemUpgradeError::PackageCheckFailed(PackageProbeError::Unrealizable(detail())),
        ] {
            let text = status_text(&status(&error));
            assert!(!text.contains(DETAIL), "{error:?} leaks into {text}");
        }
    }

    #[test]
    fn installable_supported_sizes_are_calculated_from_constraints() {
        let capabilities =
            bmc_platform::HardwareProfile::for_product(bmc_platform::Product::Bmc100)
                .capabilities();
        let platform = super::super::scene_management::PlatformDescriptor::from(&capabilities);
        let widget = crate::installable_widgets::InstallableWidget {
            package_name: "widget-fullscreen".to_owned(),
            uid: "uid-fullscreen".to_owned(),
            version: "1.0.0".to_owned(),
            display_name: "Fullscreen".to_owned(),
            subname: None,
            category: crate::installable_widgets::InstallableCategory::Unknown,
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
            vec![EnumValue::Known(bmc_grpc::web::WidgetSize::Full)]
        );
    }

    #[test]
    fn run_state_maps_to_wire_events() {
        let phase = run_state_to_progress(UpgradeRunState::Phase(UpgradePhase::PackageRealizing))
            .expect("BUG: phase maps");
        assert!(matches!(
            phase.event,
            Some(upgrade_progress::Event::PackagePhase(p))
                if p == PackageUpgradePhase::Realizing
        ));
        let finished =
            run_state_to_progress(UpgradeRunState::Finished).expect("BUG: finished maps");
        assert!(matches!(
            finished.event,
            Some(upgrade_progress::Event::Finished(_))
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
        assert!(response.firmware.is_unset());
        assert!(response.packages.is_unset());
        assert_eq!(response.disruption, UpgradeDisruption::Unspecified);
    }

    #[test]
    fn package_plan_failure_maps_to_failed_precondition_with_message() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::PlanFailed(PackagePlanFailure::MissingSystemPackages {
                names: vec!["nix".to_owned()],
            }),
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
        assert_eq!(
            error.message.as_deref(),
            Some(
                "Cannot check for upgrade: package source is incomplete; \
                 required system package \"nix\" is missing."
            )
        );
    }

    #[test]
    fn no_enabled_servers_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::NoEnabledServers,
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
        assert_eq!(
            error.message.as_deref(),
            Some("Cannot check for upgrade: no enabled package servers are configured.")
        );
    }

    #[test]
    fn transient_index_fetch_maps_to_internal() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::IndexFetchFailed("boom".to_owned()),
        ));
        assert_eq!(error.code, ErrorCode::Internal);
    }

    #[test]
    fn manifest_read_failure_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::ManifestReadFailed("io".to_owned()),
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
    }

    #[test]
    fn unusable_index_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::IndexUnusable("bad shape".to_owned()),
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
    }

    #[test]
    fn install_target_unavailable_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::InstallTargetUnavailable {
                package: "widget-nope".to_owned(),
            },
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
        assert_eq!(
            error.message.as_deref(),
            Some("Cannot check for upgrade: no package server offers \"widget-nope\".")
        );
    }

    #[test]
    fn an_ambiguous_install_target_points_at_the_servers() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::InstallTargetAmbiguous {
                package: "widget-clock".to_owned(),
            },
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
        assert_eq!(
            error.message.as_deref(),
            Some(
                "Cannot check for upgrade: package servers with the same priority offer \
                 different builds of \"widget-clock\". Check the package server configuration."
            )
        );
    }

    #[test]
    fn a_missing_feed_entry_names_the_firmware() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::NoFeedEntry {
                firmware: "26.10".to_owned(),
            },
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
        assert_eq!(
            error.message.as_deref(),
            Some("Cannot check for upgrade: no packages are published for firmware 26.10 yet.")
        );
    }

    #[test]
    fn servers_config_unavailable_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::ServersConfigUnavailable("io".to_owned()),
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
    }

    #[test]
    fn unrealizable_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::PackageCheckFailed(
            PackageProbeError::Unrealizable("/nix/store/x".to_owned()),
        ));
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
    }

    #[test]
    fn upgrade_in_progress_maps_to_unavailable() {
        let error = status(&SystemUpgradeError::UpgradeInProgress);
        assert_eq!(error.code, ErrorCode::Unavailable);
    }

    #[test]
    fn not_enough_space_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::NotEnoughSpace);
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
    }

    #[test]
    fn invalid_image_maps_to_failed_precondition() {
        let error = status(&SystemUpgradeError::InvalidImage);
        assert_eq!(error.code, ErrorCode::FailedPrecondition);
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_upgrade::firmware::FirmwareDownloadError;
use bmc_upgrade::firmware::FirmwareIndex;
use bmc_upgrade::firmware::UpgradeMetadata;
use index_bmc::{
    BmcRelease,
    query_layer::{IndexQueryLayer, UpgradeResolverError},
};
use index_common::NormalizedIndex;
use minerctl_defs::bos::version::BosVersion;
use reqwest::Client;
use std::time::Duration;

const INDEX_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct BmcIndex;

#[async_trait::async_trait]
impl FirmwareIndex for BmcIndex {
    async fn get_available_releases(
        &self,
        client: &Client,
        platform: bmc_platform::BmcPlatform,
        version: String,
    ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
        let platform = platform.into();

        let version = version
            .parse::<BosVersion>()
            .map_err(|_| FirmwareDownloadError::InvalidVersion)?;
        let downloaded = index_bmc::download(client, None, INDEX_DOWNLOAD_TIMEOUT)
            .await
            .map_err(|_| FirmwareDownloadError::IndexDownloadFailed)?;

        let index = NormalizedIndex::<BmcRelease>::normalize(downloaded);

        let upgrade_sequence =
            match IndexQueryLayer::new(&index).upgrade_sequence(platform, version) {
                Ok(upgrade_sequence) => Ok(Some(upgrade_sequence)),
                Err(err) => match err {
                    UpgradeResolverError::NoReleases => Ok(None),
                    UpgradeResolverError::AvailableVersionsDuplicates(_)
                    | UpgradeResolverError::DowngradeCrossesMajor => {
                        Err(FirmwareDownloadError::FetchUpgradeDetails)
                    }
                },
            }?;

        let Some(latest_upgrade) = upgrade_sequence.and_then(|upgrade| upgrade.first().cloned())
        else {
            return Ok(None);
        };

        let mut in_range = false;

        let available_releases = IndexQueryLayer::new(&index)
            .available_releases(platform)
            .filter_map(|(release, asset)| {
                // filter out releases before the current version
                if release.bmc_version == version {
                    in_range = true;
                    return None;
                }

                // map releases between the current version and latest upgrade
                if in_range {
                    if release.bmc_version == latest_upgrade.bmc_version {
                        in_range = false;
                    }

                    return Some(UpgradeMetadata::new(
                        asset.checksum().unwrap_or_default().to_string(),
                        release.bmc_version.to_string(),
                        release.release_date,
                        release.description.clone(),
                        asset.url().to_string(),
                        asset.size().unwrap_or_default(),
                    ));
                }
                None
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        Ok(Some(available_releases))
    }
}

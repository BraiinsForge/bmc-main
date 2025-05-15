// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod query_layer;

use chrono::NaiveDate;
use idxgen_data::IndexVariant;
pub use idxgen_data::asset_state::AssetState;
use idxgen_data::bma::v1::FileAsset;
use idxgen_data::bmc::BmcReleaseMetadata;
use idxgen_data::bmc::latest::VERSION_NAME;
use idxgen_data::bmc::v1::Assets;
use index_common::{IndexError, IndexResult, NormalizedRelease};
use minerctl_defs::bos::version::BosVersion;
use reqwest::Client;
use std::sync::LazyLock;
use std::time::Duration;
use strum::{Display, EnumString};
use url::Url;

#[derive(EnumString, Display, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum BmcPlatform {
    #[strum(serialize = "stm32mp157c-ii3-bmc1")]
    Stm32mp157cIi3Bmc1,
}

pub static DEFAULT_URL: LazyLock<Url> = LazyLock::new(|| {
    Url::parse("https://downloads.braiins.com/braiins-mining-clock")
        .expect("hardcoded url is invalid")
});

pub async fn download(
    client: &Client,
    override_url: Option<&Url>,
    timeout: Duration,
) -> IndexResult<IndexVariant<BmcReleaseMetadata>> {
    let url = override_url.unwrap_or(&DEFAULT_URL);

    let index = index_common::download(url, VERSION_NAME, client, timeout).await?;

    let idxgen_data::Index::Bmc(index_variant) = index else {
        return Err(IndexError::InvalidType);
    };

    Ok(index_variant)
}

#[derive(Debug, Clone)]
pub struct BmcRelease {
    pub bmc_version: BosVersion,
    pub is_major: bool,
    pub release_date: NaiveDate,
    pub description: String,
    pub assets: Option<Assets>,
}

impl NormalizedRelease for BmcRelease {
    type Denormalized = BmcReleaseMetadata;

    fn normalize(release: Self::Denormalized) -> Option<Self> {
        match release {
            BmcReleaseMetadata::V1(release) => Some(Self {
                bmc_version: release.bmc_version,
                is_major: release.is_major,
                release_date: release.release_date,
                description: release.description,
                assets: Some(release.assets),
            }),
        }
    }
}

impl BmcRelease {
    #[must_use]
    pub fn asset_for_platform(&self, platform: BmcPlatform) -> Option<&FileAsset> {
        let assets = self.assets.as_ref()?;

        match platform {
            BmcPlatform::Stm32mp157cIi3Bmc1 => assets.sysupgrade_emmc_stm32mp157c_ii3_bmc1.as_ref(),
        }
    }
}

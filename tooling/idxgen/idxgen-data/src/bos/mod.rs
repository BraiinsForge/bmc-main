// Copyright (C) 2023  Braiins Systems s.r.o.

pub mod v1;
pub mod v2;

use crate::asset_state::AssetState;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, MetadataVersion, ReleaseMetadata, ReleaseMetadataVersion};
use crate::signed_file_asset::SignedFileAsset;
use chrono::Local;
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use url::Url;

use crate::RELEASE_DATE_FORMAT;

/// *****************************************************************************
/// This is where we choose which version is the "latest" (and used in minerctl).
/// *****************************************************************************
pub use v2 as latest;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "metadata_version", content = "metadata")]
#[serde(rename_all = "lowercase")]
pub enum BosReleaseMetadata {
    V1(v1::Metadata),
    V2(v2::Metadata),
}

impl Default for BosReleaseMetadata {
    fn default() -> Self {
        let dummy_url = Url::parse("todo://").expect("hardcoded dummy url is invalid");
        let dummy_asset = AssetState::Available(SignedFileAsset::WithoutIntegrity {
            url: dummy_url,
            signature_url: None,
            metadata: latest::BosAssetMetadata {
                supported_models: None,
            },
        });

        Self::V2(latest::Metadata {
            bos_version: "1970-01-01-0-ffffffff-00.00-plus-x"
                .parse()
                .expect("hardcoded bos version is invalid"),
            is_major: false,
            is_silent: false,
            release_date: Local::now().date_naive(),
            supported_models: Some(BTreeSet::new()),
            description: String::new(),
            assets: latest::Assets {
                transitional_am1_s9: AssetState::None,
                transitional_am2_s17: dummy_asset.clone(),
                transitional_am3_aml: dummy_asset.clone(),
                transitional_am3_bbb: dummy_asset.clone(),
                transitional_cvitek_bm1_am2: dummy_asset.clone(),
                transitional_zynq_bm3_am2: dummy_asset.clone(),
                sysupgrade_nand_am1_s9: AssetState::None,
                sysupgrade_nand_am2_s17: dummy_asset.clone(),
                sysupgrade_nand_am3_aml: dummy_asset.clone(),
                sysupgrade_nand_am3_bbb: dummy_asset.clone(),
                sysupgrade_nand_zynq_bm3_am2: dummy_asset.clone(),
                sysupgrade_sd_am1_s9: AssetState::None,
                sysupgrade_sd_am2_s17: dummy_asset.clone(),
                sysupgrade_sd_am3_bbb: dummy_asset.clone(),
                sysupgrade_sd_stm32mp157c_ii2_bmm1: dummy_asset.clone(),
                sysupgrade_sd_stm32mp157c_ii1_am2: dummy_asset.clone(),
                sysupgrade_emmc_cvitek_bm1_am2: dummy_asset.clone(),
                sysupgrade_emmc_stm32mp15_ii1_am2: dummy_asset.clone(),
                sysupgrade_emmc_stm32mp157c_ii2_bmm1: dummy_asset.clone(),
            },
        })
    }
}

impl ReleaseMetadata for BosReleaseMetadata {
    fn metadata_version(&self) -> MetadataVersion {
        match self {
            BosReleaseMetadata::V1(_) => MetadataVersion::V1,
            BosReleaseMetadata::V2(_) => MetadataVersion::V2,
        }
    }

    fn compile(&mut self) {
        match self {
            BosReleaseMetadata::V1(m) => m.compile(),
            BosReleaseMetadata::V2(m) => m.compile(),
        }
    }

    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        match self {
            BosReleaseMetadata::V1(m) => m.collect_inputs(output),
            BosReleaseMetadata::V2(m) => m.collect_inputs(output),
        }
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        match self {
            BosReleaseMetadata::V1(m) => m.assign_integrity(lockfile),
            BosReleaseMetadata::V2(m) => m.assign_integrity(lockfile),
        }
    }

    fn downgrade(self) -> DowngradeOutput<Self> {
        match self {
            BosReleaseMetadata::V1(_) => DowngradeOutput::Terminal,
            BosReleaseMetadata::V2(metadata) => metadata.downgrade().map(BosReleaseMetadata::V1),
        }
    }

    fn release_title(&self) -> String {
        let version = match self {
            BosReleaseMetadata::V1(metadata) => &metadata.bos_version.version,
            BosReleaseMetadata::V2(metadata) => &metadata.bos_version.version,
        };

        format!("Braiins OS {version}")
    }

    fn release_date(&self) -> String {
        let date = match self {
            BosReleaseMetadata::V1(metadata) => &metadata.release_date,
            BosReleaseMetadata::V2(metadata) => &metadata.release_date,
        };
        date.format(RELEASE_DATE_FORMAT).to_string()
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        match self {
            BosReleaseMetadata::V1(m) => m.render_html(),
            BosReleaseMetadata::V2(m) => m.render_html(),
        }
    }
}

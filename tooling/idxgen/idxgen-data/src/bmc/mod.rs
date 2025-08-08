// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod v1;

use crate::file_asset::FileAsset;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, MetadataVersion, ReleaseMetadata, ReleaseMetadataVersion};
use chrono::Local;
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::RELEASE_DATE_FORMAT;

/// *****************************************************************************
/// This is where we choose which version is the "latest" (and used in minerctl).
/// *****************************************************************************
pub use v1 as latest;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "metadata_version", content = "metadata")]
#[serde(rename_all = "lowercase")]
pub enum BmcReleaseMetadata {
    V1(v1::Metadata),
}

impl Default for BmcReleaseMetadata {
    fn default() -> Self {
        let dummy_url = Url::parse("todo://").expect("hardcoded dummy url is invalid");

        Self::V1(v1::Metadata {
            bmc_version: "1970-01-01-0-ffffffff-00.00-plus-x"
                .parse()
                .expect("hardcoded bmc version is invalid"),
            is_major: false,
            release_date: Local::now().date_naive(),
            description: String::new(),
            assets: v1::Assets {
                sysupgrade_emmc_stm32mp157c_ii3_bmc1: Some(FileAsset::WithoutIntegrity(dummy_url)),
            },
        })
    }
}

impl ReleaseMetadata for BmcReleaseMetadata {
    fn metadata_version(&self) -> MetadataVersion {
        match self {
            BmcReleaseMetadata::V1(_) => MetadataVersion::V1,
        }
    }

    fn compile(&mut self) {}

    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        match self {
            BmcReleaseMetadata::V1(m) => m.collect_inputs(output),
        }
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        match self {
            BmcReleaseMetadata::V1(m) => m.assign_integrity(lockfile),
        }
    }

    fn downgrade(self) -> DowngradeOutput<Self> {
        match self {
            BmcReleaseMetadata::V1(_) => DowngradeOutput::Terminal,
        }
    }

    fn release_title(&self) -> String {
        let version = match self {
            BmcReleaseMetadata::V1(metadata) => &metadata.bmc_version.version,
        };

        format!("Braiins Deck FW {version}")
    }

    fn release_date(&self) -> String {
        let date = match self {
            BmcReleaseMetadata::V1(metadata) => &metadata.release_date,
        };
        date.format(RELEASE_DATE_FORMAT).to_string()
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        match self {
            BmcReleaseMetadata::V1(m) => m.render_html(),
        }
    }
}

// Copyright (C) 2024  Braiins Systems s.r.o.

pub mod v1;

use crate::file_asset::FileAsset;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, MetadataVersion, ReleaseMetadata, ReleaseMetadataVersion};
use chrono::Local;
use indexmap::{IndexMap, IndexSet};
use minerctl_defs::commit::CommitHashShort;
use semver::Version;
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
pub enum BmaReleaseMetadata {
    V1(Box<v1::Metadata>),
}

impl Default for BmaReleaseMetadata {
    fn default() -> Self {
        let dummy_url = Url::parse("todo://").expect("dummy url is invalid");
        Self::V1(Box::new(latest::Metadata {
            bma_version: Version::new(0, 0, 0),
            commit: CommitHashShort::from(0),
            release_date: Local::now().date_naive(),
            description: String::new(),
            assets: latest::Assets {
                linux_x86_64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                linux_aarch64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                linux_armv7: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                windows_x86_64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
            },
        }))
    }
}

impl ReleaseMetadata for BmaReleaseMetadata {
    fn metadata_version(&self) -> MetadataVersion {
        match self {
            BmaReleaseMetadata::V1(_) => MetadataVersion::V1,
        }
    }

    fn compile(&mut self) {}

    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        match self {
            BmaReleaseMetadata::V1(m) => m.collect_inputs(output),
        }
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        match self {
            BmaReleaseMetadata::V1(m) => m.assign_integrity(lockfile),
        }
    }

    fn downgrade(self) -> DowngradeOutput<Self> {
        match self {
            BmaReleaseMetadata::V1(_) => DowngradeOutput::Terminal,
        }
    }

    fn release_title(&self) -> String {
        let version = match self {
            BmaReleaseMetadata::V1(metadata) => &metadata.bma_version,
        };

        format!("Braiins Manager Agent {version}")
    }

    fn release_date(&self) -> String {
        let date = match self {
            BmaReleaseMetadata::V1(metadata) => &metadata.release_date,
        };
        date.format(RELEASE_DATE_FORMAT).to_string()
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        match self {
            BmaReleaseMetadata::V1(metadata) => metadata.render_html(),
        }
    }
}

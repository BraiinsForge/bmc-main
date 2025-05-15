// Copyright (C) 2023  Braiins Systems s.r.o.

pub mod v1;
pub mod v2;

use crate::file_asset::FileAsset;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, MetadataVersion, ReleaseMetadata, ReleaseMetadataVersion};
use chrono::Local;
use indexmap::{IndexMap, IndexSet};
use minerctl_defs::commit::CommitHashShort;
use serde::{Deserialize, Serialize};
use tooling_std::version::{AppVersion, StableVersionName};
use url::Url;

use crate::RELEASE_DATE_FORMAT;
/// *****************************************************************************
/// This is where we choose which version is the "latest" (and used in minerctl).
/// *****************************************************************************
pub use v2 as latest;

// TODO: impl From<v2::Metadata> for ToolboxReleaseMetadata
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "metadata_version", content = "metadata")]
#[serde(rename_all = "lowercase")]
pub enum ToolboxReleaseMetadata {
    V1(Box<v1::Metadata>),
    V2(Box<v2::Metadata>),
}

impl Default for ToolboxReleaseMetadata {
    fn default() -> Self {
        let dummy_url = Url::parse("todo://").expect("dummy url is invalid");
        Self::V2(Box::new(latest::Metadata {
            toolbox_version: AppVersion::Stable(StableVersionName {
                year: 0,
                month: 0,
                patch: None,
            }),
            commit: CommitHashShort::from(0),
            release_date: Local::now().date_naive(),
            description: String::new(),
            assets: latest::Assets {
                linux_x86_64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                linux_aarch64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                linux_armv7: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                macos_x86_64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
                windows_x86_64: Some(FileAsset::WithoutIntegrity(dummy_url.clone())),
            },
        }))
    }
}

impl ReleaseMetadata for ToolboxReleaseMetadata {
    fn metadata_version(&self) -> MetadataVersion {
        match self {
            ToolboxReleaseMetadata::V1(_) => MetadataVersion::V1,
            ToolboxReleaseMetadata::V2(_) => MetadataVersion::V2,
        }
    }

    fn compile(&mut self) {}

    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        match self {
            ToolboxReleaseMetadata::V1(m) => m.collect_inputs(output),
            ToolboxReleaseMetadata::V2(m) => m.collect_inputs(output),
        }
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        match self {
            ToolboxReleaseMetadata::V1(m) => m.assign_integrity(lockfile),
            ToolboxReleaseMetadata::V2(m) => m.assign_integrity(lockfile),
        }
    }

    fn downgrade(self) -> DowngradeOutput<Self> {
        match self {
            ToolboxReleaseMetadata::V1(_) => DowngradeOutput::Terminal,
            ToolboxReleaseMetadata::V2(metadata) => metadata
                .downgrade()
                .map(Box::new)
                .map(ToolboxReleaseMetadata::V1),
        }
    }

    fn release_title(&self) -> String {
        let version = match self {
            ToolboxReleaseMetadata::V1(metadata) => &metadata.toolbox_version,
            ToolboxReleaseMetadata::V2(metadata) => &metadata.toolbox_version,
        };

        format!("Toolbox {version}")
    }

    fn release_date(&self) -> String {
        let date = match self {
            ToolboxReleaseMetadata::V1(metadata) => &metadata.release_date,
            ToolboxReleaseMetadata::V2(metadata) => &metadata.release_date,
        };
        date.format(RELEASE_DATE_FORMAT).to_string()
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        match self {
            ToolboxReleaseMetadata::V1(metadata) => metadata.render_html(),
            ToolboxReleaseMetadata::V2(metadata) => metadata.render_html(),
        }
    }
}

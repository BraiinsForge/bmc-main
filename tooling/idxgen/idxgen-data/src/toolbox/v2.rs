// Copyright (C) 2023  Braiins Systems s.r.o.

pub use crate::file_asset::FileAsset;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, Input, ReleaseMetadataVersion};
use crate::toolbox::v1;
use chrono::NaiveDate;
use indexmap::{IndexMap, IndexSet};
use minerctl_defs::commit::CommitHashShort;
use serde::{Deserialize, Serialize};
use tooling_std::macros::TitledAssets;
use tooling_std::version::AppVersion;
use url::Url;

pub const VERSION_NAME: &str = "v2";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub toolbox_version: AppVersion,
    pub commit: CommitHashShort,
    pub release_date: NaiveDate,
    pub description: String,
    pub assets: Assets,
}

#[derive(Serialize, Deserialize, TitledAssets, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    #[asset(title = "Toolbox Linux x86_64")]
    pub linux_x86_64: Option<FileAsset>,
    #[asset(title = "Toolbox Linux aarch64")]
    pub linux_aarch64: Option<FileAsset>,
    #[asset(title = "Toolbox Linux armv7")]
    pub linux_armv7: Option<FileAsset>,
    #[asset(title = "Toolbox MacOS x86_64")]
    pub macos_x86_64: Option<FileAsset>,
    #[asset(title = "Toolbox Windows x86_64")]
    pub windows_x86_64: Option<FileAsset>,
}

impl ReleaseMetadataVersion for Metadata {
    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        self.assets.linux_x86_64.collect_inputs(output);
        self.assets.linux_aarch64.collect_inputs(output);
        self.assets.linux_armv7.collect_inputs(output);
        self.assets.macos_x86_64.collect_inputs(output);
        self.assets.windows_x86_64.collect_inputs(output);
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        self.assets.linux_x86_64.assign_integrity(lockfile);
        self.assets.linux_aarch64.assign_integrity(lockfile);
        self.assets.linux_armv7.assign_integrity(lockfile);
        self.assets.macos_x86_64.assign_integrity(lockfile);
        self.assets.windows_x86_64.assign_integrity(lockfile);
    }

    type Older = v1::Metadata;

    fn downgrade(self) -> DowngradeOutput<Self::Older> {
        DowngradeOutput::NonBreaking(v1::Metadata {
            toolbox_version: self.toolbox_version,
            release_date: self.release_date,
            description: self.description,
        })
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        use crate::html_render;
        use maud::{PreEscaped, html};

        html! {
            div.markdown {
                (PreEscaped(markdown::to_html(&self.description)))
            }

            (html_render::assets_table(&self.assets.titled_assets()))
        }
    }
}

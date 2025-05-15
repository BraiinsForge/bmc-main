// Copyright (C) 2025  Braiins Systems s.r.o.

pub use crate::file_asset::FileAsset;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, Input, ReleaseMetadataVersion};
use chrono::NaiveDate;
use indexmap::{IndexMap, IndexSet};
use minerctl_defs::bos::version::BosVersion;
use serde::{Deserialize, Serialize};
use tooling_std::macros::{StructIter, TitledAssets};
use url::Url;

pub const VERSION_NAME: &str = "v1";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    // NOTE: BMC version has same format as BOS version
    pub bmc_version: BosVersion,
    pub is_major: bool,
    pub release_date: NaiveDate,
    pub description: String,
    pub assets: Assets,
}

#[derive(Serialize, Deserialize, StructIter, TitledAssets, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    #[asset(title = "sysupgrade stm32mp157c-ii3-bmc1 emmc")]
    pub sysupgrade_emmc_stm32mp157c_ii3_bmc1: Option<FileAsset>,
}

impl ReleaseMetadataVersion for Metadata {
    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        self.assets.iter().for_each(|asset| {
            asset.collect_inputs(output);
        });
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        self.assets.iter_mut().for_each(|asset| {
            asset.assign_integrity(lockfile);
        });
    }

    type Older = ();

    fn downgrade(self) -> DowngradeOutput<Self::Older> {
        DowngradeOutput::Terminal
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        use crate::html_render;
        use maud::{PreEscaped, html};

        html! {
            div.meta {
                span.label { "Version" }
                span.value {
                    span.select { (self.bmc_version) }
                    @if self.is_major { span.tag { "Major" } }
                }
            }

            div.markdown {
                (PreEscaped(markdown::to_html(&self.description)))
            }

            (html_render::assets_table(&self.assets.titled_assets()))
        }
    }
}

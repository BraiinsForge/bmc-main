// Copyright (C) 2024  Braiins Systems s.r.o.

#![allow(clippy::iter_without_into_iter)]

use crate::asset_state::AssetState;
use crate::bos::{SignedFileAsset, v1};
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, Input, ReleaseMetadataVersion};
use chrono::NaiveDate;
use indexmap::{IndexMap, IndexSet};
use minerctl_defs::bos::version::BosVersion;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tooling_std::macros::{StructIter, TitledAssets};
use url::Url;

pub use v1::BosAssetMetadata;

pub const VERSION_NAME: &str = "v2";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub bos_version: BosVersion,
    pub is_major: bool,
    pub is_silent: bool,
    pub release_date: NaiveDate,
    /// Optional whitelist of model names. `None` means that all model names allowed.
    ///
    /// This is a shortcut for setting supported models for all assets
    /// If some of the assets contain their own supported models, they will be added to this list
    /// e.g.: `metadata.supported_stock_models = ["Antminer T19"];`
    ///       `assets.transitional_am1_s9.supported_stock_models = ["Antminer S19 Pro"];`
    ///        => `metadata.supported_stock_models = ["Antminer T19", "Antminer S19 Pro"];`
    /// But if the asset contains its own supported models, it will keep that and ignore the default list in metadata
    ///
    /// IMPORTANT: Supported models must be compared only with specific asset.
    ///            This is used for informational purposes only.
    pub supported_models: Option<BTreeSet<String>>,
    pub description: String,
    pub assets: Assets,
}

#[derive(Serialize, Deserialize, StructIter, TitledAssets, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    #[asset(title = "transitional am1-s9")]
    pub transitional_am1_s9: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "transitional am1-s17")]
    pub transitional_am2_s17: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "transitional am3-aml")]
    pub transitional_am3_aml: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "transitional am3-bbb")]
    pub transitional_am3_bbb: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "transitional cvitek-bm1-am2")]
    pub transitional_cvitek_bm1_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "transitional zynq-bm3-am2")]
    pub transitional_zynq_bm3_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,

    #[asset(title = "sysupgrade am1-s9 nand")]
    pub sysupgrade_nand_am1_s9: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade am2-s17 nand")]
    pub sysupgrade_nand_am2_s17: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade am3-aml nand")]
    pub sysupgrade_nand_am3_aml: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade am3-bbb nand")]
    pub sysupgrade_nand_am3_bbb: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade zynq-bm3-am2 nand")]
    pub sysupgrade_nand_zynq_bm3_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,

    #[asset(title = "sysupgrade am1-s9 sd")]
    pub sysupgrade_sd_am1_s9: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade am2-s17 sd")]
    pub sysupgrade_sd_am2_s17: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade am3-bbb sd")]
    pub sysupgrade_sd_am3_bbb: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade stm32mp157c-ii1-am2 sd")]
    pub sysupgrade_sd_stm32mp157c_ii1_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade stm32mp157c-ii2-bmm1 sd")]
    pub sysupgrade_sd_stm32mp157c_ii2_bmm1: AssetState<SignedFileAsset<BosAssetMetadata>>,

    #[asset(title = "sysupgrade cvitek-bm1-am2 emmc")]
    pub sysupgrade_emmc_cvitek_bm1_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade stm32mp15-ii1-am2 emmc")]
    pub sysupgrade_emmc_stm32mp15_ii1_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade stm32mp157c-ii2-bmm1 emmc")]
    pub sysupgrade_emmc_stm32mp157c_ii2_bmm1: AssetState<SignedFileAsset<BosAssetMetadata>>,
}

impl Metadata {
    pub fn compile(&mut self) {
        let all_supported_models = self
            .assets
            .iter_mut()
            .map(|asset| {
                if let AssetState::Available(asset) = asset {
                    let asset_metadata = asset.metadata_mut();
                    let Some(asm) = &mut asset_metadata.supported_models else {
                        // set SSM to those from release metadata if the asset doesn't have any
                        asset_metadata
                            .supported_models
                            .clone_from(&self.supported_models);
                        // no extending needed, so we just return an empty set
                        return BTreeSet::new();
                    };

                    // extend asset's SSM by the defaults from release metadata
                    if let Some(supported_models) = &self.supported_models {
                        asm.extend(supported_models.clone());
                    }

                    return asm.clone();
                }

                BTreeSet::new()
            })
            .fold(BTreeSet::new(), |mut acc, x| {
                acc.extend(x);
                acc
            });

        if !all_supported_models.is_empty() {
            self.supported_models = Some(all_supported_models);
        }
    }
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

    type Older = v1::Metadata;

    fn downgrade(self) -> DowngradeOutput<Self::Older> {
        DowngradeOutput::NonBreaking(v1::Metadata {
            bos_version: self.bos_version,
            is_major: self.is_major,
            release_date: self.release_date,
            supported_models: self.supported_models,
            description: self.description,
            assets: v1::Assets {
                transitional_am1_s9: self.assets.transitional_am1_s9,
                transitional_am2_s17: self.assets.transitional_am2_s17,
                transitional_am3_aml: self.assets.transitional_am3_aml,
                transitional_am3_bbb: self.assets.transitional_am3_bbb,
                transitional_cvitek_bm1_am2: self.assets.transitional_cvitek_bm1_am2,
                transitional_zynq_bm3_am2: self.assets.transitional_zynq_bm3_am2,
                sysupgrade_nand_am1_s9: self.assets.sysupgrade_nand_am1_s9,
                sysupgrade_nand_am2_s17: self.assets.sysupgrade_nand_am2_s17,
                sysupgrade_nand_am3_aml: self.assets.sysupgrade_nand_am3_aml,
                sysupgrade_nand_am3_bbb: self.assets.sysupgrade_nand_am3_bbb,
                sysupgrade_nand_zynq_bm3_am2: self.assets.sysupgrade_nand_zynq_bm3_am2,
                sysupgrade_sd_am1_s9: self.assets.sysupgrade_sd_am1_s9,
                sysupgrade_sd_am2_s17: self.assets.sysupgrade_sd_am2_s17,
                sysupgrade_sd_am3_bbb: self.assets.sysupgrade_sd_am3_bbb,
                sysupgrade_emmc_cvitek_bm1_am2: self.assets.sysupgrade_emmc_cvitek_bm1_am2,
                sysupgrade_emmc_stm32mp15_ii1_am2: self.assets.sysupgrade_emmc_stm32mp15_ii1_am2,
            },
        })
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        use crate::html_render;
        use maud::{PreEscaped, html};

        html! {
            div.meta {
                span.label { "Version" }
                span.value {
                    span.select { (self.bos_version) }
                    @if self.is_major { span.tag { "Major" } }
                }
            }
            @if let Some(supported_models) = &self.supported_models {
                div.meta {
                    span.label { "Supported Models" }
                    ul.value {
                        @for model in supported_models { li { (model) } }
                    }
                }
            }

            div.markdown {
                (PreEscaped(markdown::to_html(&self.description)))
            }

            (html_render::signed_assets_table(&html_render::signed_assets_html_metadata(&self.assets.titled_assets())))
        }
    }
}

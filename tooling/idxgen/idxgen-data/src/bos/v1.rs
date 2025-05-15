// Copyright (C) 2024  Braiins Systems s.r.o.

// TODO: fix StructIter macro
#![allow(clippy::iter_without_into_iter)]

use crate::asset_state::AssetState;
use crate::bos::SignedFileAsset;
use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, Input, ReleaseMetadataVersion};
use chrono::NaiveDate;
use indexmap::{IndexMap, IndexSet};
use minerctl_defs::bos::version::BosVersion;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tooling_std::macros::{StructIter, TitledAssets};
use url::Url;

pub const VERSION_NAME: &str = "v1";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub bos_version: BosVersion,
    pub is_major: bool,
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

    #[asset(title = "sysupgrade cvitek-bm1-am2 emmc")]
    pub sysupgrade_emmc_cvitek_bm1_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,
    #[asset(title = "sysupgrade stm32mp15-ii1-am2 emmc")]
    pub sysupgrade_emmc_stm32mp15_ii1_am2: AssetState<SignedFileAsset<BosAssetMetadata>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BosAssetMetadata {
    /// Optional whitelist of supported model names. `None` means that all model names allowed.
    /// This will override the supported stock models set in the metadata.
    pub supported_models: Option<BTreeSet<String>>,
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

    type Older = ();

    fn downgrade(self) -> DowngradeOutput<Self::Older> {
        DowngradeOutput::Terminal
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        use crate::html_render;
        use maud::{PreEscaped, html};

        html! {
            h3 { "Metadata" }
            ul {
                li { b { "Version: " } (self.bos_version) }
                li { b { "Major: " } (self.is_major) }
                @if let Some(supported_models) = &self.supported_models {
                  li {
                      b { "Supported models: " }
                      ul {
                          @for model in supported_models {
                              li { (model) }
                          }
                      }
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

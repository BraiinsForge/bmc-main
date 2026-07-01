// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::bos::version::BosVersion;
pub use crate::file_asset::FileAsset;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    pub sysupgrade_emmc_stm32mp157c_ii3_bmc1: Option<FileAsset>,
}

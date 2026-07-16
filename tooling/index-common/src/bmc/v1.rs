// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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

// Copyright (C) 2026  Braiins Forge s.r.o.
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

//! Canonical manifest UIDs for the in-tree WASM widgets.
//!
//! Each constant is the real `uid` declared in the corresponding
//! widget's `manifest.json` under `widgets-wasm/`. They are shared by
//! the built-in default scenes ([`crate::config::defaults`]) and the
//! v0 → current migration ([`crate::config_migration`]), so both refer
//! to the same source of truth instead of re-hardcoding literals. The
//! `manifest_uids_match_the_shipped_manifests` test cross-checks every
//! constant against its shipped manifest.

use uuid::Uuid;

/// `widgets-wasm/clock`
pub(crate) const CLOCK_UID: Uuid = Uuid::from_u128(0xfbc8_67c9_b722_4bdb_8738_c15d_20fe_2b88);
/// `widgets-wasm/weather`
pub(crate) const WEATHER_UID: Uuid = Uuid::from_u128(0x2379_712a_e573_46db_8e9c_94f6_ed75_d92c);
/// `widgets-wasm/blockheight`
pub(crate) const BLOCK_HEIGHT_UID: Uuid =
    Uuid::from_u128(0x7cb5_84a8_1f26_42a0_867e_955a_add2_391c);
/// `widgets-wasm/mining-info`
pub(crate) const MINING_INFO_UID: Uuid = Uuid::from_u128(0x6d0c_6a2d_24d0_4384_8f8b_6f4a_c2c9_675a);
/// `widgets-wasm/mining-clock`
pub(crate) const MINING_CLOCK_UID: Uuid =
    Uuid::from_u128(0x0f0b_7df0_f6d5_4d21_9ddc_7755_e503_0503);
/// `widgets-wasm/image`
pub(crate) const REMOTE_IMAGE_UID: Uuid =
    Uuid::from_u128(0xf9e4_956c_719d_450c_909d_4fc9_d444_0e15);
/// `widgets-wasm/iss-position`
pub(crate) const ISS_POSITION_UID: Uuid =
    Uuid::from_u128(0x0a39_73c9_3a97_4bf2_957a_741e_5535_3a19);
/// `widgets-wasm/nameday`
pub(crate) const NAMEDAY_UID: Uuid = Uuid::from_u128(0x5062_553f_31eb_497c_b513_bb82_f41a_2809);
/// `widgets-wasm/random-facts`
pub(crate) const RANDOM_FACTS_UID: Uuid =
    Uuid::from_u128(0xaf91_8b37_9df8_4faa_93c2_1985_563e_b94b);
/// `widgets-wasm/spacex-launch`
pub(crate) const SPACEX_LAUNCH_UID: Uuid =
    Uuid::from_u128(0xe854_e395_5d90_45ca_b4c9_eb5e_e327_a457);
/// `widgets-wasm/braiins-pool`
pub(crate) const BRAIINS_POOL_UID: Uuid =
    Uuid::from_u128(0xb4e0_608d_d38c_4494_8bee_7df2_a030_c9b1);

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bmc_widget_manifest::Manifest;

    use super::*;

    /// Every UID constant must equal the `uid` of the manifest it names.
    /// This is the single guard that keeps both the default scenes and
    /// the migration table pointing at real, shipped widgets.
    #[test]
    fn manifest_uids_match_the_shipped_manifests() {
        let cases = [
            (
                "clock",
                CLOCK_UID,
                include_str!("../../../widgets-wasm/clock/manifest.json"),
            ),
            (
                "weather",
                WEATHER_UID,
                include_str!("../../../widgets-wasm/weather/manifest.json"),
            ),
            (
                "blockheight",
                BLOCK_HEIGHT_UID,
                include_str!("../../../widgets-wasm/blockheight/manifest.json"),
            ),
            (
                "mining-info",
                MINING_INFO_UID,
                include_str!("../../../widgets-wasm/mining-info/manifest.json"),
            ),
            (
                "mining-clock",
                MINING_CLOCK_UID,
                include_str!("../../../widgets-wasm/mining-clock/manifest.json"),
            ),
            (
                "image",
                REMOTE_IMAGE_UID,
                include_str!("../../../widgets-wasm/image/manifest.json"),
            ),
            (
                "iss-position",
                ISS_POSITION_UID,
                include_str!("../../../widgets-wasm/iss-position/manifest.json"),
            ),
            (
                "nameday",
                NAMEDAY_UID,
                include_str!("../../../widgets-wasm/nameday/manifest.json"),
            ),
            (
                "random-facts",
                RANDOM_FACTS_UID,
                include_str!("../../../widgets-wasm/random-facts/manifest.json"),
            ),
            (
                "spacex-launch",
                SPACEX_LAUNCH_UID,
                include_str!("../../../widgets-wasm/spacex-launch/manifest.json"),
            ),
            (
                "braiins-pool",
                BRAIINS_POOL_UID,
                include_str!("../../../widgets-wasm/braiins-pool/manifest.json"),
            ),
        ];

        for (name, uid, json) in cases {
            let manifest = Manifest::from_str(json).expect("BUG: in-tree manifest must parse");
            assert_eq!(
                manifest.uid, uid,
                "{name} UID constant does not match its shipped manifest"
            );
        }
    }
}

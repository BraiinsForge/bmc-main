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
//! to the same source of truth instead of re-hardcoding literals.

use uuid::Uuid;

/// `widgets-wasm/clock`
pub(crate) const CLOCK_UID: Uuid = Uuid::from_u128(0xfbc8_67c9_b722_4bdb_8738_c15d_20fe_2b88);
/// `widgets-wasm/weather`
pub(crate) const WEATHER_UID: Uuid = Uuid::from_u128(0x2379_712a_e573_46db_8e9c_94f6_ed75_d92c);
/// `widgets-wasm/blockheight`
pub(crate) const BLOCK_HEIGHT_UID: Uuid =
    Uuid::from_u128(0x7cb5_84a8_1f26_42a0_867e_955a_add2_391c);
/// `widgets-wasm/halving-countdown`
pub(crate) const HALVING_COUNTDOWN_UID: Uuid =
    Uuid::from_u128(0x8a87_742d_192d_4c80_bda2_d446_e9b9_aeae);
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
/// `widgets-wasm/ticker-single`
pub(crate) const TICKER_SINGLE_UID: Uuid =
    Uuid::from_u128(0x69ed_377c_701b_4cdb_b4b6_0308_cfe5_6b64);
/// `widgets-wasm/ticker-list`
pub(crate) const TICKER_LIST_UID: Uuid = Uuid::from_u128(0x51f4_8290_a8fd_466d_8693_1911_b06c_68c8);
/// `widgets-wasm/formula-1`
pub(crate) const FORMULA_1_UID: Uuid = Uuid::from_u128(0x2032_6ae9_741c_4374_b322_b91a_d377_a0a3);
/// `widgets-wasm/bitcoin-mining-data`
pub(crate) const BITCOIN_MINING_DATA_UID: Uuid =
    Uuid::from_u128(0x020e_06d1_434e_4757_b4c3_21cf_92c4_4127);

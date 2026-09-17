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

//! The readings the gallery stages and the tests assert against:
//! one miner and one market, in the states a face has to survive.

use core::time::Duration;

use bmc_wasm_sdk::types::{
    Availability, BitcoinAmount, ElectricPower, Hashrate, Hashvalue, MiningEfficiency, Ratio,
    SiPrefix, Temperature,
};

use crate::model::{
    Constraints, Currency, MinerData, Money, PublicData, TargetRange, TemperatureRange,
};

/// Tuner target the gauge sweeps anchor against.
/// A healthy miner is tuned to the default, so `hashrate` relative to it decides the gauge state.
pub const DEFAULT_TARGET_THS: f64 = 1.0;

/// How much of itself a miner reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reported {
    /// Everything the faces can draw.
    All,
    /// No board sensor and no per-board rates, which is all it takes
    /// to lose the temperature row and the mining-mode ratio:
    /// each needs a pair of readings, and half a pair reads as nothing.
    WithoutBoards,
    /// Nothing at all — the placeholder pass every face has to survive
    /// without collapsing its layout.
    Nothing,
}

/// How the 24h price moved, which colours the change and shapes the sparkline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriceMove {
    Up,
    Down,
    /// The history endpoint has not answered: no sparkline, the change still shows.
    NoHistory,
}

#[must_use]
pub fn miner(reported: Reported, hashrate_ths: Option<f64>) -> MinerData {
    if reported == Reported::Nothing {
        return MinerData::default();
    }
    let boards = reported == Reported::All;
    MinerData {
        hashrate: hashrate_ths
            .map(Hashrate::from_terahashes_per_second)
            .into(),
        temperature: boards
            .then(|| TemperatureRange {
                board: Temperature::from_celsius(61.0),
                chip: Temperature::from_celsius(74.0),
            })
            .into(),
        power: Availability::Available(ElectricPower::from_watts(41.0)),
        efficiency: Availability::Available(MiningEfficiency::from_joules_per_terahash(21.5)),
        mcr: boards.then(|| Ratio::from_percent(98.0)).into(),
        fan_speed: Availability::Available(Ratio::from_percent(72.0)),
        uptime: Availability::Available(Duration::from_hours(2 * 24 + 3) + Duration::from_mins(57)),
        ip_address: Availability::Available("192.168.23.1".to_owned()),
        chip_type: Availability::Available("BM1370".to_owned()),
        chip_count: Availability::Available(108),
        found_blocks: Availability::Available(4),
        constraints: Constraints {
            hashrate: Some(TargetRange {
                min: 0.5,
                default: DEFAULT_TARGET_THS,
                max: 1.4,
            }),
        },
    }
}

/// The market fixture, or a fully-unavailable one.
/// Drops alongside the miner half: a face reading
/// only one of the two would otherwise still look full.
#[must_use]
pub fn public(reported: Reported, price: PriceMove) -> PublicData {
    if reported == Reported::Nothing {
        return PublicData::default();
    }
    let (change, step, points) = match price {
        PriceMove::Up => (6.25, 30.0, 64),
        PriceMove::Down => (-4.8, -30.0, 64),
        PriceMove::NoHistory => (6.25, 0.0, 0),
    };
    PublicData {
        btc_price: Availability::Available(Money::new(101_754.0, Currency::Usd)),
        btc_change_24h: Availability::Available(Ratio::from_percent(change)),
        prev_diff_adjust: Availability::Available(Ratio::from_fraction(-0.021)),
        est_diff_adjust: Availability::Available(Ratio::from_fraction(-0.045)),
        epoch_progress: Availability::Available(Ratio::from_fraction(0.87)),
        epoch_remaining: Availability::Available(Duration::from_mins(262 * 10)),
        network_hashrate: Availability::Available(Hashrate::from_si(650.0, SiPrefix::Exa)),
        avg_fees_per_block: Availability::Available(BitcoinAmount::from_bitcoin(0.055)),
        avg_fee_share: Availability::Available(Ratio::from_percent(12.1)),
        block_height: Availability::Available(880_123),
        hashvalue: Availability::Available(Hashvalue::from_satoshis_per_terahash_day(70.0)),
        btc_price_history: (0..points)
            .map(|i| 100_000.0 + f64::from(i) * step)
            .collect(),
    }
}

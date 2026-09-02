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

use core::time::Duration;

use bmc_wasm_sdk::{
    BitcoinAmount, ElectricPower, Hashrate, Hashvalue, MiningEfficiency, Ratio, Temperature,
};
// Re-exported so a caller building a `Constraints` does not need a `mining`
// dependency of its own just to name the type inside it.
pub use mining::gauge::TargetRange;

pub use crate::money::{Currency, Hashprice, Money};

pub use crate::availability::Availability;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MinerData {
    pub hashrate: Availability<Hashrate>,
    pub temperature: Availability<TemperatureRange>,
    pub power: Availability<ElectricPower>,
    pub efficiency: Availability<MiningEfficiency>,
    pub mcr: Availability<Ratio>,
    pub fan_speed: Availability<Ratio>,
    pub uptime: Availability<Duration>,
    pub ip_address: Availability<String>,
    pub chip_type: Availability<String>,
    pub chip_count: Availability<usize>,
    pub constraints: Constraints,
}

// Tuner min/default/max targets that anchor the gauge sweep.
// Each is `Some` only when the endpoint reports all three of its leaves.
// The faces render a single hashrate ring, so `power` is parsed
// for parser parity but unused here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Constraints {
    pub hashrate: Option<TargetRange>,
    pub power: Option<TargetRange>,
}

/// Board and chip temperature, the pair the miner reports
/// and the faces render as one `61-74` reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemperatureRange {
    pub board: Temperature,
    pub chip: Temperature,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PublicData {
    pub btc_price: Availability<Money>,
    pub btc_change_24h: Availability<Ratio>,
    pub prev_diff_adjust: Availability<Ratio>,
    pub est_diff_adjust: Availability<Ratio>,
    pub epoch_progress: Availability<Ratio>,
    pub avg_fee: Availability<BitcoinAmount>,
    pub avg_fee_share: Availability<Ratio>,
    pub block_height: Availability<u64>,
    pub hashvalue: Availability<Hashvalue>,
    // Chronological 1d price samples for the header sparkline.
    // Empty until the price-history endpoint replies;
    // the chart is omitted while empty.
    pub btc_price_history: Vec<f64>,
}

/// The currency the widgets quote. The API is asked for
/// [`Currency::code`] and the replies are filed under this same value, so the
/// query and the rendered symbol are one decision rather than two.
pub const CURRENCY: Currency = Currency::Usd;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_the_default_state() {
        let data = MinerData::default();
        assert_eq!(data.hashrate, Availability::Unavailable);
        assert_eq!(data.temperature, Availability::Unavailable);
    }
}

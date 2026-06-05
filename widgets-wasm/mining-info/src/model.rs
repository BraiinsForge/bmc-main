// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::units::{
    Btc, DegreeCelsius, ExaHashPerSecond, JoulePerTeraHash, Percent, SatPerTeraHashDay, Seconds,
    TeraHashPerSecond, Watt,
};
use mining::gauge::TargetRange;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Availability<T> {
    Available(T),
    Unavailable,
}

#[expect(
    clippy::derivable_impls,
    reason = "deriving Default would add a T: Default bound; this impl stays unbounded so containers holding non-Default payloads can still derive Default"
)]
impl<T> Default for Availability<T> {
    fn default() -> Self {
        Self::Unavailable
    }
}

impl<T> Availability<T> {
    pub(crate) fn as_option(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MinerData {
    pub(crate) hashrate_ths: Availability<TeraHashPerSecond>,
    pub(crate) temperature: Availability<TemperatureRange>,
    pub(crate) power_w: Availability<Watt>,
    pub(crate) efficiency_j_th: Availability<JoulePerTeraHash>,
    pub(crate) mcr_percent: Availability<Percent>,
    pub(crate) fan_percent: Availability<Percent>,
    pub(crate) uptime_s: Availability<Seconds>,
    pub(crate) ip_address: Availability<String>,
    pub(crate) constraints: Constraints,
}

// Tuner min/default/max targets that anchor the gauge sweep. Each is `Some` only
// when the endpoint reports all three of its leaves. mining-info renders a single
// hashrate ring, so `power` is parsed for parser parity but unused here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Constraints {
    pub(crate) hashrate: Option<TargetRange>,
    pub(crate) power: Option<TargetRange>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TemperatureRange {
    pub(crate) board: DegreeCelsius,
    pub(crate) chip: DegreeCelsius,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PublicData {
    pub(crate) btc_price: Availability<Money>,
    pub(crate) btc_change_24h_percent: Availability<Percent>,
    pub(crate) network_hashrate_ehs: Availability<ExaHashPerSecond>,
    pub(crate) prev_diff_adjust_percent: Availability<Percent>,
    pub(crate) est_diff_adjust_percent: Availability<Percent>,
    pub(crate) epoch_progress_percent: Availability<Percent>,
    pub(crate) avg_fee_btc: Availability<Btc>,
    pub(crate) avg_fee_percent: Availability<Percent>,
    pub(crate) block_height: Availability<u64>,
    pub(crate) hashprice: Availability<Money>,
    pub(crate) hashvalue_sat_th_day: Availability<SatPerTeraHashDay>,
    // Chronological 1d price samples for the header sparkline. Empty until the
    // price-history endpoint replies; the chart is omitted while empty.
    pub(crate) btc_price_history: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Money {
    pub(crate) currency: Currency,
    pub(crate) value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Currency {
    Usd,
    Eur,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_the_default_state() {
        let data = MinerData::default();
        assert_eq!(data.hashrate_ths, Availability::Unavailable);
        assert_eq!(data.temperature, Availability::Unavailable);
    }

    #[test]
    fn available_values_can_be_borrowed_as_options() {
        let value = Availability::Available(42_u32);
        assert_eq!(value.as_option(), Some(&42));
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

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
    pub(crate) hashrate_ths: Availability<f64>,
    pub(crate) temperature: Availability<TemperatureRange>,
    pub(crate) power_w: Availability<f64>,
    pub(crate) efficiency_j_th: Availability<f64>,
    pub(crate) mcr_percent: Availability<f64>,
    pub(crate) fan_percent: Availability<f64>,
    pub(crate) uptime_s: Availability<u64>,
    pub(crate) ip_address: Availability<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TemperatureRange {
    pub(crate) board_c: f64,
    pub(crate) chip_c: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PublicData {
    pub(crate) btc_price: Availability<Money>,
    pub(crate) btc_change_24h_percent: Availability<f64>,
    pub(crate) network_hashrate_ehs: Availability<f64>,
    pub(crate) prev_diff_adjust_percent: Availability<f64>,
    pub(crate) est_diff_adjust_percent: Availability<f64>,
    pub(crate) epoch_progress_percent: Availability<f64>,
    pub(crate) avg_fee_btc: Availability<f64>,
    pub(crate) avg_fee_percent: Availability<f64>,
    pub(crate) block_height: Availability<u64>,
    pub(crate) hashprice: Availability<Money>,
    pub(crate) hashvalue_sat_th_day: Availability<f64>,
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

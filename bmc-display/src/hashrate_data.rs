// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_utils::number_format::NumberFormat;
use serde::Deserialize;
use slint::SharedString;

const NOT_AVAILABLE: &str = "--";

#[derive(Clone, Debug, Deserialize, Default)]
pub struct HashrateData {
    avg_fees_per_block: Option<f32>,
    current_hashrate: Option<f32>,
    fees_percent: Option<f32>,
    hash_price_currency: Option<f32>,
    rev_currency: Option<f32>,
}

impl HashrateData {
    #[must_use]
    pub fn avg_fees_per_block(&self, number_format: NumberFormat) -> SharedString {
        self.avg_fees_per_block
            .map_or(NOT_AVAILABLE.into(), |avg_fees_per_block| {
                SharedString::from(format!(
                    "{} BTC",
                    number_format.format_number(avg_fees_per_block, 3),
                ))
            })
    }

    #[must_use]
    pub fn fees_percent(&self, number_format: NumberFormat) -> SharedString {
        self.fees_percent
            .map_or(NOT_AVAILABLE.into(), |fees_percent| {
                SharedString::from(format!(
                    "{} %",
                    number_format.format_number(fees_percent, 2)
                ))
            })
    }

    #[must_use]
    pub fn current_hashrate(&self, number_format: NumberFormat) -> SharedString {
        self.current_hashrate
            .map_or(NOT_AVAILABLE.into(), |current_hashrate| {
                SharedString::from(format!(
                    "{} EH/s",
                    number_format.format_number(current_hashrate, 1)
                ))
            })
    }

    #[must_use]
    pub fn hashprice(&self, number_format: NumberFormat) -> SharedString {
        self.hash_price_currency
            .map_or(NOT_AVAILABLE.into(), |hash_price_currency| {
                SharedString::from(format!(
                    "{} USD/PH/Day",
                    number_format.format_number(1000.0 * hash_price_currency, 0)
                ))
            })
    }

    #[must_use]
    pub fn total_revenue(&self, number_format: NumberFormat) -> SharedString {
        self.rev_currency
            .map_or(NOT_AVAILABLE.into(), |rev_currency| {
                SharedString::from(format!(
                    "{}M USD",
                    number_format.format_number(rev_currency / 1_000_000.0, 2)
                ))
            })
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::Deserialize;

const NOT_AVAILABLE: &str = "--";

#[derive(Clone, Copy, Deserialize, Debug, Default)]
pub struct BitcoinData {
    price: Option<f32>,
    percent_change_24h: Option<f32>,
}

impl BitcoinData {
    #[must_use]
    pub fn price_as_shared(self) -> slint::SharedString {
        self.price.map_or(NOT_AVAILABLE.into(), |price| {
            slint::SharedString::from(format!("{price}"))
        })
    }

    #[must_use]
    pub fn price_change_as_shared(self) -> slint::SharedString {
        self.percent_change_24h
            .map_or(NOT_AVAILABLE.into(), |percent_change_24h| {
                slint::SharedString::from(format!("{percent_change_24h:+.2}%"))
            })
    }

    #[must_use]
    pub fn price_change_24h(self) -> Option<f32> {
        self.percent_change_24h
    }

    #[must_use]
    pub fn increasing_trend(&self) -> bool {
        self.percent_change_24h
            .is_some_and(|percent_change_24h| percent_change_24h >= 0.0)
    }
}

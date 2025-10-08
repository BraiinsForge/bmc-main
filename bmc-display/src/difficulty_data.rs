// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_utils::number_format::NumberFormat;
use serde::Deserialize;
use slint::SharedString;

const NOT_AVAILABLE: &str = "--";
const TERA: f64 = 1_000_000_000_000.0;
const BLOCKS_EPOCH: u32 = 2016;

#[derive(Clone, Debug, Deserialize, Default)]
pub struct DifficultyData {
    difficulty: Option<f64>,
    estimated_adjustment: Option<f32>,
    previous_adjustment: Option<f32>,
    block_epoch: Option<u32>, // Mined blocks in this epoch
    epoch_block_time: Option<u32>,
}

impl DifficultyData {
    #[must_use]
    pub fn difficulty_as_shared(&self, number_format: NumberFormat) -> SharedString {
        self.difficulty.map_or(NOT_AVAILABLE.into(), |difficulty| {
            SharedString::from(format!(
                "{} T",
                number_format.format_number(difficulty / TERA, 1)
            ))
        })
    }

    #[must_use]
    pub fn prev_adjust_as_shared(&self, number_format: NumberFormat) -> SharedString {
        self.previous_adjustment
            .map_or(NOT_AVAILABLE.into(), |previous_adjustment| {
                let plus_symbol = if previous_adjustment.is_sign_positive() {
                    "+"
                } else {
                    ""
                };
                SharedString::from(format!(
                    "{plus_symbol}{}%",
                    number_format.format_number(100.0 * previous_adjustment, 1)
                ))
            })
    }

    #[must_use]
    pub fn est_adjust_as_shared(&self, number_format: NumberFormat) -> SharedString {
        self.estimated_adjustment
            .map_or(NOT_AVAILABLE.into(), |estimated_adjustment| {
                let plus_symbol = if estimated_adjustment.is_sign_positive() {
                    "+"
                } else {
                    ""
                };
                SharedString::from(format!(
                    "{plus_symbol}{}%",
                    number_format.format_number(100.0 * estimated_adjustment, 1)
                ))
            })
    }

    #[must_use]
    pub fn block_epoch(&self, number_format: NumberFormat) -> SharedString {
        self.block_epoch
            .map_or(NOT_AVAILABLE.into(), |block_epoch| {
                SharedString::from(format!(
                    "{}/{}",
                    number_format.clone().format_number(block_epoch, 0),
                    number_format.format_number(BLOCKS_EPOCH, 0),
                ))
            })
    }

    #[must_use]
    pub fn epoch_block_time(&self) -> SharedString {
        self.epoch_block_time
            .map_or(NOT_AVAILABLE.into(), |epoch_block_time| {
                #[expect(clippy::integer_division)]
                let minutes = epoch_block_time / 60;
                let seconds = epoch_block_time % 60;
                SharedString::from(format!("{minutes}:{seconds}"))
            })
    }
}

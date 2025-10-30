// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_utils::number_format::NumberFormat;
use chrono::{DateTime, NaiveDateTime, ParseError, Utc};
use serde::Deserialize;
use slint::SharedString;

const NOT_AVAILABLE: &str = "--";
const TERA: f64 = 1_000_000_000_000.0;
const BLOCKS_EPOCH: u32 = 2016;
const DATETIME_STR_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.6f";

#[derive(Clone, Debug, Deserialize, Default)]
pub struct DifficultyData {
    difficulty: Option<f64>,
    estimated_adjustment: Option<f32>,
    estimated_adjustment_date: Option<String>,
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
    pub fn prev_adjust_increase(&self) -> bool {
        self.previous_adjustment
            .is_some_and(|previous_adjustment| previous_adjustment >= 0.0)
    }

    #[must_use]
    pub fn prev_adjust_time(&self) -> SharedString {
        let Some(block_epoch) = self.block_epoch else {
            return SharedString::default();
        };
        let Some(epoch_block_time) = self.epoch_block_time else {
            return SharedString::default();
        };

        let prev_adjust_time = (block_epoch * epoch_block_time).div_euclid(3600 * 24);
        let days = if prev_adjust_time > 1 { "days" } else { "day" };
        SharedString::from(format!("{prev_adjust_time} {days} ago"))
    }

    #[must_use]
    pub fn next_adjust_as_shared(&self, number_format: NumberFormat) -> SharedString {
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
    pub fn next_adjust_increase(&self) -> bool {
        self.estimated_adjustment
            .is_some_and(|estimated_adjustment| estimated_adjustment >= 0.0)
    }

    #[must_use]
    pub fn next_adjust_time(&self) -> SharedString {
        self.estimated_adjustment_date.clone().map_or(
            SharedString::default(),
            |estimated_adjustment_date| {
                if let Ok(next_adjust_time) = parse_to_datetime_utc(&estimated_adjustment_date) {
                    let now = Utc::now();
                    let in_days = (next_adjust_time - now).abs().num_days();
                    let days = if in_days > 1 { "days" } else { "day" };
                    SharedString::from(format!("in ~ {in_days} {days}"))
                } else {
                    SharedString::default()
                }
            },
        )
    }

    #[must_use]
    pub fn block_epoch(&self, number_format: NumberFormat) -> SharedString {
        self.block_epoch
            .map_or(NOT_AVAILABLE.into(), |block_epoch| {
                SharedString::from(format!(
                    "{}/{}",
                    number_format.format_number(block_epoch, 0),
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

fn parse_to_datetime_utc(s: &str) -> Result<DateTime<Utc>, ParseError> {
    let naive = NaiveDateTime::parse_from_str(s, DATETIME_STR_FORMAT)?;
    Ok(DateTime::from_utc(naive, Utc))
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_time::time::{DateFormat, Timezone};
use bmc_shared_utils::number_format::NumberFormat;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

const NOT_AVAILABLE: &str = "--";
pub const BLOCK_HEIGHT_API_URL: &str = "https://public-api.braiins.com/v2/blocks";
pub const BLOCK_HEIGHT_LIMIT_API_PARAM: &str = "limit";

#[derive(Clone, Debug, Deserialize, Default)]
pub struct BlockheightData {
    height: Option<u32>,
    timestamp: Option<String>,
}

const TIMESTAMP_FORMAT: &str = "%FT%T%z";
const NAIVE_TIMESTAMP_FORMAT: &str = "%FT%T";
const FORMAT_24H: &str = "%H:%M";
const FORMAT_12H: &str = "%I:%M %p";

impl BlockheightData {
    #[must_use]
    pub fn blockheight_as_shared(self, number_format: NumberFormat) -> slint::SharedString {
        self.height.map_or(NOT_AVAILABLE.into(), |height| {
            slint::SharedString::from(number_format.format_number(height, 0))
        })
    }

    #[must_use]
    pub fn timestamp_as_shared(
        self,
        timezone: &Timezone,
        is_24_format: bool,
        date_format: DateFormat,
    ) -> slint::SharedString {
        self.timestamp
            .map_or(NOT_AVAILABLE.into(), |mut timestamp| {
                timestamp.push_str("+0000");
                let date_time = DateTime::parse_from_str(&timestamp, TIMESTAMP_FORMAT)
                    .map(|timestamp| {
                        let timestamp = timestamp.with_timezone(timezone.chrono()).fixed_offset();
                        let mut date_time = timestamp
                            .date()
                            .format(date_format.format_string())
                            .to_string();
                        let time_format = if is_24_format { FORMAT_24H } else { FORMAT_12H };
                        date_time.push_str(", ");
                        date_time.push_str(&timestamp.time().format(time_format).to_string());
                        date_time
                    })
                    .ok();
                slint::SharedString::from(date_time.unwrap_or(NOT_AVAILABLE.to_owned()))
            })
    }

    #[must_use]
    pub fn timestamp_as_datetime(&self) -> Option<DateTime<Utc>> {
        self.timestamp
            .as_ref()
            .and_then(|timestamp| {
                NaiveDateTime::parse_from_str(timestamp, NAIVE_TIMESTAMP_FORMAT).ok()
            })
            .map(|naive| naive.and_utc())
    }
}

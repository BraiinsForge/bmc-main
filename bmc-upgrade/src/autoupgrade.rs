// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use anyhow::anyhow;
use bmc_grpc::web::AutoUpgradeFrequency as GrpcAutoUpgradeFrequency;
use bmc_scheduler::{Cron, jobs::BoxedTask};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use chrono_tz::{Tz, TzOffset};
use croner::parser::{CronParser, Seconds};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast::Sender;
use tokio::time::Duration;
use tracing::{debug, warn};

const AUTOUPGRADE_MINIMUM_UPTIME: Duration = Duration::from_secs(60 * 60); // 1 hour
const SECONDS_IN_DAY: u64 = 60 * 60 * 24; // 86,400 seconds
const SECONDS_IN_WEEK: u64 = SECONDS_IN_DAY * 7; // 604,800 seconds
const SECONDS_IN_TWO_WEEKS: u64 = SECONDS_IN_WEEK * 2; // 1,209,600 seconds
const SECONDS_IN_MONTH: u64 = SECONDS_IN_WEEK * 4; // 2,592,000 seconds
const CRON_BIWEEKLY_DAYS: &str = "1,15";

#[derive(Debug, Deserialize, Serialize, Clone, Copy, Eq, PartialEq)]
pub enum UpgradeStatus {
    NotStarted,
    DownloadReady,
    InProgress,
    Success,
    Failed,
}

#[derive(Deserialize, Serialize, Clone, Copy, Eq, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoUpgradeFrequency {
    Daily = 1,
    #[default]
    Weekly = 2,
    BiWeekly = 3,
    Monthly = 4,
}

impl From<i32> for AutoUpgradeFrequency {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Daily,
            2 => Self::Weekly,
            3 => Self::BiWeekly,
            4 => Self::Monthly,
            _ => Self::default(),
        }
    }
}

impl AutoUpgradeFrequency {
    #[must_use]
    pub fn to_seconds(&self) -> u64 {
        match self {
            AutoUpgradeFrequency::Daily => SECONDS_IN_DAY,
            AutoUpgradeFrequency::Weekly => SECONDS_IN_WEEK,
            AutoUpgradeFrequency::BiWeekly => SECONDS_IN_TWO_WEEKS,
            AutoUpgradeFrequency::Monthly => SECONDS_IN_MONTH,
        }
    }
}

impl From<GrpcAutoUpgradeFrequency> for AutoUpgradeFrequency {
    fn from(value: GrpcAutoUpgradeFrequency) -> Self {
        match value {
            GrpcAutoUpgradeFrequency::Daily => Self::Daily,
            GrpcAutoUpgradeFrequency::Weekly => Self::Weekly,
            GrpcAutoUpgradeFrequency::BiWeekly => Self::BiWeekly,
            GrpcAutoUpgradeFrequency::Monthly => Self::Monthly,
            GrpcAutoUpgradeFrequency::Unspecified => Self::default(),
        }
    }
}

impl From<AutoUpgradeFrequency> for GrpcAutoUpgradeFrequency {
    fn from(value: AutoUpgradeFrequency) -> Self {
        match value {
            AutoUpgradeFrequency::Daily => Self::Daily,
            AutoUpgradeFrequency::Weekly => Self::Weekly,
            AutoUpgradeFrequency::BiWeekly => Self::BiWeekly,
            AutoUpgradeFrequency::Monthly => Self::Monthly,
        }
    }
}

impl From<AutoUpgradeFrequency> for i32 {
    fn from(value: AutoUpgradeFrequency) -> Self {
        let web_freq: GrpcAutoUpgradeFrequency = value.into();
        web_freq as i32
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AutoUpgradeConfig {
    pub enabled: bool,
    pub frequency: AutoUpgradeFrequency,
    pub cron: Cron,
}

impl Default for AutoUpgradeConfig {
    fn default() -> Self {
        let frequency = AutoUpgradeFrequency::default();
        let date = get_date_from_frequency(
            frequency,
            true,
            Tz::UTC.offset_from_utc_date(&NaiveDate::default()),
        );
        Self {
            enabled: false,
            frequency,
            cron: build_cron_from_frequency_date(frequency, date),
        }
    }
}

impl AutoUpgradeConfig {
    #[must_use]
    pub fn new(enabled: bool, frequency: AutoUpgradeFrequency, timezone_offset: TzOffset) -> Self {
        let date = get_date_from_frequency(frequency, true, timezone_offset);
        Self {
            enabled,
            frequency,
            cron: build_cron_from_frequency_date(frequency, date),
        }
    }
}
#[derive(Clone)]
pub struct AutoUpgrade {
    pub task: Arc<BoxedTask>,
    pub sender: Sender<()>,
}

impl Debug for AutoUpgrade {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoUpgrade")
            .field("sender", &self.sender)
            .field("task", &self.sender) // Intentionally not printing the task
            .finish()
    }
}

impl AutoUpgrade {
    pub const AUTOUPGRADE_SOURCE_NAME: &str = "autoupgrade";
    #[must_use]
    pub fn new(sender: Sender<()>, start_time: Instant) -> Self {
        let task = {
            let sender_clone = sender.clone();
            move || Self::autoupgrade_task(sender_clone.clone(), start_time)
        };
        let task: BoxedTask = Box::new(task);
        Self {
            task: Arc::new(task),
            sender,
        }
    }

    fn autoupgrade_task(
        sender: Sender<()>,
        start_time: Instant,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            if let Err(e) = AutoUpgrade::can_upgrade(start_time) {
                debug!("{e}");
                return;
            }
            // Send notification to SystemUpgrade
            if sender.send(()).is_err() {
                warn!("Failed to send autoupgrade task signal");
            }
        })
    }

    fn can_upgrade(start_time: Instant) -> anyhow::Result<()> {
        let uptime = start_time.elapsed();
        if uptime.as_secs() < AUTOUPGRADE_MINIMUM_UPTIME.as_secs() {
            return Err(anyhow!(
                "Cannot upgrade: uptime is insufficient ({} seconds required).",
                AUTOUPGRADE_MINIMUM_UPTIME.as_secs()
            ));
        }

        Ok(())
    }
}

#[must_use]
fn get_date_from_frequency(
    frequency: AutoUpgradeFrequency,
    random_time_of_day: bool,
    tz_offset: TzOffset,
) -> DateTime<Tz> {
    let divisor = frequency.to_seconds();
    let now: DateTime<Tz> = DateTime::from_naive_utc_and_offset(
        NaiveDateTime::new(NaiveDate::default(), NaiveTime::default()),
        tz_offset,
    );
    now + Duration::from_secs(if random_time_of_day {
        rand::random::<u64>() % divisor
    } else {
        divisor
    })
}
fn frequency_to_partial_cron_pattern(
    frequency: AutoUpgradeFrequency,
    date: DateTime<Tz>,
) -> String {
    match frequency {
        AutoUpgradeFrequency::Daily => "* * *".to_owned(),
        AutoUpgradeFrequency::Weekly => {
            let days_of_week = date.weekday().number_from_monday().to_string();
            format!("* * {days_of_week}")
        }
        AutoUpgradeFrequency::BiWeekly => format!("{CRON_BIWEEKLY_DAYS} * *"),
        AutoUpgradeFrequency::Monthly => {
            let day = date.day().to_string();
            format!("{day} * *")
        }
    }
}

/// Builds a CronBuilder with day, month, and days_of_week fields configured based on frequency
#[must_use]
fn build_cron_from_frequency_date(frequency: AutoUpgradeFrequency, date: DateTime<Tz>) -> Cron {
    let cron_parser = CronParser::builder()
        .seconds(Seconds::Required)
        .dom_and_dow(true)
        .build();
    let pattern = format!(
        "{} {} {} {}",
        date.second(),
        date.minute(),
        date.hour(),
        frequency_to_partial_cron_pattern(frequency, date)
    );

    cron_parser
        .parse(&pattern)
        .expect("BUG: Invalid cron pattern format")
}

#[cfg(test)]
mod test {
    use super::*;
    use bmc_shared_time::time::Timezone;
    use std::str::FromStr;

    #[test]
    fn test_get_cron_from_frequency() {
        let d = NaiveDate::from_ymd_opt(2009, 1, 3).unwrap();
        let t = NaiveTime::from_hms_milli_opt(19, 15, 5, 0).unwrap();
        let date = NaiveDateTime::new(d, t);
        let tz_offset = Timezone::from_str(Tz::Etc__GMT.name())
            .expect("BUG: Invalid timezone")
            .current_timezone_tz_offset();
        let date: DateTime<Tz> = DateTime::from_naive_utc_and_offset(date, tz_offset);

        let cron = build_cron_from_frequency_date(AutoUpgradeFrequency::Daily, date);
        assert_eq!(cron.pattern.as_str(), "5 15 19 * * *");

        let cron = build_cron_from_frequency_date(AutoUpgradeFrequency::Weekly, date);
        assert_eq!(cron.pattern.as_str(), "5 15 19 * * 6");

        let cron = build_cron_from_frequency_date(AutoUpgradeFrequency::BiWeekly, date);
        assert_eq!(
            cron.pattern.as_str(),
            format!("5 15 19 {CRON_BIWEEKLY_DAYS} * *").as_str()
        );

        let cron = build_cron_from_frequency_date(AutoUpgradeFrequency::Monthly, date);
        assert_eq!(cron.pattern.as_str(), "5 15 19 3 * *");
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::anyhow;
use bmc_grpc::web::AutoUpgradeFrequency as GrpcAutoUpgradeFrequency;
use bmc_scheduler::{Cron, jobs::BoxedTask};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
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
const SECONDS_IN_FOUR_WEEKS: u64 = SECONDS_IN_WEEK * 4; // 2,592,000 seconds
pub const SECONDS_DEVICE_SETUP_DELAY: i64 = 5;
const CRON_BIWEEKLY_DAYS: &str = "1,15";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UpgradeStatus {
    NotStarted,
    DownloadReady,
    InProgress,
    Success,
    Failed,
}

#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub enum AutoUpgradeFrequency {
    Daily = 1,
    #[default]
    Weekly = 2,
    BiWeekly = 3,
    Monthly = 4,
}

impl AutoUpgradeFrequency {
    #[must_use]
    pub fn to_seconds(&self) -> u64 {
        match self {
            AutoUpgradeFrequency::Daily => SECONDS_IN_DAY,
            AutoUpgradeFrequency::Weekly => SECONDS_IN_WEEK,
            AutoUpgradeFrequency::BiWeekly => SECONDS_IN_TWO_WEEKS,
            AutoUpgradeFrequency::Monthly => SECONDS_IN_FOUR_WEEKS,
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

impl From<&Cron> for AutoUpgradeFrequency {
    fn from(value: &Cron) -> Self {
        let pattern = value.pattern.as_str();

        // Check for biweekly pattern (contains "1,15")
        if pattern.contains(CRON_BIWEEKLY_DAYS) {
            return Self::BiWeekly;
        }

        // Split pattern to analyze day/month/weekday parts
        let parts: Vec<&str> = pattern.split_whitespace().collect();
        if parts.len() < 6 {
            return Self::default();
        }

        let day = parts[3];
        let month = parts[4];
        let weekday = parts[5];

        // Daily: "* * *" for day/month/weekday
        if day == "*" && month == "*" && weekday == "*" {
            return Self::Daily;
        }

        // Weekly: specific weekday (0-7), wildcard day and month
        if day == "*" && month == "*" && weekday != "*" {
            return Self::Weekly;
        }

        // Monthly: specific day, wildcard month and weekday
        if day != "*" && month == "*" && weekday == "*" {
            return Self::Monthly;
        }

        // Default fallback
        Self::default()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
pub struct AutoUpgradeConfig {
    pub enabled: bool,
    pub cron: Option<Cron>,
}

impl AutoUpgradeConfig {
    #[must_use]
    pub fn new(
        enabled: bool,
        frequency: AutoUpgradeFrequency,
        time_of_day: Option<NaiveTime>,
        timezone_offset: TzOffset,
    ) -> Self {
        let date = get_date_from_frequency(frequency, time_of_day, timezone_offset);
        Self {
            enabled,
            cron: Some(build_cron_from_frequency_date(frequency, date)),
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
    /// When supplied, the start time is used to determine if the upgrade should be performed.
    /// Otherwise, the upgrade is performed immediately, when available
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
            if let Err(e) = can_upgrade(start_time) {
                debug!("{e}");
                return;
            }
            // Send notification to SystemUpgrade
            if sender.send(()).is_err() {
                warn!("Failed to send autoupgrade task signal");
            }
        })
    }
}

fn can_upgrade(start_time: Instant) -> anyhow::Result<()> {
    check_uptime(start_time, AUTOUPGRADE_MINIMUM_UPTIME)?;
    Ok(())
}

/// When specified, time_of_day is used as the time of day for the first run.
/// Otherwise, a random time of day is used.
#[must_use]
fn get_date_from_frequency(
    frequency: AutoUpgradeFrequency,
    time_of_day: Option<NaiveTime>,
    tz_offset: TzOffset,
) -> DateTime<Tz> {
    let divisor = frequency.to_seconds();
    DateTime::from_naive_utc_and_offset(
        NaiveDateTime::new(
            NaiveDate::default(),
            time_of_day.unwrap_or(
                NaiveTime::default() + Duration::from_secs(rand::random::<u64>() % divisor),
            ),
        ),
        tz_offset,
    )
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

/// Builds a Cron with day, month, and days_of_week fields configured based on frequency
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

fn check_uptime(start_time: Instant, minimum_uptime: Duration) -> anyhow::Result<()> {
    let uptime = start_time.elapsed();
    if uptime.as_secs() < minimum_uptime.as_secs() {
        return Err(anyhow!(
            "Cannot upgrade: uptime is insufficient ({} seconds required).",
            minimum_uptime.as_secs()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use bmc_shared_time::time::Timezone;
    use std::str::FromStr;

    #[test]
    fn test_get_cron_from_frequency() {
        let d = NaiveDate::from_ymd_opt(2009, 1, 3).unwrap_or_default();
        let t = NaiveTime::from_hms_milli_opt(19, 15, 5, 0).unwrap_or_default();
        let date = NaiveDateTime::new(d, t);
        let tz_offset = Timezone::from_str(Tz::Etc__GMT.name())
            .expect("BUG: Invalid timezone")
            .chrono_offset();
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

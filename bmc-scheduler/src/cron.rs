// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
pub use croner::{Cron, errors::CronError};

#[derive(Debug, Clone, Default)]
pub struct CronBuilder {
    seconds: Option<String>,
    minutes: Option<String>,
    hours: Option<String>,
    days: Option<String>,
    months: Option<String>,
    days_of_week: Option<String>,
}

impl CronBuilder {
    #[must_use] pub fn new() -> Self {
        Self {
            seconds: None,
            minutes: None,
            hours: None,
            days: None,
            months: None,
            days_of_week: None,
        }
    }

    #[must_use] pub fn seconds(mut self, seconds: &str) -> Self {
        self.seconds = Some(seconds.to_owned());
        self
    }

    #[must_use] pub fn minutes(mut self, minutes: &str) -> Self {
        self.minutes = Some(minutes.to_owned());
        self
    }

    #[must_use] pub fn hours(mut self, hours: &str) -> Self {
        self.hours = Some(hours.to_owned());
        self
    }

    #[must_use] pub fn days(mut self, days: &str) -> Self {
        self.days = Some(days.to_owned());
        self
    }

    #[must_use] pub fn months(mut self, months: &str) -> Self {
        self.months = Some(months.to_owned());
        self
    }

    #[must_use] pub fn days_of_week(mut self, days_of_week: &str) -> Self {
        self.days_of_week = Some(days_of_week.to_owned());
        self
    }

    /// Build the cron pattern string
    pub fn build(self) -> Result<Cron> {
        let seconds = self.seconds.map_or("0".to_owned(), |s| s);
        let minutes = self.minutes.unwrap_or_else(|| "*".to_owned());
        let hours = self.hours.unwrap_or_else(|| "*".to_owned());
        let days = self.days.unwrap_or_else(|| "*".to_owned());
        let months = self.months.unwrap_or_else(|| "*".to_owned());
        let days_of_week = self.days_of_week.unwrap_or_else(|| "*".to_owned());

        Cron::new(&format!(
            "{seconds} {minutes} {hours} {days} {months} {days_of_week}",
        )).with_seconds_required()
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse cron pattern: {e}"))
    }

    /// Convenience method for common patterns
    pub fn every_minute() -> Result<Cron> {
        CronBuilder::new().seconds("0").build()
    }

    pub fn every_hour() -> Result<Cron> {
        CronBuilder::new().seconds("0").minutes("0").build()
    }

    pub fn daily_at(hour: u8, minute: u8) -> Result<Cron> {
        CronBuilder::new()
            .seconds("0")
            .minutes(&minute.to_string())
            .hours(&hour.to_string())
            .build()
    }

    pub fn weekly_on(day_of_week: u8, hour: u8, minute: u8) -> Result<Cron> {
        CronBuilder::new()
            .seconds("0")
            .minutes(&minute.to_string())
            .hours(&hour.to_string())
            .days_of_week(&day_of_week.to_string())
            .build()
    }

    #[must_use] pub fn from_cron(cron: &Cron) -> Self {
        let pattern = cron.pattern.as_str();
        let parts = pattern.split_whitespace().collect::<Vec<&str>>();
        Self {
            seconds: if parts[0] == "*" { None } else { Some(parts[0].to_owned()) },
            minutes: if parts[1] == "*" { None } else { Some(parts[1].to_owned()) },
            hours: if parts[2] == "*" { None } else { Some(parts[2].to_owned()) },
            days: if parts[3] == "*" { None } else { Some(parts[3].to_owned()) },
            months: if parts[4] == "*" { None } else { Some(parts[4].to_owned()) },
            days_of_week: if parts[5] == "*" { None } else { Some(parts[5].to_owned()) },
        }
    }
}

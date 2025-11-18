// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt;

/// Display mode for countdown widget — determines which time units are shown
#[derive(Copy, Clone, Debug, Default)]
pub enum DisplayMode {
    Days,
    Hours,
    #[default]
    Minutes,
}

/// Represents the countdown display state
#[derive(Clone, Debug, Default)]
pub struct CountdownData {
    /// Total remaining seconds (0 if countdown completed)
    pub remaining_seconds: i64,
    /// Whether the countdown has completed
    pub is_completed: bool,
    /// Formatted display strings based on remaining time
    pub display: CountdownDisplay,
}

/// Display format for countdown widget
#[derive(Clone, Debug, Default)]
pub struct CountdownDisplay {
    /// Primary value (days or hours depending on remaining time)
    pub primary_value: String,
    /// Primary unit label
    pub primary_unit: String,
    /// Secondary value (hours or minutes)
    pub secondary_value: String,
    /// Secondary unit label
    pub secondary_unit: String,
    /// Tertiary value (minutes or seconds)
    pub tertiary_value: String,
    /// Tertiary unit label
    pub tertiary_unit: String,
    /// Which time units are being displayed
    pub display_mode: DisplayMode,
}

impl CountdownData {
    /// Create a new CountdownData from a target timestamp
    #[must_use]
    pub fn new(target_timestamp: i64, now: i64) -> Self {
        let remaining = target_timestamp - now;

        if remaining <= 0 {
            return Self {
                remaining_seconds: 0,
                is_completed: true,
                display: CountdownDisplay {
                    primary_value: String::from("00"),
                    primary_unit: String::from("MIN"),
                    secondary_value: String::from("00"),
                    secondary_unit: String::from("SEC"),
                    tertiary_value: String::new(),
                    tertiary_unit: String::new(),
                    display_mode: DisplayMode::Minutes,
                },
            };
        }

        let display = Self::calculate_display(remaining);

        Self {
            remaining_seconds: remaining,
            is_completed: false,
            display,
        }
    }

    /// Calculate the display format based on remaining seconds
    fn calculate_display(remaining_seconds: i64) -> CountdownDisplay {
        const SECONDS_PER_MINUTE: i64 = 60;
        const SECONDS_PER_HOUR: i64 = 3600;
        const SECONDS_PER_DAY: i64 = 86400;

        let days = remaining_seconds / SECONDS_PER_DAY;
        let hours = (remaining_seconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR;
        let minutes = (remaining_seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
        let seconds = remaining_seconds % SECONDS_PER_MINUTE;

        if remaining_seconds >= SECONDS_PER_DAY {
            // >= 1 day: show days, hours, minutes (DD:HH:MM)
            CountdownDisplay {
                primary_value: format!("{days:02}"),
                primary_unit: if days == 1 {
                    String::from("DAY")
                } else {
                    String::from("DAYS")
                },
                secondary_value: format!("{hours:02}"),
                secondary_unit: if hours == 1 {
                    String::from("HR")
                } else {
                    String::from("HRS")
                },
                tertiary_value: format!("{minutes:02}"),
                tertiary_unit: String::from("MIN"),
                display_mode: DisplayMode::Days,
            }
        } else if remaining_seconds >= SECONDS_PER_HOUR {
            // >= 1 hour but < 1 day: show hours, minutes, seconds (HH:MM:SS)
            CountdownDisplay {
                primary_value: format!("{hours:02}"),
                primary_unit: if hours == 1 {
                    String::from("HR")
                } else {
                    String::from("HRS")
                },
                secondary_value: format!("{minutes:02}"),
                secondary_unit: String::from("MIN"),
                tertiary_value: format!("{seconds:02}"),
                tertiary_unit: String::from("SEC"),
                display_mode: DisplayMode::Hours,
            }
        } else {
            // < 1 hour: show minutes and seconds (MM:SS)
            CountdownDisplay {
                primary_value: format!("{minutes:02}"),
                primary_unit: String::from("MIN"),
                secondary_value: format!("{seconds:02}"),
                secondary_unit: String::from("SEC"),
                tertiary_value: String::new(),
                tertiary_unit: String::new(),
                display_mode: DisplayMode::Minutes,
            }
        }
    }
}

impl fmt::Display for CountdownData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.display.primary_value,
            self.display.primary_unit,
            self.display.secondary_value,
            self.display.secondary_unit
        )?;
        if !self.display.tertiary_value.is_empty() {
            write!(
                f,
                " {} {}",
                self.display.tertiary_value, self.display.tertiary_unit
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_countdown_days() {
        let now = 1_000_000;
        let target = now + 2 * 86400 + 3 * 3600 + 45 * 60; // 2 days 3 hours 45 min
        let data = CountdownData::new(target, now);

        assert!(matches!(data.display.display_mode, DisplayMode::Days));
        assert!(!data.is_completed);
        assert_eq!(data.display.primary_value, "02");
        assert_eq!(data.display.primary_unit, "DAYS");
        assert_eq!(data.display.secondary_value, "03");
        assert_eq!(data.display.secondary_unit, "HRS");
        assert_eq!(data.display.tertiary_value, "45");
        assert_eq!(data.display.tertiary_unit, "MIN");
    }

    #[test]
    fn test_countdown_hours() {
        let now = 1_000_000;
        let target = now + 5 * 3600 + 30 * 60 + 15; // 5 hours 30 min 15 sec
        let data = CountdownData::new(target, now);

        assert!(matches!(data.display.display_mode, DisplayMode::Hours));
        assert!(!data.is_completed);
        assert_eq!(data.display.primary_value, "05");
        assert_eq!(data.display.primary_unit, "HRS");
        assert_eq!(data.display.secondary_value, "30");
        assert_eq!(data.display.secondary_unit, "MIN");
        assert_eq!(data.display.tertiary_value, "15");
        assert_eq!(data.display.tertiary_unit, "SEC");
    }

    #[test]
    fn test_countdown_minutes() {
        let now = 1_000_000;
        let target = now + 45 * 60 + 15; // 45 minutes 15 seconds
        let data = CountdownData::new(target, now);

        assert!(matches!(data.display.display_mode, DisplayMode::Minutes));
        assert!(!data.is_completed);
        assert_eq!(data.display.primary_value, "45");
        assert_eq!(data.display.primary_unit, "MIN");
        assert_eq!(data.display.secondary_value, "15");
        assert_eq!(data.display.secondary_unit, "SEC");
        assert!(data.display.tertiary_value.is_empty());
    }

    #[test]
    fn test_countdown_completed() {
        let now = 1_000_000;
        let target = now - 100; // Past timestamp
        let data = CountdownData::new(target, now);

        assert!(data.is_completed);
        assert_eq!(data.remaining_seconds, 0);
    }
}

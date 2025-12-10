// Copyright (C) 2025  Braiins Systems s.r.o.

//! Shared Slint types and components for BMC widgets.

slint::include_modules!();

/// Macro to convert a chrono DateTime to a Slint DateTime struct.
/// Use this in your widget code where you have access to the generated DateTime type.
#[macro_export]
macro_rules! to_datetime {
    ($datetime:expr, $timezone:expr, $is_24_format:expr, $date_format:expr) => {{
        use chrono::{Datelike, Timelike};
        let datetime = $datetime;
        let timezone = $timezone;
        let is_24_format = $is_24_format;
        let date_format = $date_format;

        let hour24 = i32::try_from(datetime.hour()).unwrap_or_default();
        let hour12 = i32::try_from(datetime.hour12().1).unwrap_or_default();
        let is_pm = datetime.hour12().0;
        let minute = i32::try_from(datetime.minute()).unwrap_or_default();
        let second = i32::try_from(datetime.second()).unwrap_or_default();
        let day = i32::try_from(datetime.day()).unwrap_or_default();
        let month = i32::try_from(datetime.month()).unwrap_or_default();
        let year = datetime.year();
        let weekday = slint::format!("{}", datetime.weekday());
        let time_sec_24 = slint::format!("{hour24:02}:{minute:02}:{second:02}");
        let time_sec_12 = slint::format!("{hour12:02}:{minute:02}:{second:02}");
        let time_24 = slint::format!("{hour24:02}:{minute:02}");
        let time_12 = slint::format!("{hour12:02}:{minute:02}");
        let date = slint::format!("{}", datetime.format(date_format.format_string()));

        DateTime {
            is_24_format,
            hour24,
            hour12,
            is_pm,
            minute,
            second,
            day,
            month,
            year,
            weekday,
            time_sec_24,
            time_sec_12,
            time_12,
            time_24,
            date,
            timezone: timezone.into(),
        }
    }};
}

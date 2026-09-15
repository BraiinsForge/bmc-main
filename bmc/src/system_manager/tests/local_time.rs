// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
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
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::str::FromStr;

use crate::system_manager::SystemManager;
use bmc_shared_time::time::Timezone;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

use super::DummyBacklightDriver;

type TestSystemManager = SystemManager<DummyBacklightDriver>;

/// Helper to create a UTC NaiveDateTime from local time in Prague
fn prague_local_to_utc(date: NaiveDate, time: NaiveTime, timezone: &Timezone) -> NaiveDateTime {
    let local_dt = NaiveDate::and_time(&date, time);
    timezone
        .chrono()
        .from_local_datetime(&local_dt)
        .single()
        .map_or(local_dt, |dt| dt.naive_utc()) // fallback for gap times
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_midnight() {
    // Europe/Prague on 2026-02-02 is UTC+1 (standard time, no DST)
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 2, 2).expect("BUG: invalid date");
    // Current time is 00:00 local (23:00 UTC previous day)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // Midnight in Prague (UTC+1) = 23:00 previous day in UTC = 23*60 = 1380 minutes
    assert_eq!(utc_minutes, 23 * 60);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_late_evening() {
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 2, 2).expect("BUG: invalid date");
    // Current time is 00:00 local so 23:45 hasn't passed
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(23, 45, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // 23:45 in Prague (UTC+1) = 22:45 UTC = 22*60 + 45 = 1365 minutes
    assert_eq!(utc_minutes, 22 * 60 + 45);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_summer_midnight() {
    // Europe/Prague on 2026-07-02 is UTC+2 (DST)
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 7, 2).expect("BUG: invalid date");
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // Midnight in Prague (UTC+2) = 22:00 previous day in UTC = 22*60 = 1320 minutes
    assert_eq!(utc_minutes, 22 * 60);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_summer_late_evening() {
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 7, 2).expect("BUG: invalid date");
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(23, 45, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // 23:45 in Prague (UTC+2) = 21:45 UTC = 21*60 + 45 = 1305 minutes
    assert_eq!(utc_minutes, 21 * 60 + 45);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_dst_spring_nonexistent_time() {
    // On 2026-03-29, clocks jump from 02:00 to 03:00 (CET -> CEST)
    // 02:30 doesn't exist that day - it's in the "gap"
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 3, 29).expect("BUG: invalid date");
    // Use 00:00 local which is still valid (before the gap)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(2, 30, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // Since 02:30 doesn't exist, we fall back to using the base (standard) offset.
    // Prague's base offset is UTC+1 (CET).
    // 02:30 with UTC+1 = 01:30 UTC = 1*60 + 30 = 90 minutes
    assert_eq!(utc_minutes, 60 + 30);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_dst_spring_same_day() {
    // Scenario: It's Saturday 2026-03-28 at 06:00 local (before DST switch).
    // Night mode ends at 06:30. Since 06:30 hasn't passed yet today,
    // we should use today's offset (UTC+1).
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 3, 28).expect("BUG: invalid date");
    // 06:00 local on 2026-03-28 = 05:00 UTC (Prague is UTC+1)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(6, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // 06:30 hasn't passed yet (it's 06:00 now), so use today (2026-03-28).
    // On 2026-03-28, Prague is UTC+1 (CET, before DST switch).
    // 06:30 CET = 05:30 UTC = 5*60 + 30 = 330 minutes
    assert_eq!(utc_minutes, 5 * 60 + 30);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_dst_spring_next_day() {
    // Scenario: It's Saturday 2026-03-28 at 07:00 local (before DST switch).
    // Night mode ends at 06:30. Since 06:30 has already passed today,
    // the next 06:30 will be on Sunday 2026-03-29, after the DST switch.
    // So 06:30 on Sunday should use UTC+2, not UTC+1.
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 3, 28).expect("BUG: invalid date");
    // 07:00 local on 2026-03-28 = 06:00 UTC (Prague is still UTC+1)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(7, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // 06:30 has already passed today (it's 07:00 now), so use tomorrow (2026-03-29).
    // On 2026-03-29, Prague is UTC+2 (CEST after DST switch).
    // 06:30 CEST = 04:30 UTC = 4*60 + 30 = 270 minutes
    assert_eq!(utc_minutes, 4 * 60 + 30);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_dst_fall_back_use_today() {
    // Scenario: It's Saturday 2026-10-24 at 07:00 local (still DST, UTC+2).
    // Night mode ends at 06:30. Since 06:30 has passed today,
    // normally we'd use tomorrow (Sunday 2026-10-25, after fall-back, UTC+1).
    //
    // However, using tomorrow would cause a problem:
    // - Today 06:30 CEST = 04:30 UTC (270 min)
    // - Tomorrow 06:30 CET = 05:30 UTC (330 min)
    //
    // If we returned 330 min and U-Boot checks now (05:00 UTC = 300 min), it would think
    // night mode is still on (300 < 330). So we must use today's value.
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 10, 24).expect("BUG: invalid date");
    // 07:00 local on 2026-10-24 = 05:00 UTC (Prague is still UTC+2)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(7, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // Even though 06:30 has passed, we use today's value to avoid U-Boot misinterpretation.
    // 06:30 CEST = 04:30 UTC = 4*60 + 30 = 270 minutes
    assert_eq!(utc_minutes, 4 * 60 + 30);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_dst_fall_back_use_tomorrow() {
    // Scenario: It's Saturday 2026-10-24 at 08:00 local (still DST, UTC+2).
    // Night mode ends at 06:30. Since 06:30 has passed today,
    // we should use tomorrow (Sunday 2026-10-25, after fall-back, UTC+1).
    //
    // Values:
    // - Today 06:30 CEST = 04:30 UTC (270 min)
    // - Tomorrow 06:30 CET = 05:30 UTC (330 min)
    // - Now 08:00 CEST = 06:00 UTC (360 min)
    //
    // Since now (360 min) > tomorrow's value (330 min), it's safe to use tomorrow.
    // U-Boot will correctly see that night mode has ended.
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 10, 24).expect("BUG: invalid date");
    // 08:00 local on 2026-10-24 = 06:00 UTC (Prague is still UTC+2)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(8, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // Now it's safe to use tomorrow's value.
    // 06:30 CET = 05:30 UTC = 5*60 + 30 = 330 minutes
    assert_eq!(utc_minutes, 5 * 60 + 30);
}

#[tokio::test]
async fn test_local_time_to_utc_minutes_prague_dst_fall_back_from_time() {
    // Scenario: It's Saturday 2026-10-24 at 23:00 local (still DST, UTC+2).
    // Night mode starts at 22:30. Since 22:30 has passed today,
    // normally we'd consider tomorrow.
    //
    // Values:
    // - Today 22:30 CEST = 20:30 UTC (1230 min)
    // - Tomorrow 22:30 CET = 21:30 UTC (1290 min)
    // - Now 23:00 CEST = 21:00 UTC (1260 min)
    //
    // Since now (1230 min) < tomorrow's value (1290 min), we use today's value.
    // This is correct: at 21:00 UTC, night mode should be ON (past 20:30 UTC).
    // If we used 1290, U-Boot would think night mode hasn't started yet.
    let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
    let date = NaiveDate::from_ymd_opt(2026, 10, 24).expect("BUG: invalid date");
    // 23:00 local on 2026-10-24 = 21:00 UTC (Prague is still UTC+2)
    let now_utc = prague_local_to_utc(
        date,
        NaiveTime::from_hms_opt(23, 0, 0).expect("BUG: invalid time"),
        &timezone,
    );
    let local_time = NaiveTime::from_hms_opt(22, 30, 0).expect("BUG: invalid time");

    let utc_minutes =
        TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

    // Use today's value to ensure U-Boot correctly sees night mode as ON.
    // 22:30 CEST = 20:30 UTC = 20*60 + 30 = 1230 minutes
    assert_eq!(utc_minutes, 20 * 60 + 30);
}

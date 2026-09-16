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

use super::super::*;

#[test]
fn test_weekday_to_num_string() {
    let cases = [
        (WeekDay::Monday, "1".to_owned()),
        (WeekDay::Tuesday, "2".to_owned()),
        (WeekDay::Wednesday, "3".to_owned()),
        (WeekDay::Thursday, "4".to_owned()),
        (WeekDay::Friday, "5".to_owned()),
        (WeekDay::Saturday, "6".to_owned()),
        (WeekDay::Sunday, "7".to_owned()),
    ];

    for (weekday, expected) in cases {
        let num_string = weekday.as_number_string();
        assert_eq!(num_string, expected);
    }
}

#[test]
fn test_weekday_to_string() {
    let cases = [
        (WeekDay::Monday, "Monday".to_owned()),
        (WeekDay::Tuesday, "Tuesday".to_owned()),
        (WeekDay::Wednesday, "Wednesday".to_owned()),
        (WeekDay::Thursday, "Thursday".to_owned()),
        (WeekDay::Friday, "Friday".to_owned()),
        (WeekDay::Saturday, "Saturday".to_owned()),
        (WeekDay::Sunday, "Sunday".to_owned()),
    ];

    for (weekday, expected) in cases {
        let value = weekday.to_string();
        assert_eq!(value, expected);
    }
}

#[test]
fn test_parse_simple_alarm_data_to_cron() -> anyhow::Result<()> {
    let time = NaiveTime::parse_from_str("10:30", "%H:%M")?;
    let alarm_data = AlarmData::new(false, String::new(), time, BTreeSet::new(), None, None);

    let cron = alarm_data
        .cron()
        .expect("BUG: failed to create cron from alarm data");

    let cron_string = cron.to_string();

    assert_eq!(String::from("0 30 10 * * *"), cron_string);

    Ok(())
}

#[test]
fn test_weekdays_to_num_string() {
    let cases: Vec<(BTreeSet<WeekDay>, &str)> = vec![
        (
            [WeekDay::Monday, WeekDay::Tuesday, WeekDay::Wednesday].into(),
            "1,2,3",
        ),
        (
            [WeekDay::Thursday, WeekDay::Monday, WeekDay::Sunday].into(),
            "1,4,7",
        ),
        ([].into(), ""),
    ];

    for (weekdays, expected) in cases {
        let num_string = AlarmData::weekdays_to_number_string(&weekdays);

        assert_eq!(&num_string, expected);
    }
}

#[test]
fn test_parse_alarm_data_with_repeat_to_cron() -> anyhow::Result<()> {
    let time = NaiveTime::parse_from_str("14:58", "%H:%M")?;
    let repeat = [WeekDay::Monday, WeekDay::Tuesday, WeekDay::Wednesday].into();
    let alarm_data = AlarmData::new(false, String::new(), time, repeat, None, None);

    let cron = alarm_data
        .cron()
        .expect("BUG: failed to create cron from alarm data");

    let cron_string = cron.to_string();

    assert_eq!(String::from("0 58 14 * * 1,2,3"), cron_string);

    Ok(())
}

#[test]
fn test_parse_alarm_data_with_all_days_to_cron() -> anyhow::Result<()> {
    let time = NaiveTime::parse_from_str("14:58", "%H:%M")?;
    let repeat = [
        WeekDay::Monday,
        WeekDay::Tuesday,
        WeekDay::Wednesday,
        WeekDay::Thursday,
        WeekDay::Friday,
        WeekDay::Saturday,
        WeekDay::Sunday,
    ]
    .into();

    let alarm_data = AlarmData::new(false, String::new(), time, repeat, None, None);

    let cron = alarm_data
        .cron()
        .expect("BUG: failed to create cron from alarm data");

    let cron_string = cron.to_string();

    assert_eq!(String::from("0 58 14 * * 1,2,3,4,5,6,7"), cron_string);

    Ok(())
}

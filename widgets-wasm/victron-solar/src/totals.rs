// Copyright (C) 2026  Jan Novák
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Today / month / year solar totals from a lifetime energy counter.
//!
//! Venus OS only knows the lifetime total and, per MPPT, the last 31 days
//! (MQTT) or just today and yesterday (Modbus TCP).
//! The [`Ledger`] remembers the lifetime reading at the start of the
//! current day, month and year (persisted by the caller), so each total
//! is "lifetime now − lifetime at the period's start". On first use the
//! starts are back-filled from the daily history; whatever the history
//! doesn't reach is marked "since <date>" until a full period has passed.

/// A local calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Date {
    #[must_use]
    pub fn new(y: i32, m: u32, d: u32) -> Self {
        Self { y, m, d }
    }

    fn days_in_month(y: i32, m: u32) -> u32 {
        match m {
            4 | 6 | 9 | 11 => 30,
            2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
            2 => 28,
            _ => 31,
        }
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
    fn epoch_days(self) -> i64 {
        let y = i64::from(self.y) - i64::from(self.m <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.m);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.d) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_epoch_days(z: i64) -> Self {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "month and day are 1..=31 by construction, the year is small"
        )]
        Self::new(y as i32, m as u32, d as u32)
    }

    #[must_use]
    pub fn minus_days(self, n: usize) -> Self {
        Self::from_epoch_days(self.epoch_days() - i64::try_from(n).unwrap_or(0))
    }

    /// 1 on January 1st.
    fn day_of_year(self) -> usize {
        usize::try_from(self.epoch_days() - Self::new(self.y, 1, 1).epoch_days()).unwrap_or(0) + 1
    }

    fn prev_month(self) -> (i32, u32) {
        if self.m == 1 {
            (self.y - 1, 12)
        } else {
            (self.y, self.m - 1)
        }
    }

    fn to_text(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }

    fn from_text(s: &str) -> Option<Self> {
        let mut it = s.split('-');
        let d = Self::new(
            it.next()?.parse().ok()?,
            it.next()?.parse().ok()?,
            it.next()?.parse().ok()?,
        );
        (it.next().is_none()
            && (1..=12).contains(&d.m)
            && d.d >= 1
            && d.d <= Self::days_in_month(d.y, d.m))
        .then_some(d)
    }
}

/// Lifetime readings at the start of the current periods, plus the
/// finished previous periods. This is what gets persisted.
#[derive(Clone, Debug, PartialEq)]
pub struct Ledger {
    day: Date,
    day_base: f64,
    month_base: f64,
    /// First day the month total covers, when that isn't the 1st.
    month_since: Option<Date>,
    year_base: f64,
    year_since: Option<Date>,
    last_month: Option<f64>,
    last_year: Option<f64>,
}

/// What the widget shows, in kWh.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub today: f64,
    pub month: f64,
    pub month_since: Option<Date>,
    pub last_month: Option<f64>,
    pub year: f64,
    pub year_since: Option<Date>,
    pub last_year: Option<f64>,
}

/// Before this local hour the first reading of a new day counts as its
/// start (no sun yet); later, the widget was off at midnight and the
/// MPPT's own "today" is the better estimate of what was already made.
const NIGHT_UNTIL_HOUR: u32 = 4;

/// Counter going backwards by more than this means a charger was
/// replaced or reset: start the periods over rather than show negatives.
const RESET_TOLERANCE_KWH: f64 = 0.05;

impl Ledger {
    /// Back-fill the period starts from `daily` (index 0 = today).
    #[must_use]
    pub fn seed(today: Date, lifetime: f64, daily: &[f64]) -> Self {
        let sum = |n: usize| daily.iter().take(n).sum::<f64>();
        let covered = |n: usize| -> (f64, Option<Date>) {
            if daily.len() >= n {
                (lifetime - sum(n), None)
            } else {
                // History ends before the period starts: count from its first day.
                let since = today.minus_days(daily.len().saturating_sub(1));
                (lifetime - sum(daily.len()), Some(since))
            }
        };
        let (month_base, month_since) = covered(today.d as usize);
        let (year_base, year_since) = covered(today.day_of_year());
        let (py, pm) = today.prev_month();
        let prev_days = Date::days_in_month(py, pm) as usize;
        let first = today.d as usize;
        let last_month = (daily.len() >= first + prev_days)
            .then(|| daily[first..first + prev_days].iter().sum::<f64>());
        Self {
            day: today,
            day_base: lifetime - sum(1),
            month_base,
            month_since,
            year_base,
            year_since,
            last_month,
            last_year: None,
        }
    }

    /// Account for a new reading. Returns whether the ledger changed
    /// (and so should be persisted).
    pub fn observe(&mut self, today: Date, hour: u32, lifetime: f64, daily: &[f64]) -> bool {
        if lifetime + RESET_TOLERANCE_KWH < self.day_base {
            let (lm, ly) = (self.last_month, self.last_year);
            *self = Self::seed(today, lifetime, daily);
            self.last_month = self.last_month.or(lm);
            self.last_year = ly;
            return true;
        }
        if today <= self.day {
            return false;
        }
        let made_today = if hour < NIGHT_UNTIL_HOUR {
            0.0
        } else {
            daily.first().copied().unwrap_or(0.0)
        };
        let start = lifetime - made_today;
        let prev = self.day;
        let gap_days = today.epoch_days() - prev.epoch_days();

        if (today.y, today.m) != (prev.y, prev.m) {
            // Only a month that ended right before this one, and was counted
            // from its 1st, is a full "last month".
            let adjacent = today.prev_month() == (prev.y, prev.m) && gap_days <= 31;
            self.last_month = (adjacent && self.month_since.is_none())
                .then(|| (start - self.month_base).max(0.0));
            self.month_base = start;
            self.month_since = None;
        }
        if today.y != prev.y {
            let adjacent = today.y == prev.y + 1 && gap_days <= 366;
            self.last_year =
                (adjacent && self.year_since.is_none()).then(|| (start - self.year_base).max(0.0));
            self.year_base = start;
            self.year_since = None;
        }
        self.day = today;
        self.day_base = start;
        true
    }

    #[must_use]
    pub fn totals(&self, lifetime: f64) -> Totals {
        Totals {
            today: (lifetime - self.day_base).max(0.0),
            month: (lifetime - self.month_base).max(0.0),
            month_since: self.month_since,
            last_month: self.last_month,
            year: (lifetime - self.year_base).max(0.0),
            year_since: self.year_since,
            last_year: self.last_year,
        }
    }

    /// One line of `key=value` pairs; `-` for an absent value.
    #[must_use]
    pub fn to_text(&self) -> String {
        let date = |d: Option<Date>| d.map_or_else(|| "-".to_owned(), Date::to_text);
        let num = |v: Option<f64>| v.map_or_else(|| "-".to_owned(), |v| format!("{v}"));
        format!(
            "v1 day={} db={} mb={} ms={} yb={} ys={} lm={} ly={}",
            self.day.to_text(),
            self.day_base,
            self.month_base,
            date(self.month_since),
            self.year_base,
            date(self.year_since),
            num(self.last_month),
            num(self.last_year),
        )
    }

    /// `None` for anything not written by [`Ledger::to_text`] v1.
    #[must_use]
    pub fn from_text(s: &str) -> Option<Self> {
        let mut it = s.split_ascii_whitespace();
        if it.next()? != "v1" {
            return None;
        }
        let mut get = |key: &str| -> Option<&str> {
            let (k, v) = it.next()?.split_once('=')?;
            (k == key).then_some(v)
        };
        let opt_date = |v: &str| -> Option<Option<Date>> {
            if v == "-" {
                Some(None)
            } else {
                Date::from_text(v).map(Some)
            }
        };
        let opt_num = |v: &str| -> Option<Option<f64>> {
            if v == "-" {
                Some(None)
            } else {
                v.parse().ok().map(Some)
            }
        };
        Some(Self {
            day: Date::from_text(get("day")?)?,
            day_base: get("db")?.parse().ok()?,
            month_base: get("mb")?.parse().ok()?,
            month_since: opt_date(get("ms")?)?,
            year_base: get("yb")?.parse().ok()?,
            year_since: opt_date(get("ys")?)?,
            last_month: opt_num(get("lm")?)?,
            last_year: opt_num(get("ly")?)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[wasm_test]
    fn date_arithmetic_crosses_months_years_and_leap_days() {
        assert_eq!(Date::new(2026, 3, 1).minus_days(1), Date::new(2026, 2, 28));
        assert_eq!(Date::new(2028, 3, 1).minus_days(1), Date::new(2028, 2, 29));
        assert_eq!(Date::new(2027, 1, 1).minus_days(1), Date::new(2026, 12, 31));
        assert_eq!(Date::new(2026, 10, 29).day_of_year(), 302);
        assert_eq!(Date::from_text("2026-02-29"), None);
        assert_eq!(
            Date::from_text(&Date::new(2026, 9, 30).to_text()),
            Some(Date::new(2026, 9, 30))
        );
    }

    #[wasm_test]
    fn seeding_backfills_today_and_the_month_and_marks_a_partial_year() {
        // 29 Oct, 31 days of history at 1 kWh/day, today 0.5 so far.
        let mut daily = vec![1.0; 31];
        daily[0] = 0.5;
        let l = Ledger::seed(Date::new(2026, 10, 29), 1000.0, &daily);
        let t = l.totals(1000.0);
        assert!(close(t.today, 0.5));
        assert!(
            close(t.month, 28.5),
            "29 days of October covered by history"
        );
        assert_eq!(t.month_since, None);
        assert_eq!(t.last_month, None, "September isn't fully in the 31 days");
        assert!(close(t.year, 30.5));
        assert_eq!(t.year_since, Some(Date::new(2026, 9, 29)));
        assert_eq!(t.last_year, None);
    }

    #[wasm_test]
    fn seeding_early_in_a_month_also_gets_last_month() {
        let l = Ledger::seed(Date::new(2026, 10, 1), 1000.0, &[2.0; 31]);
        let t = l.totals(1000.0);
        assert!(close(t.month, 2.0));
        assert_eq!(t.last_month, Some(60.0), "all 30 days of September");
    }

    #[wasm_test]
    fn seeding_without_history_counts_from_today() {
        let l = Ledger::seed(Date::new(2026, 10, 29), 500.0, &[]);
        let t = l.totals(503.0);
        assert!(close(t.today, 3.0) && close(t.month, 3.0) && close(t.year, 3.0));
        assert_eq!(t.month_since, Some(Date::new(2026, 10, 29)));
    }

    #[wasm_test]
    fn midnight_rollover_starts_the_day_from_the_counter_not_the_mppt_day() {
        let d = Date::new(2026, 10, 29);
        let mut l = Ledger::seed(d, 1000.0, &[5.0; 31]);
        assert!(!l.observe(d, 15, 1001.0, &[6.0; 31]));
        // 00:05 next day: the MPPT may still report yesterday as "today".
        assert!(l.observe(Date::new(2026, 10, 30), 0, 1002.0, &[7.0; 31]));
        let t = l.totals(1002.5);
        assert!(close(t.today, 0.5));
    }

    #[wasm_test]
    fn a_first_look_in_daylight_trusts_the_mppt_for_what_was_already_made() {
        let mut l = Ledger::seed(Date::new(2026, 10, 29), 1000.0, &[5.0; 31]);
        l.observe(Date::new(2026, 10, 30), 13, 1004.0, &[3.0; 31]);
        assert!(close(l.totals(1004.0).today, 3.0));
    }

    #[wasm_test]
    fn month_and_year_rollovers_close_the_finished_period() {
        // Seeded on 1 Dec so December is complete; year only partially known.
        let mut l = Ledger::seed(Date::new(2026, 12, 1), 1000.0, &[1.0; 31]);
        assert!(close(l.totals(1000.0).month, 1.0));
        l.observe(Date::new(2026, 12, 31), 0, 1030.0, &[]);
        l.observe(Date::new(2027, 1, 1), 1, 1031.0, &[]);
        let t = l.totals(1031.0);
        assert_eq!(
            t.last_month,
            Some(32.0),
            "1 kWh on the 1st, then 31 more by 1 January"
        );
        assert_eq!(t.last_year, None, "2026 was only partly counted");
        assert!(close(t.month, 0.0) && close(t.year, 0.0));
        assert_eq!((t.month_since, t.year_since), (None, None));
        // A full year later, last year is known.
        l.observe(Date::new(2027, 12, 31), 0, 2000.0, &[]);
        l.observe(Date::new(2028, 1, 1), 0, 2001.0, &[]);
        let t = l.totals(2001.0);
        assert_eq!(t.last_year, Some(970.0));
    }

    #[wasm_test]
    fn a_gap_longer_than_one_month_leaves_last_month_unknown() {
        let mut l = Ledger::seed(Date::new(2026, 10, 1), 1000.0, &[0.0; 31]);
        l.observe(Date::new(2026, 12, 5), 2, 1100.0, &[]);
        assert_eq!(l.totals(1100.0).last_month, None);
    }

    #[wasm_test]
    fn a_counter_reset_reseeds_instead_of_going_negative() {
        let mut l = Ledger::seed(Date::new(2026, 10, 29), 1000.0, &[1.0; 31]);
        assert!(l.observe(Date::new(2026, 10, 29), 12, 3.0, &[1.0; 31]));
        let t = l.totals(3.0);
        assert!(t.today >= 0.0 && t.month >= 0.0 && t.year >= 0.0);
        assert!(close(t.today, 1.0));
    }

    #[wasm_test]
    fn the_ledger_survives_a_text_round_trip() {
        let mut l = Ledger::seed(Date::new(2026, 10, 1), 1234.5, &[2.0; 31]);
        l.observe(Date::new(2026, 10, 2), 0, 1236.0, &[]);
        assert_eq!(Ledger::from_text(&l.to_text()), Some(l));
        assert_eq!(Ledger::from_text("v0 day=2026-10-01"), None);
        assert_eq!(Ledger::from_text("garbage"), None);
    }
}

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

use std::time::Duration;

use chrono::{DateTime, TimeZone, Timelike};
use rand::Rng;

pub(crate) const MAINTENANCE_MIN_DELAY: Duration = Duration::from_mins(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HourParity {
    Even,
    Odd,
}

impl HourParity {
    const fn grid_start(self) -> u32 {
        match self {
            Self::Even => 0,
            Self::Odd => 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MaintenanceStagger {
    minute: u32,
    second: u32,
}

impl MaintenanceStagger {
    pub(crate) fn draw<Tz: TimeZone>(now: &DateTime<Tz>, rng: &mut impl Rng) -> Self {
        // The one-second ceiling for fractional registration times can extend
        // the draw, so the maximum stays two seconds below the hour boundary.
        let max_delay_secs = Duration::from_hours(1).as_secs() - 2;
        let delay =
            Duration::from_secs(rng.random_range(MAINTENANCE_MIN_DELAY.as_secs()..=max_delay_secs));
        Self::from_delay(now, delay)
    }

    fn from_delay<Tz: TimeZone>(now: &DateTime<Tz>, delay: Duration) -> Self {
        let ceiling = if now.nanosecond() > 0 {
            chrono::TimeDelta::seconds(1)
        } else {
            chrono::TimeDelta::zero()
        };
        let delay = chrono::TimeDelta::from_std(delay)
            .expect("BUG: the drawn maintenance delay does not fit in a TimeDelta");
        let target = now.clone() + ceiling + delay;
        Self {
            minute: target.minute(),
            second: target.second(),
        }
    }

    pub(crate) fn pattern(self, parity: HourParity) -> String {
        // In croner, `0/2` spans the even hours and `1/2` spans the odd hours;
        // both grids continue cleanly across midnight. The grid follows civil
        // time, so a DST fall-back stretches one gap to three elapsed hours —
        // accepted, like the occurrence a clock correction can cost.
        format!(
            "{} {} {}/2 * * *",
            self.second,
            self.minute,
            parity.grid_start()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_scheduler::Cron;
    use chrono::{DateTime, TimeZone, Timelike};
    use rand::SeedableRng as _;
    use std::str::FromStr as _;

    const HOUR: Duration = Duration::from_hours(1);
    const PERIOD: Duration = Duration::from_hours(2);

    fn at(tz: chrono_tz::Tz, hour: u32, minute: u32, second: u32) -> DateTime<chrono_tz::Tz> {
        tz.with_ymd_and_hms(2026, 7, 29, hour, minute, second)
            .single()
            .expect("BUG: constructed an invalid timestamp")
    }

    fn stagger_for(now: &DateTime<chrono_tz::Tz>, delay_secs: u64) -> MaintenanceStagger {
        MaintenanceStagger::from_delay(now, Duration::from_secs(delay_secs))
    }

    fn first_occurrence(
        now: &DateTime<chrono_tz::Tz>,
        stagger: MaintenanceStagger,
        parity: HourParity,
    ) -> DateTime<chrono_tz::Tz> {
        let cron = Cron::from_str(&stagger.pattern(parity))
            .expect("BUG: derived an unparsable cron pattern");
        cron.find_next_occurrence(now, false)
            .expect("BUG: derived a pattern with no next occurrence")
    }

    fn target_parity(now: &DateTime<chrono_tz::Tz>, delay_secs: u64) -> HourParity {
        let ceiling = u64::from(now.nanosecond() > 0);
        let delay = chrono::TimeDelta::from_std(Duration::from_secs(delay_secs + ceiling))
            .expect("BUG: the test delay does not fit in a TimeDelta");
        let target = *now + delay;
        if target.hour().is_multiple_of(2) {
            HourParity::Even
        } else {
            HourParity::Odd
        }
    }

    fn opposite(parity: HourParity) -> HourParity {
        match parity {
            HourParity::Even => HourParity::Odd,
            HourParity::Odd => HourParity::Even,
        }
    }

    #[test]
    fn target_parity_selects_the_first_hour_window() {
        let starts = [
            at(chrono_tz::UTC, 23, 47, 13),
            at(chrono_tz::UTC, 23, 47, 13)
                .with_nanosecond(1)
                .expect("BUG: constructed an invalid timestamp"),
            at(chrono_tz::UTC, 23, 47, 13)
                .with_nanosecond(999_999_999)
                .expect("BUG: constructed an invalid timestamp"),
            at(chrono_tz::UTC, 0, 0, 0),
            at(chrono_tz::UTC, 11, 59, 59),
        ];

        for now in starts {
            for delay_secs in [1_800, 1_801, 2_700, 3_597, 3_598] {
                let stagger = stagger_for(&now, delay_secs);
                let matching_parity = target_parity(&now, delay_secs);
                let matching_gap = (first_occurrence(&now, stagger, matching_parity) - now)
                    .to_std()
                    .expect("BUG: the matching occurrence precedes registration");
                let opposite_gap = (first_occurrence(&now, stagger, opposite(matching_parity))
                    - now)
                    .to_std()
                    .expect("BUG: the opposite occurrence precedes registration");

                assert!(
                    matching_gap >= MAINTENANCE_MIN_DELAY && matching_gap < HOUR,
                    "delay {delay_secs}s at {now} put matching parity at {matching_gap:?}"
                );
                assert!(
                    opposite_gap >= HOUR + MAINTENANCE_MIN_DELAY && opposite_gap < PERIOD,
                    "delay {delay_secs}s at {now} put opposite parity at {opposite_gap:?}"
                );
            }
        }
    }

    #[test]
    fn parity_occurrences_alternate_hourly_across_midnight() {
        let now = at(chrono_tz::UTC, 22, 47, 13);
        let stagger = stagger_for(&now, 1_800);
        let odd = first_occurrence(&now, stagger, HourParity::Odd);
        let even = first_occurrence(&now, stagger, HourParity::Even);
        let next_odd = first_occurrence(&even, stagger, HourParity::Odd);

        assert_eq!(
            (even - odd)
                .to_std()
                .expect("BUG: parity occurrences are out of order"),
            HOUR
        );
        assert_eq!(
            (next_odd - even)
                .to_std()
                .expect("BUG: parity occurrences are out of order"),
            HOUR
        );
    }

    #[test]
    fn a_fractional_hour_zone_keeps_both_parities_inside_the_period() {
        let now = at(chrono_tz::Asia::Kathmandu, 23, 47, 13)
            .with_nanosecond(999_999_999)
            .expect("BUG: constructed an invalid timestamp");

        for delay_secs in [1_800, 1_801, 2_700, 3_597, 3_598] {
            let stagger = stagger_for(&now, delay_secs);
            for parity in [HourParity::Even, HourParity::Odd] {
                let gap = (first_occurrence(&now, stagger, parity) - now)
                    .to_std()
                    .expect("BUG: the occurrence precedes registration");

                assert!(
                    gap >= MAINTENANCE_MIN_DELAY && gap < PERIOD,
                    "delay {delay_secs}s with {parity:?} parity landed at {gap:?}"
                );
            }
        }
    }

    #[test]
    fn seeded_draws_keep_both_parities_inside_the_period() {
        let now = at(chrono_tz::UTC, 23, 47, 13)
            .with_nanosecond(999_999_999)
            .expect("BUG: constructed an invalid timestamp");
        let mut rng = rand::rngs::StdRng::seed_from_u64(634);

        for _ in 0..1_000 {
            let stagger = MaintenanceStagger::draw(&now, &mut rng);
            for parity in [HourParity::Even, HourParity::Odd] {
                let gap = (first_occurrence(&now, stagger, parity) - now)
                    .to_std()
                    .expect("BUG: the occurrence precedes registration");

                assert!(
                    gap >= MAINTENANCE_MIN_DELAY && gap < PERIOD,
                    "draw with {parity:?} parity landed at {gap:?}"
                );
            }
        }
    }
}

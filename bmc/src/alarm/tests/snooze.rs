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

fn active_alarm(snooze_options: Option<SnoozeOptions>, snooze_count: u32) -> ActiveAlarm {
    let data = AlarmData::new(
        true,
        "test".to_owned(),
        NaiveTime::from_hms_opt(7, 30, 0).expect("BUG: valid test time"),
        BTreeSet::new(),
        None,
        snooze_options,
    );
    ActiveAlarm { data, snooze_count }
}

fn snooze(limit: SnoozeLimit) -> SnoozeOptions {
    SnoozeOptions {
        limit,
        duration: SnoozeDuration::FiveMinutes,
    }
}

#[test]
fn snooze_not_allowed_without_snooze_options() {
    // Regression: a snooze request on a no-snooze alarm must be rejected up
    // front so it is not cancelled with nothing to re-fire.
    assert!(!active_alarm(None, 0).snooze_allowed());
}

#[test]
fn snooze_allowed_forever_ignores_count() {
    assert!(active_alarm(Some(snooze(SnoozeLimit::Forever)), 0).snooze_allowed());
    assert!(active_alarm(Some(snooze(SnoozeLimit::Forever)), 99).snooze_allowed());
}

#[test]
fn snooze_allowed_until_limit_reached() {
    assert!(active_alarm(Some(snooze(SnoozeLimit::Three)), 0).snooze_allowed());
    assert!(active_alarm(Some(snooze(SnoozeLimit::Three)), 2).snooze_allowed());
    assert!(!active_alarm(Some(snooze(SnoozeLimit::Three)), 3).snooze_allowed());
    assert!(!active_alarm(Some(snooze(SnoozeLimit::Three)), 4).snooze_allowed());
}

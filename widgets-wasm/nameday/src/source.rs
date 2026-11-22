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

//! Fetches today's names and refetches them once the day has passed.

use bmc_wasm_sdk::timer::Timer;
use bmc_wasm_sdk::*;

use crate::manifest_params::Country;

const NAMEDAY_API_URL_TEMPLATE: &str =
    "https://nameday.abalin.net/api/V2/date?day={DAY_PLACEHOLDER}&month={MONTH_PLACEHOLDER}";

// Checked at least every minute rather than waiting for midnight from hours ahead:
// a wait computed before a clock jump, such as the first NTP sync, ends at the wrong time.
const DAY_CHECK_MS: u32 = 60 * 1_000;

/// How far past midnight the check aimed at it is due,
/// so a tick that fires a little early still reads the new date.
const PAST_MIDNIGHT_MS: u32 = 1_000;

#[derive(Debug, Clone)]
pub(super) struct TimeWithTimezone {
    pub(super) time: SystemTime,
    pub(super) tz: Tz,
}

pub(super) struct NamedayData {
    /// Names per supported country, parsed once per fetch. `None` for a
    /// country the response omitted or that has no nameday today; cached so a
    /// country change in the params only re-renders and never triggers a fetch.
    names_per_country: [Option<String>; Country::ALL.len()],
    pub(super) fetched_at: TimeWithTimezone,
}

impl NamedayData {
    /// Names for `country`, or `None` if the response carried no usable entry.
    pub(super) fn names_for(&self, country: Country) -> Option<&str> {
        self.names_per_country
            .get(country as usize)
            .and_then(|slot| slot.as_deref())
    }
}

pub(super) type Names = Part<NamedaySource>;

pub(super) struct NamedaySource {
    poll: PollHandle,
    /// What the last request built asked for, so its reply can be checked against today;
    /// `None` until a build succeeds and after one fails, so the day check asks again.
    requested_for: Option<TimeWithTimezone>,
    /// The day of the names last sent, so the widget can be told once it has passed.
    served_for: Option<TimeWithTimezone>,
    _day_check: Timer,
    on_names: Notify<NamedayData>,
    on_day_change: Notify<()>,
}

/// Whether `then` falls on today's local date; see [`is_same_local_date`].
pub(super) fn is_today(then: &TimeWithTimezone) -> bool {
    is_same_local_date(then, SystemTime::now(), current_tz().as_ref())
}

fn current_tz() -> Option<Tz> {
    system::current().timezone().map(Tz::from_runtime)
}

/// The wait until the next day check: a minute, or less to aim
/// just past the midnight that `now` reads as coming;
/// the minute cap covers a timezone change that moves it.
fn next_check_ms(now: SystemTime, tz: Option<&Tz>) -> u32 {
    const SECS_PER_DAY: u32 = 86_400;
    let to_midnight_ms = (SECS_PER_DAY - local_or_utc(now, tz).seconds_since_midnight()) * 1_000;
    DAY_CHECK_MS.min(to_midnight_ms + PAST_MIDNIGHT_MS)
}

/// `time` read in `tz`, or in UTC when there is none or the host cannot read it.
fn local_or_utc(time: SystemTime, tz: Option<&Tz>) -> LocalDateTime {
    tz.and_then(|tz| time.local(tz))
        .unwrap_or_else(|| time.utc())
}

/// Whether `then`, read in its own timezone, falls on the date `now` reads in `now_tz`,
/// or in UTC when the device has no timezone.
fn is_same_local_date(then: &TimeWithTimezone, now: SystemTime, now_tz: Option<&Tz>) -> bool {
    let local_now = local_or_utc(now, now_tz);
    let local_then = local_or_utc(then.time, Some(&then.tz));

    (local_now.year, local_now.month, local_now.day)
        == (local_then.year, local_then.month, local_then.day)
}

impl NamedaySource {
    /// `on_names` gets the names of each reply for today;
    /// a failed or empty reply, or one for another day, sends nothing.
    /// `on_day_change` is sent once the day of the names last sent has passed.
    pub(super) fn new(on_names: Notify<NamedayData>, on_day_change: Notify<()>) -> Part<Self> {
        Part::new_cyclic(|me| Self {
            poll: PollHandle::register(
                me,
                Self::build_request,
                Self::on_nameday_data,
                PollConfig {
                    // Refetched by the day check, not on a cadence.
                    interval_ms: None,
                    debounce_ms: 0,
                    ..Default::default()
                },
            ),
            requested_for: None,
            served_for: None,
            _day_check: Timer::start(
                me,
                next_check_ms(SystemTime::now(), current_tz().as_ref()),
                Self::on_day_check,
            )
            .expect(
                "BUG: the first check is at least a second away, the host's shortest timer period, \
                 and without it the names never move on to the next day",
            ),
            on_names,
            on_day_change,
        })
    }

    /// Tell the widget and refetch once the day of the names last sent has passed,
    /// and ask again while no request could be built, for want of a timezone.
    pub(super) fn check_day(&mut self) {
        // Decided from what was sent, not asked for: a retry built after midnight
        // asks for today while the widget still shows yesterday's names.
        if self
            .served_for
            .as_ref()
            .is_some_and(|served| !is_today(served))
        {
            self.served_for = None;
            self.poll.invalidate();
            self.on_day_change.send(());
        } else if self.requested_for.is_none() {
            self.poll.invalidate();
        }
    }

    fn on_day_check(&mut self, timer: &Timer, _elapsed_periods: u32) {
        // Read before the day check: a midnight passing between the two clock reads
        // is then checked twice rather than skipped for a minute.
        let next_ms = next_check_ms(SystemTime::now(), current_tz().as_ref());
        self.check_day();
        if !timer.set_period(next_ms) {
            log_warn!(
                "nameday: the host refused a {} ms day check; it keeps its previous period",
                next_ms
            );
        }
    }

    fn build_request(&mut self, _handle: &PollHandle) -> Option<FetchSpec> {
        self.requested_for = None;
        let tz = current_tz()?;
        let time = SystemTime::now();
        let local_time = time.local(&tz)?;

        let request_url = NAMEDAY_API_URL_TEMPLATE
            .replace("{DAY_PLACEHOLDER}", &local_time.day.to_string())
            .replace("{MONTH_PLACEHOLDER}", &local_time.month.to_string());
        self.requested_for = Some(TimeWithTimezone { time, tz });

        Some(FetchSpec::get(request_url))
    }

    fn on_nameday_data(&mut self, handle: &PollHandle, response: &FetchResponse) {
        if response.ok() {
            match (
                self.requested_for.clone().filter(is_today),
                names_per_country(&response.json()),
            ) {
                (None, _) => {
                    // Data were fetched for incorrect date. This happens when NTP sync occurs
                    // after scheduling the fetch before the fetch successfully completes
                    // (it fails on TLS cert validation before time is synced).
                    handle.retry();
                }
                (Some(requested_for), Some(names_per_country)) => {
                    self.served_for = Some(requested_for.clone());
                    self.on_names.send(NamedayData {
                        names_per_country,
                        fetched_at: requested_for,
                    });
                }
                (Some(_), None) => {
                    // No country parsed at all => `/data` is absent or the schema
                    // changed; the response is genuinely unusable, so retry soon.
                    if let Some(response_text) = response.text() {
                        log_warn!(
                            "nameday: response had no usable /data, raw data: {}",
                            response_text
                        );
                    }

                    // If response is OK but parsing failed, poll engine will NOT reschedule
                    // new fetch automatically => we need to reschedule explicitly.
                    handle.retry();
                }
            }
        } else {
            // If response is not OK, poll engine will reschedule new fetch automatically.
            log_warn!("nameday: fetch failed (status {})", response.status);
        }
    }
}

/// The names a reply carries for every supported country, so switching country needs no new fetch,
/// or `None` when it names none, as when `/data` is absent or its schema changed.
/// A country the reply omits is `None` without failing the parse, so it never discards the others.
fn names_per_country(json: &JsonDoc) -> Option<[Option<String>; Country::ALL.len()]> {
    let mut per_country: [Option<String>; Country::ALL.len()] = Default::default();
    for country in Country::ALL {
        // Missing, empty and the API's "n/a" alike mean no nameday there today.
        per_country[*country as usize] = json
            .str(&fmt!("/data/{}", country.as_manifest_value()))
            .filter(|names| !names.is_empty() && names != "n/a");
    }
    per_country
        .iter()
        .any(Option::is_some)
        .then_some(per_country)
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::{JsonDoc, SystemTime, tz, wasm_test};

    use super::{TimeWithTimezone, is_same_local_date, names_per_country, next_check_ms};
    use crate::manifest_params::Country;

    /// 18 June 2026, 12:00 in Prague.
    const NOON: i64 = 1_781_776_800;
    /// 18 June 2026, 23:30 in Prague; 06:30 on the 19th in Tokyo.
    const HALF_PAST_ELEVEN: i64 = 1_781_818_200;
    /// 18 June 2026, 23:59 in Prague.
    const MINUTE_TO_MIDNIGHT: i64 = 1_781_819_940;
    /// 18 June 2026, 23:59:30 in Prague.
    const HALF_A_MINUTE_TO_MIDNIGHT: i64 = 1_781_819_970;
    /// 18 June 2026, 23:59:30 in UTC; 01:59:30 on the 19th in Prague.
    const HALF_A_MINUTE_TO_UTC_MIDNIGHT: i64 = 1_781_827_170;
    /// 19 June 2026, 00:01 in Prague; still 18 June in UTC.
    const MINUTE_PAST_MIDNIGHT: i64 = 1_781_820_060;
    /// 18 June 2027, 12:00 in Prague.
    const NOON_A_YEAR_ON: i64 = 1_813_312_800;

    fn asked_in_prague(unix_secs: i64) -> TimeWithTimezone {
        TimeWithTimezone {
            time: SystemTime { unix_secs },
            tz: tz!("Europe/Prague"),
        }
    }

    fn in_prague(then: i64, now: i64) -> bool {
        is_same_local_date(
            &asked_in_prague(then),
            SystemTime { unix_secs: now },
            Some(&tz!("Europe/Prague")),
        )
    }

    #[wasm_test]
    fn names_asked_earlier_that_day_stay_current() {
        assert!(
            in_prague(NOON, MINUTE_TO_MIDNIGHT),
            "noon's names serve until midnight; a miss would refetch them every minute"
        );
    }

    #[wasm_test]
    fn names_turn_stale_at_local_midnight() {
        assert!(
            !in_prague(MINUTE_TO_MIDNIGHT, MINUTE_PAST_MIDNIGHT),
            "two minutes later it is the next day, whose names have to be fetched"
        );
    }

    #[wasm_test]
    fn the_same_date_a_year_on_is_another_day() {
        assert!(
            !in_prague(NOON, NOON_A_YEAR_ON),
            "a year-old fetch is a past day's names, though the API asks only for day and month"
        );
    }

    #[wasm_test]
    fn names_asked_abroad_count_on_the_date_they_were_asked_for() {
        assert!(
            is_same_local_date(
                &TimeWithTimezone {
                    time: SystemTime {
                        unix_secs: HALF_PAST_ELEVEN
                    },
                    tz: tz!("Asia/Tokyo"),
                },
                SystemTime {
                    unix_secs: MINUTE_PAST_MIDNIGHT
                },
                Some(&tz!("Europe/Prague")),
            ),
            "asked on the 19th in Tokyo, the names serve Prague's 19th, though UTC is still on the 18th"
        );
    }

    #[wasm_test]
    fn a_device_without_a_timezone_reads_today_in_utc() {
        assert!(
            is_same_local_date(
                &asked_in_prague(MINUTE_TO_MIDNIGHT),
                SystemTime {
                    unix_secs: MINUTE_PAST_MIDNIGHT
                },
                None,
            ),
            "00:01 in Prague is still the 18th in UTC, the date the names were asked for"
        );
    }

    #[wasm_test]
    fn a_timezone_past_midnight_moves_today_on() {
        assert!(
            !is_same_local_date(
                &asked_in_prague(HALF_PAST_ELEVEN),
                SystemTime {
                    unix_secs: HALF_PAST_ELEVEN
                },
                Some(&tz!("Asia/Tokyo")),
            ),
            "moved to Tokyo, the device is already on the 19th, so Prague's 18th is past"
        );
    }

    #[wasm_test]
    fn the_check_before_midnight_lands_just_past_it() {
        assert_eq!(
            next_check_ms(
                SystemTime {
                    unix_secs: HALF_A_MINUTE_TO_MIDNIGHT
                },
                Some(&tz!("Europe/Prague")),
            ),
            31_000,
            "a full minute would show yesterday's names until 00:00:30"
        );
    }

    #[wasm_test]
    fn a_check_far_from_midnight_waits_a_minute() {
        assert_eq!(
            next_check_ms(SystemTime { unix_secs: NOON }, Some(&tz!("Europe/Prague"))),
            60_000,
            "a wait aimed hours ahead would miss midnight once NTP steps the clock"
        );
    }

    #[wasm_test]
    fn a_device_without_a_timezone_checks_past_utc_midnight() {
        assert_eq!(
            next_check_ms(
                SystemTime {
                    unix_secs: HALF_A_MINUTE_TO_UTC_MIDNIGHT
                },
                None,
            ),
            31_000,
            "the day check reads today in UTC then, so it aims at UTC's midnight"
        );
    }

    #[wasm_test]
    fn a_country_the_reply_omits_keeps_the_others() {
        let names = names_per_country(&JsonDoc::parse(br#"{"data":{"cz":"Milan"}}"#))
            .expect("one country named is a usable reply");

        assert_eq!(
            (
                names[Country::Cz as usize].as_deref(),
                names[Country::Sk as usize].as_deref()
            ),
            (Some("Milan"), None),
            "Slovakia missing from the reply must not discard the Czech names"
        );
    }

    #[wasm_test]
    fn the_api_sentinel_reads_as_no_nameday() {
        let names = names_per_country(&JsonDoc::parse(br#"{"data":{"cz":"Milan","sk":"n/a"}}"#))
            .expect("one country named is a usable reply");

        assert_eq!(
            names[Country::Sk as usize],
            None,
            "\"n/a\" would otherwise show as a name"
        );
    }

    #[wasm_test]
    fn an_empty_name_reads_as_no_nameday() {
        let names = names_per_country(&JsonDoc::parse(br#"{"data":{"cz":"Milan","sk":""}}"#))
            .expect("one country named is a usable reply");

        assert_eq!(
            names[Country::Sk as usize],
            None,
            "an empty name would otherwise show a blank line"
        );
    }

    #[wasm_test]
    fn a_reply_naming_no_country_is_unusable() {
        assert!(
            names_per_country(&JsonDoc::parse(br#"{"names":{"cz":"Milan"}}"#)).is_none(),
            "a changed schema has to retry soon rather than serve a day of blanks"
        );
    }
}

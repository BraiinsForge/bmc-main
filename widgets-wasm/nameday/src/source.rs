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

// Checked every minute rather than once at midnight:
// a wait computed before a clock jump, such as the first NTP sync, ends at the wrong time.
const DAY_CHECK_MS: u32 = 60 * 1000;

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

pub(super) struct NamedaySource {
    poll: PollHandle,
    /// What the last fetch asked for; an accepted reply copies it to [`NamedayData`].
    requested_for: Option<TimeWithTimezone>,
    /// The day of the names last sent, so the widget can be told once it has passed.
    served_for: Option<TimeWithTimezone>,
    _day_check: Option<Timer>,
    on_names: Notify<NamedayData>,
    on_day_change: Notify<()>,
}

/// Milliseconds from now until the next local midnight.
/// Note: The result is off by an hour on DST changes.
fn ms_to_next_local_midnight() -> u32 {
    const SECS_PER_DAY: i64 = 86_400;
    let now = SystemTime::now();
    let tz = system::current().timezone().map(Tz::from_runtime);
    let secs_since_midnight = tz.as_ref().and_then(|tz| now.local(tz)).map_or_else(
        || now.utc().seconds_since_midnight(),
        |l| l.seconds_since_midnight(),
    );
    let remaining = SECS_PER_DAY - i64::from(secs_since_midnight);
    u32::try_from(remaining.max(1) * 1_000)
        .expect("BUG: ms-to-midnight always fits in u32 (< 1 day)")
}

/// Whether localized date in timestamp falls on the current local calendar date. Used to tell
/// whether the cached data was downloaded today and is therefore still current.
pub(super) fn is_today(time_with_timezone: &TimeWithTimezone) -> bool {
    let tz = system::current().timezone().map(Tz::from_runtime);
    is_same_local_date(time_with_timezone, SystemTime::now(), tz.as_ref())
}

/// Whether `then`, read in its own timezone, falls on the date `now` reads in `now_tz`,
/// or in UTC when the device has no timezone.
fn is_same_local_date(then: &TimeWithTimezone, now: SystemTime, now_tz: Option<&Tz>) -> bool {
    let local_now = now_tz
        .and_then(|tz| now.local(tz))
        .unwrap_or_else(|| now.utc());
    let local_then = then.time.local(&then.tz).unwrap_or_else(|| then.time.utc());

    (local_now.month, local_now.day) == (local_then.month, local_then.day)
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
                    debounce_ms: 0,
                    ..Default::default()
                },
            ),
            requested_for: None,
            served_for: None,
            _day_check: Timer::start(me, DAY_CHECK_MS, Self::on_day_check),
            on_names,
            on_day_change,
        })
    }

    /// Refetch once the day the names were asked for has passed,
    /// and tell the widget once the day of the names last sent has.
    pub(super) fn check_day(&mut self) {
        if self
            .requested_for
            .as_ref()
            .is_some_and(|asked| !is_today(asked))
        {
            self.poll.invalidate();
        }
        // Decided from what was sent, not asked for: a retry built after midnight
        // asks for today while the widget still shows yesterday's names.
        if self
            .served_for
            .as_ref()
            .is_some_and(|served| !is_today(served))
        {
            self.served_for = None;
            self.on_day_change.send(());
        }
    }

    fn on_day_check(&mut self, _timer: &Timer, _elapsed_periods: u32) {
        self.check_day();
    }

    fn build_request(&mut self, _handle: &PollHandle) -> Option<FetchSpec> {
        let tz = system::current().timezone().map(Tz::from_runtime)?;
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
            // Parse the response once and pull out the names for every supported
            // country, so switching country later needs no new fetch. A country
            // the response omits is stored as `None` rather than failing the whole
            // parse, so one missing field never discards the others.
            let json = response.json();

            let mut names_per_country: [Option<String>; Country::ALL.len()] = Default::default();
            let mut data_found = false;

            for country in Country::ALL {
                let json_path_to_names = fmt!("/data/{}", country.as_manifest_value());

                // Treat missing, empty, and the API's "n/a" sentinel alike: no
                // nameday for this country today.
                match json.str(&json_path_to_names) {
                    Some(names) if !names.is_empty() && names != "n/a" => {
                        names_per_country[*country as usize] = Some(names);
                        data_found = true;
                    }
                    _ => {}
                }
            }

            match self.requested_for.clone().filter(is_today) {
                None => {
                    // Data were fetched for incorrect date. This happens when NTP sync occurs
                    // after scheduling the fetch before the fetch successfully completes
                    // (it fails on TLS cert validation before time is synced).
                    handle.retry();
                }
                Some(requested_for) if data_found => {
                    self.served_for = Some(requested_for.clone());
                    self.on_names.send(NamedayData {
                        names_per_country,
                        fetched_at: requested_for,
                    });

                    handle.retry_after(ms_to_next_local_midnight());
                }
                Some(_) => {
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

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::{SystemTime, tz, wasm_test};

    use super::{TimeWithTimezone, is_same_local_date};

    /// 18 June 2026, 12:00 in Prague.
    const NOON: i64 = 1_781_776_800;
    /// 18 June 2026, 23:30 in Prague; 06:30 on the 19th in Tokyo.
    const HALF_PAST_ELEVEN: i64 = 1_781_818_200;
    /// 18 June 2026, 23:59 in Prague.
    const MINUTE_TO_MIDNIGHT: i64 = 1_781_819_940;
    /// 19 June 2026, 00:01 in Prague; still 18 June in UTC.
    const MINUTE_PAST_MIDNIGHT: i64 = 1_781_820_060;

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
}

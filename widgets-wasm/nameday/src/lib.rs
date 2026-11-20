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

//! Nameday widget, four sizes.

#[cfg(target_arch = "wasm32")]
mod icons;
mod manifest_params;
#[cfg(target_arch = "wasm32")]
mod render;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {

    use super::manifest_params;
    use super::render;

    use bmc_wasm_sdk::timer::Timer;
    use bmc_wasm_sdk::*;

    use crate::manifest_params::Country;
    use crate::render::{country_draw, date_draw};

    const NAMEDAY_API_URL_TEMPLATE: &str =
        "https://nameday.abalin.net/api/V2/date?day={DAY_PLACEHOLDER}&month={MONTH_PLACEHOLDER}";

    // Checked every minute rather than once at midnight:
    // a wait computed before a clock jump, such as the first NTP sync, ends at the wrong time.
    const DAY_CHECK_MS: u32 = 60 * 1000;

    const LOADING_TEXT: &str = "Loading...";
    const NOT_APPLICABLE_TEXT: &str = "N/A";
    const NOT_AVAILABLE_TEXT: &str = "--";

    #[derive(Debug, Clone)]
    struct TimeWithTimezone {
        time: SystemTime,
        tz: Tz,
    }

    struct NamedayData {
        /// Names per supported country, parsed once per fetch. `None` for a
        /// country the response omitted or that has no nameday today; cached so a
        /// country change in the params only re-renders and never triggers a fetch.
        names_per_country: [Option<String>; Country::ALL.len()],
        fetched_at: TimeWithTimezone,
    }

    impl NamedayData {
        /// Names for `country`, or `None` if the response carried no usable entry.
        fn names_for(&self, country: Country) -> Option<&str> {
            self.names_per_country
                .get(country as usize)
                .and_then(|slot| slot.as_deref())
        }
    }

    struct Nameday {
        data: Option<NamedayData>,
        poll: PollHandle,
        /// What the last fetch asked for; an accepted reply copies it to [`NamedayData`].
        requested_for: Option<TimeWithTimezone>,
        _day_check: Option<Timer>,
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
    fn is_today(time_with_timezone: &TimeWithTimezone) -> bool {
        let now = SystemTime::now();
        let tz = system::current().timezone().map(Tz::from_runtime);
        let local_now = tz
            .as_ref()
            .and_then(|tz| now.local(tz))
            .unwrap_or_else(|| now.utc());

        let local_time = time_with_timezone
            .time
            .local(&time_with_timezone.tz)
            .unwrap_or_else(|| time_with_timezone.time.utc());

        local_now.month == local_time.month && local_now.day == local_time.day
    }

    impl Nameday {
        /// Refetch once the day the data were asked for has passed,
        /// and repaint while a past day's names show, so their age keeps counting.
        fn check_day(&mut self) {
            if self
                .requested_for
                .as_ref()
                .is_some_and(|asked| !is_today(asked))
            {
                self.poll.invalidate();
            }
            if self
                .data
                .as_ref()
                .is_some_and(|data| !is_today(&data.fetched_at))
            {
                request_frame();
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

                let mut names_per_country: [Option<String>; Country::ALL.len()] =
                    Default::default();
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
                        self.data = Some(NamedayData {
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

            request_frame();
        }
    }

    #[widget]
    impl Widget for Nameday {
        fn init(me: &Building<Self>) -> Self {
            let poll = PollHandle::register(
                me,
                Self::build_request,
                Self::on_nameday_data,
                PollConfig {
                    debounce_ms: 0,
                    ..Default::default()
                },
            );
            Self {
                data: None,
                poll,
                requested_for: None,
                _day_check: Timer::start(me, DAY_CHECK_MS, Self::on_day_check),
            }
        }

        fn render(&mut self, _delta_ms: u32) {
            let ws = widget_size();
            let params = manifest_params::Params::current();
            let mut root_children: Vec<Node> = vec![country_draw(ws, params.country)];
            let mut stale_anchor: Option<SystemTime> = None;

            let names_string: &str = match self.data.as_ref() {
                Some(data) => data
                    .names_for(params.country)
                    .unwrap_or(NOT_APPLICABLE_TEXT),
                None => LOADING_TEXT,
            };

            root_children.push(render::names_draw(ws, names_string));

            if params.show_date {
                match self.data.as_ref() {
                    Some(data) => {
                        let date_string = format_date(
                            data.fetched_at.time,
                            FormatDateOpts {
                                format: None,
                                timezone: Some(data.fetched_at.tz.clone()),
                            },
                        );

                        root_children.push(date_draw(ws, &date_string));

                        // Names for a past day are stale; flag with the shared pill.
                        if !is_today(&data.fetched_at) {
                            stale_anchor = Some(data.fetched_at.time);
                        }
                    }
                    None => {
                        root_children.push(date_draw(ws, NOT_AVAILABLE_TEXT));
                    }
                }
            }

            let mut root = col(props!(background: BLACK), root_children);
            if let Some(anchor) = stale_anchor {
                root = with_stale_overlay(root, anchor, widget_viewport().shape);
            }

            let _ = render_ui(ws.width, ws.height, root);
        }

        fn on_params_update(&mut self) {
            // Names for all supported countries are cached in the state, so a country
            // change (or any other param change) only needs a re-render, never a fetch.
            request_frame();
        }

        fn on_system_update(&mut self) {
            // A new timezone may move today to another date.
            self.check_day();
            request_frame();
        }
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Nameday widget, four sizes.

mod icons;
mod manifest_params;
mod render;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {

    use super::manifest_params;
    use super::render;
    use std::cell::RefCell;

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render code uses many SDK exports and macros in one file"
    )]
    use bmc_wasm_sdk::*;

    use crate::manifest_params::Country;
    use crate::render::{country_draw, date_draw};

    const NAMEDAY_API_URL_TEMPLATE: &str =
        "https://nameday.abalin.net/api/V2/date?day={DAY_PLACEHOLDER}&month={MONTH_PLACEHOLDER}";

    // As the data are checked for validity in render (as we do not have
    // a scheduler and fetch_after is unreliable due to RTC implementation),
    // we need to periodically call render.
    // This constant sets a delay for request_frame_after().
    const RERENDER_AFTER_MS: u32 = 60 * 1000;

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

    thread_local! {
        static NAMEDAY_DATA: RefCell<Option<NamedayData>> = const { RefCell::new(None) };
        static POLL_HANDLE: RefCell<Option<PollHandle>> = const { RefCell::new(None) };
        // Localized timestamp for which we requested the nameday data in the last fetch.
        // If the request comes OK, this is copied to NamedayData.
        static FETCHED_NAMEDAY_TIMESTAMP: RefCell<Option<TimeWithTimezone>> = const { RefCell::new(None) };
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        let tz = system::current().timezone().map(Tz::from_runtime)?;
        let time = SystemTime::now();
        let local_time = time.local(&tz)?;

        let request_url = NAMEDAY_API_URL_TEMPLATE
            .replace("{DAY_PLACEHOLDER}", &local_time.day.to_string())
            .replace("{MONTH_PLACEHOLDER}", &local_time.month.to_string());

        // Store the date for which the currently fetched namedays apply.
        FETCHED_NAMEDAY_TIMESTAMP.with_borrow_mut(|d| {
            *d = Some(TimeWithTimezone { time, tz });
        });

        Some(FetchSpec::get(request_url))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_nameday_data,
            PollConfig {
                debounce_ms: 0,
                ..Default::default()
            },
        );

        POLL_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
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

    fn on_nameday_data(handle: PollHandle, response: &FetchResponse) {
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

            if data_found {
                let fetched_at = FETCHED_NAMEDAY_TIMESTAMP
                    .with_borrow(std::clone::Clone::clone)
                    .expect("BUG: build_request did not populate FETCHED_NAMEDAY_TIMESTAMP.");

                NAMEDAY_DATA.with(|d| {
                    *d.borrow_mut() = Some(NamedayData {
                        names_per_country,
                        fetched_at,
                    });
                });

                handle.retry_after(ms_to_next_local_midnight());
            } else {
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
        } else {
            // If response is not OK, poll engine will reschedule new fetch automatically.
            log_warn!("nameday: fetch failed (status {})", response.status);
        }

        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let params = manifest_params::Params::current();
        let mut root_children: Vec<Node> = vec![country_draw(ws, params.country)];
        let mut stale_anchor: Option<SystemTime> = None;

        NAMEDAY_DATA.with(|d| {
            let data = d.borrow();

            let names_string: &str = match data.as_ref() {
                Some(data) => data
                    .names_for(params.country)
                    .unwrap_or(NOT_APPLICABLE_TEXT),
                None => LOADING_TEXT,
            };

            root_children.push(render::names_draw(ws, names_string));

            if params.show_date {
                match data.as_ref() {
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
        });

        let mut root = col(props!(background: BLACK), root_children);
        if let Some(anchor) = stale_anchor {
            root = with_stale_overlay(root, anchor, widget_viewport().shape);
        }

        let _ = render_ui(ws.width, ws.height, root);

        // If the data are outdated, schedule a new retrieval.
        // TODO: Reimplement using scheduler after it's ready.
        if let Some(timestamp) = FETCHED_NAMEDAY_TIMESTAMP.with_borrow(std::clone::Clone::clone)
            && !is_today(&timestamp)
        {
            POLL_HANDLE.with_borrow(|p| {
                if let Some(handle) = *p {
                    handle.invalidate();
                }
            });
        } else {
            request_frame_after(RERENDER_AFTER_MS);
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        // Names for all supported countries are cached in the state, so a country
        // change (or any other param change) only needs a re-render, never a fetch.
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        // As validity of the nameday data is verified in the render,
        // no logic needs to be here.
        // TODO: After scheduler is implemented, move the logic back here.
        request_frame();
    }
}

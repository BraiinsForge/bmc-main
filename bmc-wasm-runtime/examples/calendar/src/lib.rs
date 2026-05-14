// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(clippy::cast_precision_loss)]

//! iCal Agenda View Calendar Widget
//! Fetches .ics feeds, parses VEVENTs, expands RRULEs via host and renders an agenda view.

mod calendar;
mod ical_parser;
mod render;

use std::cell::RefCell;
use std::collections::HashMap;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

/// Install a panic hook that logs the panic message before aborting.
/// Without this, `panic = "abort"` just emits `unreachable` with no info.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        log_error!("PANIC: {}", msg.as_str());
    }));
}

use calendar::{CalendarSource, CalendarState};

/// Refresh interval for iCal feeds (15 minutes).
const REFRESH_INTERVAL_MS: u32 = 15 * 60 * 1_000;

/// How many days ahead to show (covers full month grid + next month padding).
const DAYS_AHEAD: u32 = 45;

/// Default calendar source definition (static data).
struct ICalSource {
    url: &'static str,
    label: &'static str,
    color: Color,
}

/// Default calendar sources for testing.
const DEFAULT_SOURCES: &[ICalSource] = &[
    ICalSource {
        url: "https://calendar.google.com/calendar/ical/en.czech%23holiday%40group.v.calendar.google.com/public/basic.ics",
        label: "Czech Holidays",
        color: BLUE_40,
    },
    ICalSource {
        url: "https://better-f1-calendar.vercel.app/api/calendar.ics",
        label: "Formula 1",
        color: RED_40,
    },
    ICalSource {
        url: "https://www.officeholidays.com/ics/finland",
        label: "Finland Holidays",
        color: BLUE_50,
    },
    ICalSource {
        url: "https://calendar.google.com/calendar/ical/en.usa%23holiday%40group.v.calendar.google.com/public/basic.ics",
        label: "US Holidays",
        color: GREEN_50,
    },
];

thread_local! {
    static STATE: RefCell<CalendarState> = RefCell::new(CalendarState::new());
    static THEME_KEY: RefCell<render::ThemeKey> = const { RefCell::new(render::ThemeKey::Dark) };
    /// Maps fetch request_id → source index. HTTP responses arrive in arbitrary
    /// order, so we cannot use a FIFO queue.
    static PENDING: RefCell<HashMap<bmc_wasm_sdk::FetchRequestId, usize>> = RefCell::new(HashMap::new());
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    install_panic_hook();
    THEME_KEY.with(|t| {
        *t.borrow_mut() = match kv::get_string("theme").as_deref() {
            Some("light") => render::ThemeKey::Light,
            Some("dark") => render::ThemeKey::Dark,
            _ => render::ThemeKey::Dark,
        };
    });

    STATE.with(|state| {
        let mut state = state.borrow_mut();

        // Try restoring saved calendar URLs from KV store
        state.load_sources_from_kv();

        // If no saved sources, use defaults
        if state.sources.is_empty() {
            for src in DEFAULT_SOURCES {
                state.sources.push(CalendarSource::new(
                    src.url.into(),
                    src.label.into(),
                    src.color,
                ));
            }
        }

        // Kick off initial fetch for all sources
        for (idx, source) in state.sources.iter().enumerate() {
            log_info!("fetching calendar {}: {}", idx, source.url);
            match FetchRequest::get(&source.url).send(on_ics_response) {
                Some(request_id) => {
                    PENDING.with(|p| p.borrow_mut().insert(request_id, idx));
                }
                None => {
                    log_error!("calendar fetch rejected for source {}: {}", idx, source.url);
                }
            }
        }
    });

    // Schedule periodic refresh
    request_frame_after(REFRESH_INTERVAL_MS);
}

fn on_ics_response(response: &FetchResponse) {
    let source_idx = PENDING.with(|p| p.borrow_mut().remove(&response.request_id));

    let Some(idx) = source_idx else {
        log_error!("received fetch response with unknown request_id");
        return;
    };

    if !response.ok() {
        log_error!(
            "fetch failed for calendar {}: status {}",
            idx,
            response.status
        );
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(src) = s.sources.get_mut(idx) {
                src.error = Some("fetch failed".to_string());
                src.loading = false;
            }
        });
        request_frame();
        return;
    }

    let body = response.text().unwrap_or("");

    // Split into per-VEVENT chunks — cheap byte scan, no allocations per line.
    // Actual parsing is deferred to render() in batches to stay within fuel.
    let chunks = ical_parser::split_into_chunks(body);
    log_info!("split {} VEVENT chunks from calendar {}", chunks.len(), idx);

    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(source) = s.sources.get_mut(idx) {
            source.raw_events.clear();
            source.loading = false;
            source.error = None;
        }
        // Purge any stale chunks from a previous fetch of the same source
        s.parse_queue.retain(|(si, _)| *si != idx);
        for chunk in chunks {
            s.parse_queue.push_back((idx, chunk));
        }
    });

    request_frame();
}

fn retry_failed_sources() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        for (idx, source) in state.sources.iter_mut().enumerate() {
            if source.error.is_some() {
                source.error = None;
                source.loading = true;
                log_info!("retrying calendar {}: {}", idx, source.url);
                match FetchRequest::get(&source.url).send(on_ics_response) {
                    Some(request_id) => {
                        PENDING.with(|p| p.borrow_mut().insert(request_id, idx));
                    }
                    None => {
                        log_error!("calendar retry rejected for source {}: {}", idx, source.url);
                        source.error = Some("fetch queue full".into());
                        source.loading = false;
                    }
                }
            }
        }
        state.any_loading = true;
    });
    request_frame();
}

fn toggle_theme() {
    THEME_KEY.with(|t| {
        let mut current = t.borrow_mut();
        *current = match *current {
            render::ThemeKey::Light => render::ThemeKey::Dark,
            render::ThemeKey::Dark => render::ThemeKey::Light,
        };
        let value = match *current {
            render::ThemeKey::Light => "light",
            render::ThemeKey::Dark => "dark",
        };
        kv::set("theme", value.as_bytes());
    });
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = widget_size();

    let theme_key = THEME_KEY.with(|t| *t.borrow());
    render::set_theme_key(theme_key);

    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.update_time();

        // Parse the next batch of VEVENT chunks (deferred from fetch callback
        // to stay within fuel budget — large .ics files can have hundreds of events).
        state.drain_parse_queue();

        // Rebuild expanded events if sources changed
        if state.dirty {
            state.dirty = false;
            state.rebuild_events();
        }

        let tree = render::render_agenda(&state, size);
        let result = render_ui(size.width, size.height, tree);

        if result.clicks.contains_key("theme_toggle") {
            toggle_theme();
        }
        if result.clicks.contains_key("retry") {
            retry_failed_sources();
        }

        // If chunks remain, request immediate next frame to continue draining.
        // Otherwise, slow-tick for clock updates.
        if state.has_pending_chunks() {
            request_frame();
        } else {
            request_frame_after(60_000);
        }
    });
}

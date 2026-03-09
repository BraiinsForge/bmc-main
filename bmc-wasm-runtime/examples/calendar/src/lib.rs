// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(clippy::cast_precision_loss)]

//! iCal Agenda View Calendar Widget
//! Fetches .ics feeds, parses VEVENTs, expands RRULEs via host and renders an agenda view.

mod calendar;
mod ical_parser;
mod render;

use std::cell::RefCell;
use std::collections::HashMap;

#[allow(clippy::wildcard_imports)]
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
    color: u32,
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
    static SIZE: RefCell<WidgetSize> = RefCell::new(WidgetSize::from_dimensions(1_280, 480));
    /// Maps fetch request_id → source index. HTTP responses arrive in arbitrary
    /// order, so we cannot use a FIFO queue.
    static PENDING: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
}

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    install_panic_hook();
    SIZE.with(|s| *s.borrow_mut() = WidgetSize::from_dimensions(width, height));

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
            let request_id = FetchRequest::get(&source.url).send(on_ics_response);
            PENDING.with(|p| p.borrow_mut().insert(request_id, idx));
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

    // Parse OUTSIDE the STATE borrow — ical parsing is expensive and could
    // exhaust WASM fuel. If fuel runs out while STATE is borrowed, the RefCell
    // is permanently locked and all subsequent calls panic.
    let raw_events = ical_parser::parse_ics(body);
    log_info!("parsed {} events from calendar {}", raw_events.len(), idx);

    // Store parsed events and mark dirty. Don't rebuild here — RRULE expansion
    // is expensive and would exhaust the fetch callback's fuel budget. The render
    // function gets its own fuel budget and will rebuild when it sees the dirty flag.
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(source) = s.sources.get_mut(idx) {
            source.raw_events = raw_events;
            source.loading = false;
            source.error = None;
        }
        s.dirty = true;
    });

    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = SIZE.with(|s| *s.borrow());

    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.update_time();

        // Rebuild expanded events if sources changed (deferred from fetch callback
        // to avoid exhausting the callback's fuel budget)
        if state.dirty {
            state.dirty = false;
            state.rebuild_events();
        }

        let tree = render::render_agenda(&state, size);
        let result = render_ui(size.width, size.height, tree);

        // Handle button clicks
        for (i, &clicked) in result.clicks.iter().enumerate() {
            if clicked {
                state.on_click(i);
            }
        }
    });

    // Keep refreshing for clock updates (every 60s is fine for agenda)
    request_frame_after(60_000);
}

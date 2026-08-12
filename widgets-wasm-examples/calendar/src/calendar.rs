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

//! Calendar data model — sources, expanded events, day grouping.

use std::collections::VecDeque;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use crate::DAYS_AHEAD;
use crate::ical_parser::RawEvent;

/// Project `now` into the system tz's local fields, falling back to UTC
/// when the snapshot has no timezone or the host can't resolve it.
fn system_tz_local(now: &SystemTime) -> LocalDateTime {
    system::current()
        .timezone()
        .and_then(|name| now.local(&Tz::from_runtime(name)))
        .unwrap_or_else(|| now.utc())
}

/// A single calendar event after RRULE expansion.
#[derive(Debug, Clone)]
pub struct CalendarEvent {
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    /// UTC unix timestamp of event start.
    pub start: i64,
    /// UTC unix timestamp of event end.
    pub end: i64,
    /// Whether this is an all-day event.
    pub all_day: bool,
    /// Index into `CalendarState::sources` (for color lookup).
    pub source_idx: usize,
}

/// One calendar feed.
#[derive(Debug, Clone)]
pub struct CalendarSource {
    pub url: String,
    pub label: String,
    /// Display color — clamped to ensure white text readability.
    pub color: Color,
    pub loading: bool,
    pub error: Option<String>,
    pub raw_events: Vec<RawEvent>,
}

impl CalendarSource {
    pub fn new(url: String, label: String, color: Color) -> Self {
        Self {
            url,
            label,
            color: darken_for_text(color),
            loading: true,
            error: None,
            raw_events: Vec::new(),
        }
    }
}

/// Clamp a color's lightness so white text is always readable on it.
/// Uses BT.601 luminance to detect bright colors, then OkLCH to darken them.
fn darken_for_text(color: Color) -> Color {
    let lum = u32::from(color.red()) * 299
        + u32::from(color.green()) * 587
        + u32::from(color.blue()) * 114;
    if lum > 120_000 {
        color.lightness(0.45)
    } else {
        color
    }
}

/// A group of events for a single day.
#[derive(Debug)]
pub struct DayGroup {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub is_today: bool,
    pub all_day: Vec<usize>,
    pub timed: Vec<usize>,
}

/// Maximum VEVENT chunks to parse per render frame.
const PARSE_BATCH_SIZE: usize = 20;

/// Top-level calendar state.
#[derive(Debug)]
pub struct CalendarState {
    pub sources: Vec<CalendarSource>,
    /// All expanded events, sorted by start time.
    pub events: Vec<CalendarEvent>,
    /// Events grouped by day.
    pub day_groups: Vec<DayGroup>,
    /// Current UTC instant (updated each frame).
    pub now: SystemTime,
    /// `now` projected into the system timezone, refreshed alongside `now`.
    pub local: LocalDateTime,
    /// Whether any source is still loading.
    pub any_loading: bool,
    /// Whether raw events have changed and need rebuilding.
    pub dirty: bool,
    /// Queue of unparsed VEVENT chunks: (source_idx, chunk_text).
    pub parse_queue: VecDeque<(usize, String)>,
    /// First day of the week: 0=Monday, 6=Sunday.
    pub first_day_of_week: u8,
    /// Whether to use 24-hour time format.
    pub use_24h: bool,
}

impl CalendarState {
    pub fn new() -> Self {
        let now = SystemTime::now();
        Self {
            sources: Vec::new(),
            events: Vec::new(),
            day_groups: Vec::new(),
            local: system_tz_local(&now),
            now,
            any_loading: true,
            dirty: false,
            parse_queue: VecDeque::new(),
            first_day_of_week: 0,
            use_24h: true,
        }
    }

    /// Update the current time and its system-tz projection.
    pub fn update_time(&mut self) {
        self.now = SystemTime::now();
        self.local = system_tz_local(&self.now);
    }

    /// Returns `true` if there are unparsed chunks remaining.
    pub fn has_pending_chunks(&self) -> bool {
        !self.parse_queue.is_empty()
    }

    /// Parse up to [`PARSE_BATCH_SIZE`] chunks from the queue into `raw_events`.
    /// Returns `true` if any events were parsed (caller should mark dirty).
    pub fn drain_parse_queue(&mut self) -> bool {
        if self.parse_queue.is_empty() {
            return false;
        }

        let batch = self.parse_queue.len().min(PARSE_BATCH_SIZE);
        let mut parsed_any = false;

        for _ in 0..batch {
            let Some((source_idx, chunk)) = self.parse_queue.pop_front() else {
                break;
            };
            if let Some(event) = crate::ical_parser::parse_chunk(&chunk) {
                if let Some(source) = self.sources.get_mut(source_idx) {
                    source.raw_events.push(event);
                    parsed_any = true;
                }
            }
        }

        if parsed_any {
            self.dirty = true;
        }

        parsed_any
    }

    /// Expand all raw events (including RRULEs) and rebuild the sorted event list + day groups.
    pub fn rebuild_events(&mut self) {
        let now = &self.now;
        let window_start = now.unix_secs;
        let window_end = window_start + i64::from(DAYS_AHEAD) * 86_400;

        let mut events = Vec::new();

        for (source_idx, source) in self.sources.iter().enumerate() {
            for raw in &source.raw_events {
                let expanded = expand_raw_event(raw, source_idx, window_start, window_end);
                events.extend(expanded);
            }
        }

        // Inject test multi-day events for development (never compiled into release)
        #[cfg(debug_assertions)]
        inject_test_events(&mut events, &self.now, &self.local);

        // Sort by start time, all-day events first within each day
        events.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then(b.all_day.cmp(&a.all_day))
                .then(a.summary.cmp(&b.summary))
        });

        self.events = events;
        self.rebuild_day_groups();

        self.any_loading = self.sources.iter().any(|s| s.loading);
    }

    /// Group events by day using the device's local timezone.
    fn rebuild_day_groups(&mut self) {
        self.day_groups.clear();

        let today_year = self.local.year;
        let today_month = self.local.month;
        let today_day = self.local.day;

        for (event_idx, event) in self.events.iter().enumerate() {
            // Convert event start to local time for grouping
            let local = calendar::tz_convert(event.start, "Local").unwrap_or(self.local);

            let needs_new_group = self.day_groups.last().is_none_or(|g| {
                g.year != local.year || g.month != local.month || g.day != local.day
            });

            if needs_new_group {
                self.day_groups.push(DayGroup {
                    year: local.year,
                    month: local.month,
                    day: local.day,
                    weekday: local.weekday,
                    is_today: local.year == today_year
                        && local.month == today_month
                        && local.day == today_day,
                    all_day: Vec::new(),
                    timed: Vec::new(),
                });
            }

            let group = self
                .day_groups
                .last_mut()
                .expect("BUG: just pushed a group");
            if event.all_day {
                group.all_day.push(event_idx);
            } else {
                group.timed.push(event_idx);
            }
        }
    }

    /// Load source URLs from KV store.
    pub fn load_sources_from_kv(&mut self) {
        if let Some(data) = kv::get_string("calendar_sources") {
            // Simple line-based format: url|label|color per line
            for line in data.lines() {
                let parts: Vec<&str> = line.splitn(3, '|').collect();
                if parts.len() >= 2 {
                    let url = parts[0].to_string();
                    let label = parts[1].to_string();
                    let color = parts
                        .get(2)
                        .and_then(|s| u32::from_str_radix(s, 16).ok())
                        .map_or(Color::from_hex(0x42_8B_CA), Color::from_raw);
                    self.sources.push(CalendarSource::new(url, label, color));
                }
            }
            log_info!("restored {} calendar sources from KV", self.sources.len());
        }

        // Load preferences
        if let Some(fdow) = kv::get_string("first_day_of_week") {
            self.first_day_of_week = fdow.parse().unwrap_or(0) % 7;
        }
        if let Some(tf) = kv::get_string("time_format") {
            self.use_24h = tf != "12h";
        }
    }
}

/// Expand a single raw event into concrete occurrences within the window.
fn expand_raw_event(
    raw: &RawEvent,
    source_idx: usize,
    window_start: i64,
    window_end: i64,
) -> Vec<CalendarEvent> {
    let mut result = Vec::new();

    // Parse DTSTART into a unix timestamp
    let Some(start_ts) = parse_ical_datetime(&raw.dtstart) else {
        return result;
    };

    // Calculate duration
    let duration_secs = if let Some(dtend) = &raw.dtend {
        parse_ical_datetime(dtend).map_or(if raw.all_day { 86_400 } else { 3_600 }, |end| {
            end - start_ts
        })
    } else if raw.all_day {
        86_400
    } else {
        3_600 // Default 1 hour for timed events without DTEND
    };

    if let Some(rrule) = &raw.rrule {
        // Recurring event — delegate to host
        let occurrences = calendar::expand_rrule(
            rrule,
            &raw.dtstart,
            raw.tzid.as_deref(),
            window_start,
            window_end,
            200_u16,
        );
        for occ_start in occurrences {
            result.push(CalendarEvent {
                summary: raw.summary.clone(),
                description: raw.description.clone(),
                location: raw.location.clone(),
                start: occ_start,
                end: occ_start + duration_secs,
                all_day: raw.all_day,
                source_idx,
            });
        }
    } else {
        // Single event — check if it falls in the window
        let end_ts = start_ts + duration_secs;
        if end_ts >= window_start && start_ts < window_end {
            result.push(CalendarEvent {
                summary: raw.summary.clone(),
                description: raw.description.clone(),
                location: raw.location.clone(),
                start: start_ts,
                end: end_ts,
                all_day: raw.all_day,
                source_idx,
            });
        }
    }

    result
}

/// Parse an iCal datetime string into a unix timestamp.
///
/// Supports formats: `YYYYMMDD`, `YYYYMMDDTHHmmss`, `YYYYMMDDTHHmmssZ`
fn parse_ical_datetime(s: &str) -> Option<i64> {
    let iso = ical_to_iso(s);
    parse_datetime(&iso)
}

/// Convert iCal datetime format to ISO 8601 for host parsing.
///
/// `20260310T090000` → `2026-03-10T09:00:00`
/// `20260310` → `2026-03-10T00:00:00`
/// `20260310T090000Z` → `2026-03-10T09:00:00Z`
fn ical_to_iso(s: &str) -> String {
    let s = s.trim();
    if s.len() < 8 {
        return s.to_string();
    }

    let year = &s[0..4];
    let month = &s[4..6];
    let day = &s[6..8];

    if s.len() == 8 {
        return fmt!("{year}-{month}-{day}T00:00:00Z");
    }

    if s.len() >= 15 && s.as_bytes()[8] == b'T' {
        let hour = &s[9..11];
        let min = &s[11..13];
        let sec = &s[13..15];
        let suffix = if s.ends_with('Z') { "Z" } else { "" };
        return fmt!("{year}-{month}-{day}T{hour}:{min}:{sec}{suffix}");
    }

    s.to_string()
}

// ── Month Grid ──────────────────────────────────────────────────────

/// A month calendar grid ready for rendering.
pub struct MonthGrid {
    pub year: u16,
    pub month: u8,
    pub weeks: Vec<WeekRow>,
}

/// One week in the month grid.
pub struct WeekRow {
    pub cells: [GridCell; 7],
    /// Multi-day / all-day event bars spanning columns within this week.
    pub spans: Vec<EventSpan>,
}

/// A single day cell in the grid.
#[derive(Clone, Default)]
pub struct GridCell {
    pub day: u8,
    pub month: u8,
    pub year: u16,
    pub is_current_month: bool,
    pub is_today: bool,
    pub is_weekend: bool,
    /// Indices into `CalendarState::events` for timed events on this date.
    pub events: Vec<usize>,
}

/// A multi-day event bar spanning columns within a week.
pub struct EventSpan {
    pub event_idx: usize,
    /// First column (0–6) in this week.
    pub start_col: u8,
    /// Last column (0–6, inclusive) in this week.
    pub end_col: u8,
    /// Vertical lane within the week's span area.
    pub lane: u8,
}

impl CalendarState {
    /// Build a month calendar grid for the current month.
    pub fn build_month_grid(&self) -> MonthGrid {
        let now = &self.local;
        let fdow = self.first_day_of_week;

        // Weekday of the 1st of current month (0=Mon..6=Sun)
        let wd1 = (i32::from(now.weekday) - (i32::from(now.day) - 1) + 700) % 7;
        // Column offset: which column does the 1st fall on?
        let offset = (wd1 - i32::from(fdow) + 7) % 7;
        let dim = days_in_month(now.year, now.month);
        let (prev_y, prev_m) = prev_month(now.year, now.month);
        let prev_dim = days_in_month(prev_y, prev_m);
        let (next_y, next_m) = next_month(now.year, now.month);
        let num_weeks = to_usize((offset + i32::from(dim) + 6) / 7);

        let mut weeks = Vec::with_capacity(num_weeks);
        let mut day_counter: i32 = 1 - offset;

        for _ in 0..num_weeks {
            let cells: [GridCell; 7] = std::array::from_fn(|ci| {
                let weekday = (usize::from(fdow) + ci) % 7;
                let is_weekend = weekday >= 5; // 5=Sat, 6=Sun
                let (d, m, y, is_cur) = if day_counter < 1 {
                    (
                        to_u8(i32::from(prev_dim) + day_counter),
                        prev_m,
                        prev_y,
                        false,
                    )
                } else if day_counter > i32::from(dim) {
                    (to_u8(day_counter - i32::from(dim)), next_m, next_y, false)
                } else {
                    (to_u8(day_counter), now.month, now.year, true)
                };
                let cell = GridCell {
                    day: d,
                    month: m,
                    year: y,
                    is_current_month: is_cur,
                    is_today: y == now.year && m == now.month && d == now.day,
                    is_weekend,
                    events: Vec::new(),
                };
                day_counter += 1;
                cell
            });
            weeks.push(WeekRow {
                cells,
                spans: Vec::new(),
            });
        }

        // Assign events to cells / spans
        for (ei, event) in self.events.iter().enumerate() {
            let start_local = calendar::tz_convert(event.start, "Local").unwrap_or(self.local);
            let sd = (start_local.year, start_local.month, start_local.day);

            if event.end - event.start > 86_400 {
                // Multi-day → span bars (single-day all-day events go to cell instead)
                // All-day end dates are exclusive in iCal — DTEND is the day AFTER the last day.
                // Subtract a full day (not 1 second) so tz_convert doesn't shift to the next day.
                let end_ts = if event.all_day {
                    event.end - 86_400
                } else {
                    event.end
                };
                let end_local =
                    calendar::tz_convert(end_ts.max(event.start), "Local").unwrap_or(self.local);
                let ed = (end_local.year, end_local.month, end_local.day);

                for week in &mut weeks {
                    let mut sc: Option<u8> = None;
                    let mut ec: Option<u8> = None;
                    for (ci, cell) in week.cells.iter().enumerate() {
                        let cd = (cell.year, cell.month, cell.day);
                        if cd >= sd && cd <= ed {
                            let col = to_u8(ci);
                            if sc.is_none() {
                                sc = Some(col);
                            }
                            ec = Some(col);
                        }
                    }
                    if let (Some(s), Some(e)) = (sc, ec) {
                        week.spans.push(EventSpan {
                            event_idx: ei,
                            start_col: s,
                            end_col: e,
                            lane: 0,
                        });
                    }
                }
            } else {
                // Timed single-day event → find its cell
                for week in &mut weeks {
                    for cell in &mut week.cells {
                        if cell.year == sd.0 && cell.month == sd.1 && cell.day == sd.2 {
                            cell.events.push(ei);
                        }
                    }
                }
            }
        }

        // Assign lanes to spans within each week
        for week in &mut weeks {
            assign_span_lanes(&mut week.spans);
        }

        MonthGrid {
            year: now.year,
            month: now.month,
            weeks,
        }
    }
}

/// Assign vertical lanes to multi-day spans within a week using greedy placement.
fn assign_span_lanes(spans: &mut [EventSpan]) {
    // Sort by start column, then longer spans first for stable lane assignment
    spans.sort_by(|a, b| {
        a.start_col
            .cmp(&b.start_col)
            .then((b.end_col - b.start_col).cmp(&(a.end_col - a.start_col)))
    });
    // Track the last occupied column per lane (all values 0–6)
    let mut lane_ends: Vec<u8> = Vec::new();
    for span in spans.iter_mut() {
        let mut assigned = false;
        for (lane, end) in lane_ends.iter_mut().enumerate() {
            if span.start_col > *end {
                span.lane = to_u8(lane);
                *end = span.end_col;
                assigned = true;
                break;
            }
        }
        if !assigned {
            span.lane = to_u8(lane_ends.len());
            lane_ends.push(span.end_col);
        }
    }
}

/// Convert any integer to `u8`, panicking if out of range.
fn to_u8(v: impl TryInto<u8, Error: core::fmt::Debug>) -> u8 {
    v.try_into().expect("BUG: value does not fit in u8")
}

/// Convert any integer to `usize`, panicking if out of range.
fn to_usize(v: impl TryInto<usize, Error: core::fmt::Debug>) -> usize {
    v.try_into().expect("BUG: value does not fit in usize")
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn prev_month(year: u16, month: u8) -> (u16, u8) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

fn next_month(year: u16, month: u8) -> (u16, u8) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

/// Inject synthetic multi-day events for testing grid rendering.
/// Only compiled when the `test-events` feature is active.
#[cfg(debug_assertions)]
fn inject_test_events(events: &mut Vec<CalendarEvent>, now: &SystemTime, local: &LocalDateTime) {
    let today_midnight = now.unix_secs
        - i64::from(local.hour) * 3_600
        - i64::from(local.minute) * 60
        - i64::from(local.second);

    // 3-day event starting today (spans within a week)
    events.push(CalendarEvent {
        summary: "[TEST] Lorem Ipsum Dolor".into(),
        description: Some("Vestibulum ante ipsum primis in faucibus orci luctus.".into()),
        location: Some("Sit Amet".into()),
        start: today_midnight,
        end: today_midnight + 3 * 86_400,
        all_day: true,
        source_idx: 0,
    });

    // 5-day event starting in 4 days (likely crosses a week boundary)
    events.push(CalendarEvent {
        summary: "[TEST] Consectetur Adipiscing".into(),
        description: Some("Nulla facilisi. Cras accumsan elit vel metus bibendum.".into()),
        location: None,
        start: today_midnight + 4 * 86_400,
        end: today_midnight + 9 * 86_400,
        all_day: true,
        source_idx: 1,
    });

    // 2-day event next week
    events.push(CalendarEvent {
        summary: "[TEST] Sed Do Eiusmod".into(),
        description: None,
        location: Some("Tempor".into()),
        start: today_midnight + 10 * 86_400,
        end: today_midnight + 12 * 86_400,
        all_day: true,
        source_idx: 2,
    });

    // Single all-day event in 2 days
    events.push(CalendarEvent {
        summary: "[TEST] Ut Labore".into(),
        description: Some("Duis aute irure dolor in reprehenderit in voluptate velit.".into()),
        location: None,
        start: today_midnight + 2 * 86_400,
        end: today_midnight + 3 * 86_400,
        all_day: true,
        source_idx: 3,
    });

    // Timed events — exercise 24h/12h format rendering
    events.push(CalendarEvent {
        summary: "[TEST] Evening Run".into(),
        description: None,
        location: Some("Stromovka".into()),
        start: today_midnight + 18 * 3_600 + 45 * 60, // 18:45
        end: today_midnight + 19 * 3_600 + 30 * 60,   // 19:30
        all_day: false,
        source_idx: 3,
    });
    events.push(CalendarEvent {
        summary: "[TEST] Architecture Review".into(),
        description: Some("Review the new widget rendering pipeline.".into()),
        location: None,
        start: today_midnight + 86_400 + 14 * 3_600 + 15 * 60, // tomorrow 14:15
        end: today_midnight + 86_400 + 15 * 3_600 + 30 * 60,   // tomorrow 15:30
        all_day: false,
        source_idx: 2,
    });
}

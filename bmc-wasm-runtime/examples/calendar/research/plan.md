# BDK-340: iCal Agenda View Calendar Widget POC

## Context

Robert suggested a Google Calendar widget for BF Deck. Team agreed on **iCal-based approach** — avoids OAuth/auth
complexity, works with any calendar provider (Google, Outlook, Facebook, sync2cal.com, etc.). The ticket was written in
the era of server-rendered image widgets, but this POC targets the new WASM runtime stack. Beyond a basic agenda list,
we should be ambitious and exercise the SDK fully — color-coded calendars, proper layout, touch interaction.

Reference screenshots: dumbed-down agenda views from Jira (BDK-340 attachments). The designer can push further since
we're no longer constrained by server-side image generation.

---

## Architecture

```
Widget (WASM, ~52K + ical crate)
├── Data
│   ├── fetch .ics feeds via FetchRequest::get() (host HTTP)
│   ├── parse with `ical` crate (structural parser, WASM-safe)
│   ├── extract VEVENTs: DTSTART, DTEND, SUMMARY, DESCRIPTION, LOCATION, RRULE, EXDATE, TZID
│   └── call host_expand_rrule() for recurring events → concrete occurrence timestamps
├── State
│   ├── CalendarSource[] — URL, label, color, parsed events, refresh timer
│   ├── ExpandedEvent[] — flattened occurrences sorted chronologically
│   └── view window (start_date, days_ahead)
└── UI (tree API + canvas + scroll)
    ├── Day headers (bold date, today highlighted)
    ├── All-day events section per day
    ├── Timed events with time + title + color bar
    ├── Scrollable event list
    └── Size-adaptive layout (Full → Small)

Host (new primitives, zero new deps)
├── host_expand_rrule — rrule crate + chrono-tz (both already workspace deps)
│   Input:  RRULE string, DTSTART, TZID, window_start, window_end
│   Output: array of occurrence timestamps (UTC i64[])
├── host_tz_convert — convert UTC timestamp to wall-clock in a named timezone
│   Input:  unix_secs, tz_name (IANA)
│   Output: SystemTime-like struct (year/month/day/hour/min/sec/weekday/utc_offset)
└── [existing] host_get_system_time, host_format_date, host_parse_date, fetch, kv
```

### Why this split

- **RRULE on host**: the `rrule` crate hard-depends on `chrono-tz` (full IANA DB, ~1.8M). Both `chrono` and `chrono-tz`
  are already workspace dependencies — zero marginal cost on the host. Putting it in WASM would 35x the binary size.
- **iCal parsing in WASM**: the `ical` crate is 52K, structural-only, zero heavy deps. Calendar-specific domain logic
  stays in the widget.
- **TZ conversion on host**: universal concern, reusable by any future widget needing timezone-aware display. Currently
  `host_format_date` uses `Local::now()` offset only — a named-timezone conversion primitive is a natural extension.

### Data flow

```
1. Widget init → load saved iCal URLs from KV store
2. For each URL: fetch .ics → parse with ical → extract VEVENTs
3. For recurring events: call host_expand_rrule(rrule, dtstart, tzid, window)
4. Merge all occurrences, sort by start time, group by day
5. Render agenda view
6. Re-fetch on timer (configurable interval, default 15 min)
```

---

## Host API Design

### `host_expand_rrule`

```
SDK declaration:
  host_expand_rrule(
    input_ptr: *const u8, input_len: u32,   // JSON: {rrule, dtstart, tzid, window_start, window_end, max_count}
    out_ptr: *mut u8, out_cap: u32          // output buffer
  ) -> i32                                  // bytes written, or required size if out_cap=0 (two-call pattern)

Input JSON:
  {
    "rrule": "FREQ=WEEKLY;BYDAY=MO,WE,FR;UNTIL=20260601T000000Z",
    "dtstart": "20260310T090000",
    "tzid": "Europe/Prague",
    "window_start": 1741564800,  // unix timestamp
    "window_end": 1742774400,
    "max_count": 200             // safety cap
  }

Output: packed i64[] little-endian (UTC timestamps of each occurrence)
```

Uses two-call pattern (like `host_kv_get`): first call with `out_cap=0` returns required size, second call fills buffer.

### `host_tz_convert`

```
SDK declaration:
  host_tz_convert(
    unix_secs: i64,
    tz_ptr: *const u8, tz_len: u32,        // IANA timezone name
    out_ptr: *mut u8                        // 20-byte SystemTime struct (same format as host_get_system_time)
  ) -> i32                                  // 0 = ok, -1 = unknown timezone

Output: same 20-byte struct as SystemTime (year/month/day/hour/min/sec/weekday/utc_offset)
```

This lets WASM convert any UTC timestamp to wall-clock time in any timezone — needed because iCal events may reference
timezones different from the device's local timezone.

---

## Size Variants & Interaction

### Full (1280×480)

**Layout**: two-column — left column: mini month calendar or "today" summary card; right column: scrollable agenda list
**Content**: 7-14 days of events visible, day headers, all-day section, timed events with time + title + location
**Interaction**: vertical scroll through days/events, tap event for detail expansion (inline or modal) **Color**:
per-calendar color bar on left edge of each event row

### Large (638×480)

**Layout**: single-column scrollable agenda list (full height available) **Content**: 7 days visible, day headers,
all-day + timed events, time + title (no location unless space allows) **Interaction**: vertical scroll, tap event for
detail expansion **Color**: per-calendar color dot or thin bar

### Medium (638×238)

**Layout**: single-column, compact rows **Content**: 3-5 days visible (scrollable), day headers, time + title only
**Interaction**: vertical scroll **Color**: per-calendar color dot

### Small (317×238)

**Layout**: single-column, today focus **Content**: today's events only (or today + tomorrow if few events), time +
title, truncated with ellipsis **Interaction**: vertical scroll if events overflow **Color**: per-calendar color dot

### Common elements across all sizes

- **Today header** highlighted (different background or accent color)
- **All-day events** grouped at top of each day, labeled "All-Day" or badge
- **Current time indicator** — subtle line/marker showing "now" position among today's events
- **Empty state** — friendly message when no events in window
- **Loading state** — shown during initial fetch
- **Error state** — shown when fetch fails (with retry)

---

## Implementation Stages

### Stage 0: Project scaffolding + iCal parsing proof

**Goal**: new `calendar` example that fetches a hardcoded iCal URL and logs parsed events to console.

**Files**:

- `examples/calendar/Cargo.toml` — depends on `bmc-wasm-sdk`, `ical`
- `examples/calendar/src/lib.rs` — minimal widget: fetch URL on init, parse with `ical`, log events
- `examples/calendar/research/plan.md` — this plan (copy)

**Success**: `make run EXAMPLE=calendar` shows parsed VEVENT summaries + dates in testbed log.

### Stage 1: Host RRULE expansion primitive

**Goal**: add `host_expand_rrule` and `host_tz_convert` to the SDK and runtime.

**Files**:

- `sdk/src/host.rs` — extern declarations + safe wrappers
- `sdk/src/calendar.rs` (new) — higher-level helpers: `expand_rrule()`, `tz_convert()`
- `sdk/src/lib.rs` — pub mod calendar
- `src/runtime_wasmi.rs` — linker.func_wrap for both functions
- `Cargo.toml` — add `rrule` dependency to workspace (host-only, not WASM)

**Success**: widget calls `expand_rrule(...)` with a WEEKLY RRULE, gets back correct occurrence timestamps.

### Stage 2: Event data model + calendar merging

**Goal**: proper data model for calendar sources and expanded events. Multi-feed support.

**Files**:

- `examples/calendar/src/ical_parser.rs` — wraps `ical` crate, extracts typed CalendarEvent from raw properties
- `examples/calendar/src/calendar.rs` — CalendarSource (url, label, color, events), merge + sort logic
- `examples/calendar/src/lib.rs` — manages multiple sources, refresh timer, KV persistence of URLs

**Data types**:

```rust
struct CalendarEvent {
    summary: String,
    location: Option<String>,
    description: Option<String>,
    start: i64,          // UTC unix timestamp
    end: i64,
    all_day: bool,
    calendar_idx: u8,    // index into sources (for color)
}

struct CalendarSource {
    url: String,
    label: String,
    color: u32,          // RGBA
    events: Vec<CalendarEvent>,
}

struct DayGroup {
    date: (u16, u8, u8), // year, month, day
    is_today: bool,
    all_day: Vec<usize>,  // indices into merged events
    timed: Vec<usize>,
}
```

**Success**: multiple .ics feeds fetched, parsed, RRULE-expanded, merged chronologically, grouped by day.

### Stage 3: Agenda view rendering — Full + Large

**Goal**: render the agenda view at Full and Large sizes with scrolling.

**Files**:

- `examples/calendar/src/render.rs` — render functions per size variant
- `examples/calendar/src/lib.rs` — render dispatch

**Visual elements**:

- Day header rows (bold date text, today highlighted with accent background)
- All-day event rows (badge + title, color bar)
- Timed event rows (time left-aligned, title, color bar/dot)
- Scroll container wrapping the day list
- Current time indicator line for today

**Success**: Full and Large sizes show scrollable agenda with real iCal data.

### Stage 4: Medium + Small rendering

**Goal**: compact rendering for Medium and Small tiles.

**Files**: same as Stage 3, extended

**Visual elements**:

- Medium: compact rows (time + title), 3-5 days, scrollable
- Small: today-only focus, minimal chrome, scrollable if overflow

**Success**: all 4 size variants render correctly in testbed.

### Stage 5: Polish + interaction

**Goal**: tap-to-expand event details, error/loading/empty states, visual polish.

**Files**: same as above

**Features**:

- Tap event row → expand inline to show location + description (Full/Large only)
- Loading spinner during fetch
- Error state with retry button
- Empty state ("No upcoming events")
- Multi-day event rendering (spans across day boundaries)
- Event time formatting (12h/24h based on locale — use host_format_date)

**Success**: polished, interactive agenda widget across all sizes.

---

## Dependencies

### WASM side (widget)

- `ical = "0.11"` — iCal structural parser (~52K WASM)
- `bmc-wasm-sdk` — existing SDK

### Host side (runtime)

- `rrule = "0.14"` — RRULE expansion (new, host-only)
- `chrono-tz` — already workspace dep, used by rrule

### Existing SDK features used

- `FetchRequest::get()` — HTTP fetch for .ics URLs
- `kv::set/get` — persist configured iCal URLs
- `SystemTime::now()` — current time for "today" detection
- `format_date()` — time/date formatting
- `scroll()` — scrollable event list
- `button!` — retry button, config buttons
- `text()`, `row()`, `col()` — layout primitives
- `canvas()` + `Draw::rect()` — color bars, time indicator
- Design system colors from `bmc_wasm_protocol::colors::*`

---

## Verification

```bash
# Build and run in testbed
cd bmc-wasm-runtime
make run EXAMPLE=calendar

# Check WASM binary size
make size EXAMPLE=calendar
# Target: <80K optimized (ical ~52K + widget logic)

# Validate formatting/clippy
make validate-wasm

# Test with real iCal feeds
# Google Calendar: Settings → calendar → "Secret address in iCal format"
# Outlook: Settings → View all Outlook settings → Calendar → Shared calendars → Publish
```

---

## Key files to modify

| File                                 | Change                                                                                    |
| ------------------------------------ | ----------------------------------------------------------------------------------------- |
| `sdk/src/host.rs`                    | Add `host_expand_rrule`, `host_tz_convert` extern declarations                            |
| `sdk/src/calendar.rs` (new)          | Safe wrappers for RRULE expansion + TZ conversion                                         |
| `sdk/src/lib.rs`                     | `pub mod calendar`                                                                        |
| `src/runtime_wasmi.rs`               | `linker.func_wrap` implementations for both new host functions                            |
| `Cargo.toml` (workspace)             | Add `rrule` to workspace dependencies                                                     |
| `examples/calendar/` (new)           | Entire widget: Cargo.toml, src/lib.rs, src/ical_parser.rs, src/calendar.rs, src/render.rs |
| `examples/calendar/research/plan.md` | Copy of this plan                                                                         |

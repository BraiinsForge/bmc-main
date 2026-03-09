# Stage 5: Month Grid View + Polish

## Status: IN PROGRESS

### What's done so far

**calendar.rs** — all edits applied:

- Added `first_day_of_week: u8` and `use_24h: bool` to `CalendarState` struct + constructor
- Added KV loading for both preferences in `load_sources_from_kv`
- Added complete month grid data model: `MonthGrid`, `WeekRow`, `GridCell`, `EventSpan`
- Added `build_month_grid()` method on `CalendarState`
- Added helpers: `assign_span_lanes`, `days_in_month`, `prev_month`, `next_month`

**lib.rs** — all edits applied:

- Bumped `DAYS_AHEAD` from 14 to 45 (covers full month grid)
- Added 4th calendar source: US Holidays (green, Google Calendar public iCal)

**render.rs** — NEEDS COMPLETE REWRITE (not yet done): The file still has the old 3-column day layout for Full variant.
Needs to be rewritten with:

1. Month grid for Full variant
2. Proper semantic grouping for agenda views
3. Time format helper
4. Reduced code repetition

### What needs to happen in render.rs

#### Current state of render.rs

- Theme struct exists with: surface, surface_accent, surface_event, text_primary, text_secondary, text_day_header,
  now_line, calendar_fallback
- Need to add: `surface_grid_cell: u32` (= GRAY_90) for grid cell backgrounds
- Full variant currently shows 3-column day layout (wrong, needs month grid)
- Agenda views (Large/Medium/Small) push items flat into a single Vec — no semantic grouping
- event_row_full and event_row_compact have duplicated color/time logic

#### Target structure for render.rs

```
// Theme (add surface_grid_cell field)
// Constants (unchanged)
// render_agenda entry point (unchanged)
// Loading/Empty states (unchanged)

// ── Full: Month Grid ────────────────────────
render_full(state, size) → sidebar + month grid
render_month_grid(grid, state) → Vec<Node>: title, day headers, week rows
render_grid_week(week, state) → Node: span lanes + day cells
render_grid_span_lane(spans, state) → Node: spacers + colored bars via flex
render_grid_cell(cell, state) → Node: day number + timed events

// ── Agenda views ────────────────────────────
render_large(state, size): scroll → col(gap: 12.0, render_agenda_stream(compact=false))
render_medium(state, size): scroll → col(gap: 10.0, render_agenda_stream(compact=true))
render_small(state, size): scroll → col(gap: 8.0, limited day sections)

render_agenda_stream(state, compact) → Vec<Node>: builds day sections
render_day_section(group, state, compact, max_events) → Node: PROPER SEMANTIC GROUPING

// ── Shared components ───────────────────────
render_today_card(state) → Vec<Node>: sidebar content
day_header(group, wide) → Node
now_indicator() → Node
event_row_full(event, state, is_all_day) → Node
event_row_compact(event, state) → Node

// ── Helpers ─────────────────────────────────
event_color(event, state) → u32
event_time(event, state) → String: respects use_24h
grid_day_names(first_day_of_week) → [&str; 7]
```

#### Semantic day section grouping (fixes the spacing issues)

```
fn render_day_section(group, state, compact, max_events) -> Node {
    let mut section = vec![day_header(group, !compact)];
    if group.is_today { section.push(now_indicator()); }

    let mut events: Vec<Node> = Vec::new();
    // ... collect events with optional limit ...

    if !events.is_empty() {
        section.push(col(props!(gap: 2.0), events));  // tight within-day gap
    }
    col(props!(), section)  // no extra gap between header and events
}
```

The OUTER col wrapping all day sections uses `gap: 12.0` for between-day spacing. This means: header is tight with its
events, but days are spaced apart.

#### Month grid layout (Full variant, 1280×480)

```
┌──────────┬──────────────────────────────────────────┐
│ Sidebar  │ Mar 2026                                 │
│ 200px    │ Mon  Tue  Wed  Thu  Fri  Sat  Sun        │
│          │ ┌────┬────┬────┬────┬────┬────┬────┐     │
│ Today    │ │ 23 │ 24 │ 25 │ 26 │ 27 │ 28 │  1 │     │
│ card     │ ├────┼────┼────┼────┼────┼────┼────┤     │
│          │ │  2 │  3 │  4 │  5 │  6 │  7 │  8 │     │
│          │ ├────┼────┼────┼────┼────┼────┼────┤     │
│ Legend   │ │  9 │ 10 │ 11 │ 12 │ 13 │ 14 │ 15 │     │
│          │ │    │    │████ F1 Chinese GP █████│     │
│          │ ├────┼────┼────┼────┼────┼────┼────┤     │
│          │ │ 16 │ 17 │ 18 │ 19 │ 20 │ 21 │ 22 │     │
│          │ ├────┼────┼────┼────┼────┼────┼────┤     │
│          │ │ 23 │ 24 │ 25 │ 26 │ 27 │ 28 │ 29 │     │
│          │ └────┴────┴────┴────┴────┴────┴────┘     │
└──────────┴──────────────────────────────────────────┘
```

Grid dimensions (1080px wide, 480px tall):

- Month title: ~30px
- Day name headers: ~24px
- Week rows: fill remaining (~425px / 5-6 weeks = 70-85px each)

Each week row:

- Span lanes: 16px each, max 3 lanes
- Day cells row: flex: 1.0 to fill remaining

Grid cell rendering:

- Day number: today = bold RED_50, current month = text_primary, other = text_secondary
- Timed events: max 2, shown as dot + abbreviated time
- "+N" overflow text
- Background: current month = surface_grid_cell (GRAY_90), other = surface (GRAY_100)
- 1px gaps between cells (surface color shows through as grid lines)

Multi-day span rendering (flex-based):

```
// Event spanning columns 2-4 in a 7-column week:
row(props!(height: 16.0, gap: 1.0), [
    spacer(2.0),                                    // cols 0-1 empty
    row(props!(flex: 3.0, background: color), [...]), // cols 2-4 colored bar
    spacer(2.0),                                    // cols 5-6 empty
])
```

Each flex unit = 1 column. Spacers fill empty columns.

#### Time format

```rust
fn event_time(event: &CalendarEvent, state: &CalendarState) -> String {
    if state.use_24h {
        format_date(event.start, "%H:%M")
    } else {
        format_date(event.start, "%I:%M %p")
    }
}
```

Default: `use_24h = true`. Loaded from KV key `"time_format"` (value `"12h"` switches to 12h).

### SDK capabilities confirmed (from research)

- `TextAlign::Center` — via `style!(align: TextAlign::Center)`
- `TextOverflow::Ellipsis` — via `style!(text_overflow: TextOverflow::Ellipsis)`
- `CrossAlign::{Stretch, Center, Start, End}` — via `props!(cross_align: ...)`
- `Draw::text(x, y, content, style)` — canvas text drawing
- `max_width` on text style — pixel limit for text width
- `strikethrough: bool` on text style — for future past-event styling
- All in `bmc_wasm_sdk::*` (wildcard import already present)

### Grid data model (already implemented in calendar.rs)

```rust
pub struct MonthGrid {
    year: u16,
    month: u8,
    weeks: Vec<WeekRow>
}
pub struct WeekRow {
    cells: [GridCell; 7],
    spans: Vec<EventSpan>
}
pub struct GridCell {
    day: u8,
    month: u8,
    year: u16,
    is_current_month: bool,
    is_today: bool,
    events: Vec<usize>
}
pub struct EventSpan {
    event_idx: usize,
    start_col: u8,
    end_col: u8,
    lane: u8
}
```

Grid computation in `build_month_grid()`:

1. Compute weekday of 1st: `wd1 = (now.weekday - (now.day - 1) + 700) % 7`
2. Offset for first_day_of_week: `offset = (wd1 - fdow + 7) % 7`
3. Fill 5-6 weeks of cells (prev/next month padding)
4. For each event:
   - All-day or duration >= 86400 → span bars (tz_convert start + end, find overlapping weeks)
   - Otherwise → timed event in cell (tz_convert start only)
5. Assign lanes to spans per week (greedy, sorted by start_col then span length)

### Theme changes needed

Add to Theme struct:

```
surface_grid_cell: u32,  // = GRAY_90
```

### Calendar sources (already updated in lib.rs)

1. Czech Holidays (blue) — Google Calendar
2. Formula 1 (red) — better-f1-calendar
3. Finland Holidays (Finnish blue) — officeholidays.com
4. US Holidays (green) — Google Calendar ← NEW

DAYS_AHEAD = 45 (was 14). Covers current month + padding into next month.

### User feedback addressed in earlier stages

- All inline GRAY\_\* replaced with THEME.\* fields ✓
- Sidebar text uses text_primary for contrast ✓
- text_muted removed (semantic pleonasm with text_secondary) ✓
- Day headers: no special background for today (now_line suffices) ✓
- Now indicator: red dot + horizontal line (more prominent than 2px rect) ✓
- surface_accent bumped to GRAY_80 (distinct from surface_event GRAY_90) ✓

### Known issues / future work

- Past events: currently use text_secondary color. User suggested opacity/strikethrough instead. SDK supports
  `strikethrough: bool` on TextStyle. No opacity support yet.
- First day of week: defaults to Monday (0). Configurable via KV `"first_day_of_week"`.
- Grid only shows current month. No month navigation yet.
- Dead code warnings (description, expanded_event, save_sources_to_kv, exdates, duration) left for later stages.

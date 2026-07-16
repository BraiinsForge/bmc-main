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

//! Calendar rendering — month grid (Full) and agenda list (Large/Medium/Small).

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use crate::calendar::{CalendarEvent, CalendarState, DayGroup};
use core::sync::atomic::{AtomicU8, Ordering};

// ── Theme ───────────────────────────────────────────────────────────

/// Semantic color palette for the calendar widget.
/// Construct different instances for different themes/skins.
struct Theme {
    /// Widget background.
    surface: Color,
    /// Sidebar background.
    surface_sidebar: Color,
    /// Event row background (agenda full rows).
    surface_event: Color,
    /// Grid cell background (current month, weekday).
    surface_grid_cell: Color,
    /// Grid cell background (current month, weekend).
    surface_grid_weekend: Color,
    /// Grid cell background (off-month padding days).
    surface_grid_other: Color,
    /// Primary text (titles, headings).
    text_primary: Color,
    /// Secondary text (times, locations, empty states).
    text_secondary: Color,
    /// Day header text.
    text_day_header: Color,
    /// "Now" indicator line / today highlight.
    now_line: Color,
    /// Fallback color when a calendar source has no color.
    calendar_fallback: Color,
}

struct ThemeSet {
    light: Theme,
    dark: Theme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ThemeKey {
    Light = 0,
    Dark = 1,
}

impl ThemeKey {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Light,
            1 => Self::Dark,
            _ => Self::Dark,
        }
    }
}

impl Theme {
    fn dark() -> Self {
        Self {
            surface: GRAY_100,
            surface_sidebar: GRAY_90,
            surface_event: GRAY_90,
            surface_grid_cell: GRAY_90,
            surface_grid_weekend: GRAY_90.lightness(0.22),
            surface_grid_other: GRAY_100.lightness(0.14),
            text_primary: GRAY_10,
            text_secondary: GRAY_50,
            text_day_header: GRAY_30,
            now_line: RED_50,
            calendar_fallback: GRAY_50,
        }
    }

    fn light() -> Self {
        Self {
            surface: GRAY_10,
            surface_sidebar: WHITE,
            surface_event: WHITE,
            surface_grid_cell: WHITE,
            surface_grid_weekend: GRAY_10.lightness(0.98),
            surface_grid_other: GRAY_20.lightness(0.95),
            text_primary: GRAY_100,
            text_secondary: GRAY_60,
            text_day_header: GRAY_80,
            now_line: RED_60,
            calendar_fallback: GRAY_60,
        }
    }
}

static THEMES: std::sync::LazyLock<ThemeSet> = std::sync::LazyLock::new(|| ThemeSet {
    light: Theme::light(),
    dark: Theme::dark(),
});
static THEME_KEY: AtomicU8 = AtomicU8::new(ThemeKey::Dark as u8);

const THEME_DARK_ICON: Svg = include_svg!("assets/icons/theme-dark.svg");
const THEME_LIGHT_ICON: Svg = include_svg!("assets/icons/theme-light.svg");

pub fn set_theme_key(key: ThemeKey) {
    THEME_KEY.store(key as u8, Ordering::Relaxed);
}

fn theme_key() -> ThemeKey {
    ThemeKey::from_u8(THEME_KEY.load(Ordering::Relaxed))
}

fn active_theme() -> &'static Theme {
    match theme_key() {
        ThemeKey::Light => &THEMES.light,
        ThemeKey::Dark => &THEMES.dark,
    }
}

/// Weekday names (0=Mon..6=Sun).
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Month names.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ── Entry point ─────────────────────────────────────────────────────

/// Render the calendar widget for any size variant.
pub fn render_agenda(state: &CalendarState, size: WidgetSize) -> Node {
    if state.any_loading && state.events.is_empty() {
        return render_loading(size);
    }

    if state.events.is_empty() {
        return render_empty(state, size);
    }

    match size.variant {
        SizeVariant::Full => render_full(state, size),
        SizeVariant::Large => render_large(state, size),
        SizeVariant::Medium => render_medium(state, size),
        SizeVariant::Small => render_small(state, size),
    }
}

// ── Loading / Empty ─────────────────────────────────────────────────

fn theme_toggle_fab(theme: &Theme) -> Node {
    let icon = if theme_key() == ThemeKey::Dark {
        tree::ensure_registered(&THEME_LIGHT_ICON)
    } else {
        tree::ensure_registered(&THEME_DARK_ICON)
    };
    let size = 36.0;
    let r = size / 2.0;
    let icon_inset = 10.0;
    let icon_size = size - icon_inset * 2.0;
    touchable(
        "theme_toggle",
        props!(width: size, height: size, inset_bottom: 12.0, inset_right: 12.0),
        [
            // Subtle outline ring — text color mixed toward the button face
            Draw::circle(
                r,
                r,
                r,
                theme.text_secondary.mix(theme.surface_sidebar, 0.7),
            ),
            // Button face — uses sidebar surface for theme-aware contrast
            Draw::circle(r, r, r - 1.0, theme.surface_sidebar),
            // Icon
            Draw::svg_builtin(
                icon_inset,
                icon_inset,
                icon_size,
                icon_size,
                icon,
                theme.text_secondary,
            ),
        ],
    )
}

fn with_fab_overlay(content: Node, theme: &Theme) -> Node {
    col(props!(), [content, theme_toggle_fab(theme)])
}

fn render_loading(size: WidgetSize) -> Node {
    let theme = active_theme();
    with_fab_overlay(
        col(
            props!(
                background: theme.surface,
                padding: 24.0,
                width: size.width as f32,
                height: size.height as f32,
                gap: 8.0
            ),
            [
                spacer(1.0),
                text(
                    "Loading calendars...",
                    style!(size: 20, color: theme.text_secondary),
                ),
                spacer(1.0),
            ],
        ),
        theme,
    )
}

fn render_empty(state: &CalendarState, size: WidgetSize) -> Node {
    let theme = active_theme();
    let has_errors = state.sources.iter().any(|s| s.error.is_some());
    let msg = if has_errors {
        "Failed to load calendars"
    } else {
        "No upcoming events"
    };

    let mut children: Vec<Node> = vec![
        spacer(1.0),
        text(msg, style!(size: 20, color: theme.text_secondary)),
    ];

    if has_errors {
        children.push(button!("retry", "Retry", style: Tertiary, size: Small));
    }

    children.push(spacer(1.0));

    with_fab_overlay(
        col(
            props!(
                background: theme.surface,
                padding: 24.0,
                width: size.width as f32,
                height: size.height as f32,
                gap: 8.0
            ),
            children,
        ),
        theme,
    )
}

// ── Full: Month Grid (1280×480) ─────────────────────────────────────

/// Grid layout constants
const GRID_GAP: f32 = 1.0;
const GRID_PAD: f32 = 8.0;
const TITLE_H: f32 = 32.0;
const HEADER_H: f32 = 24.0;
const DAY_NUM_H: f32 = 16.0;
const SPAN_LANE_H: f32 = 13.0;
const MAX_SPAN_LANES: u8 = 2;
const EVENT_LINE_H: f32 = 12.0;

fn render_full(state: &CalendarState, size: WidgetSize) -> Node {
    let theme = active_theme();
    let w = size.width as f32;
    let h = size.height as f32;
    let sidebar_w: f32 = 200.0;
    let gap: f32 = 4.0;
    let grid = state.build_month_grid();
    let grid_w = w - sidebar_w - gap - GRID_PAD * 2.0;
    let grid_h = h - GRID_PAD * 2.0;

    with_fab_overlay(
        row(
            props!(background: theme.surface, width: w, height: h, gap: gap),
            [
                // Sidebar
                col(
                    props!(
                        width: sidebar_w,
                        height: h,
                        padding: 16.0,
                        gap: 12.0,
                        background: theme.surface_sidebar
                    ),
                    render_today_card(state),
                ),
                // Month grid — single canvas
                canvas(
                    props!(flex: 1.0, height: h, padding: GRID_PAD),
                    render_month_grid_canvas(&grid, state, grid_w, grid_h),
                ),
            ],
        ),
        theme,
    )
}

/// Render the entire month grid as canvas draw commands.
fn render_month_grid_canvas(
    grid: &crate::calendar::MonthGrid,
    state: &CalendarState,
    w: f32,
    h: f32,
) -> Vec<Draw> {
    let theme = active_theme();
    let mut draws = Vec::new();
    let num_weeks = grid.weeks.len() as f32;

    // Column and row geometry
    let col_w = (w - GRID_GAP * 6.0) / 7.0;
    let grid_body_h = h - TITLE_H - HEADER_H;
    let row_h = (grid_body_h - GRID_GAP * (num_weeks - 1.0)) / num_weeks;

    // Month title
    let month_name = MONTHS
        .get(grid.month.wrapping_sub(1) as usize)
        .unwrap_or(&"???");
    draws.push(Draw::text(
        4.0,
        2.0,
        fmt!("{month_name} {}", grid.year),
        style!(size: 18, weight: FontWeight::BOLD, color: theme.text_primary),
    ));

    // Day name headers
    let day_names = grid_day_names(state.first_day_of_week);
    for (ci, name) in day_names.iter().enumerate() {
        let cx = ci as f32 * (col_w + GRID_GAP) + col_w / 2.0;
        draws.push(Draw::text(
            cx,
            TITLE_H,
            *name,
            style!(size: 11, weight: FontWeight::BOLD, color: theme.text_secondary, align: TextAlign::Center),
        ));
    }

    // Week rows
    let body_y = TITLE_H + HEADER_H;
    for (wi, week) in grid.weeks.iter().enumerate() {
        let wy = body_y + wi as f32 * (row_h + GRID_GAP);
        draw_grid_week(&mut draws, week, state, wy, col_w, row_h);
    }

    draws
}

/// Draw a single week row: cell backgrounds, day numbers, span bars, timed events.
fn draw_grid_week(
    draws: &mut Vec<Draw>,
    week: &crate::calendar::WeekRow,
    state: &CalendarState,
    wy: f32,
    col_w: f32,
    row_h: f32,
) {
    let theme = active_theme();
    let col_step = col_w + GRID_GAP;

    // Cell backgrounds + day numbers
    for (ci, cell) in week.cells.iter().enumerate() {
        let cx = ci as f32 * col_step;
        let bg = if !cell.is_current_month {
            theme.surface_grid_other
        } else if cell.is_weekend {
            theme.surface_grid_weekend
        } else {
            theme.surface_grid_cell
        };
        draws.push(Draw::rect(cx, wy, col_w, row_h, bg));

        // Day number (today gets a filled circle badge)
        if cell.is_today {
            draws.push(Draw::circle(cx + 10.0, wy + 9.0, 8.0, theme.now_line));
            draws.push(Draw::text(
                cx + 10.0,
                wy + 1.0,
                fmt!("{}", cell.day),
                style!(size: 11, weight: FontWeight::BOLD, color: WHITE, align: TextAlign::Center),
            ));
        } else {
            let (color, size, weight) = if cell.is_current_month {
                (theme.text_primary, 11, FontWeight::REGULAR)
            } else {
                (theme.text_secondary, 11, FontWeight::REGULAR)
            };
            draws.push(Draw::text(
                cx + 3.0,
                wy + 2.0,
                fmt!("{}", cell.day),
                style!(size: size, weight: weight, color: color),
            ));
        }
    }

    // Multi-day span bars — drawn across cell boundaries
    for span in &week.spans {
        if span.lane >= MAX_SPAN_LANES {
            continue;
        }
        let event = &state.events[span.event_idx];
        let color = event_color(event, state);
        let sx = f32::from(span.start_col) * col_step;
        let ex = (f32::from(span.end_col) + 1.0) * col_step - GRID_GAP;
        let sy = wy + DAY_NUM_H + f32::from(span.lane) * SPAN_LANE_H;
        let bar_w = ex - sx;

        // Colored bar spanning across columns (color already darkened at source)
        let bar_h = SPAN_LANE_H - 1.0;
        draws.push(Draw::rect(sx, sy, bar_w, bar_h, color));

        // Event label text — summary + description snippet for wide multi-day bars
        let label = match &event.description {
            Some(desc) if bar_w > 120.0 && desc != &event.summary => {
                let max_chars = (bar_w / 5.0) as usize;
                let combined = fmt!("{} — {desc}", event.summary);
                truncate_str(&combined, max_chars).to_string()
            }
            _ => event.summary.clone(),
        };
        draws.push(Draw::text(
            sx + 3.0,
            sy - 2.0,
            label,
            style!(size: 10, color: WHITE, max_width: px_to_u32(bar_w - 6.0)),
        ));
    }

    // Timed events within cells — capped to available vertical space
    let num_span_lanes = week
        .spans
        .iter()
        .map(|s| s.lane + 1)
        .max()
        .unwrap_or(0)
        .min(MAX_SPAN_LANES);
    let events_y = wy + DAY_NUM_H + f32::from(num_span_lanes) * SPAN_LANE_H;
    let avail = row_h - (events_y - wy);
    let max_visible = (avail / EVENT_LINE_H).max(0.0) as usize;

    for (ci, cell) in week.cells.iter().enumerate() {
        let cx = ci as f32 * col_step;
        let text_color = if cell.is_current_month {
            theme.text_primary
        } else {
            theme.text_secondary
        };

        // If there are more events than fit, reserve one line for "+N" overflow
        let has_overflow = cell.events.len() > max_visible;
        let limit = if has_overflow {
            max_visible.saturating_sub(1)
        } else {
            max_visible
        };
        for (ei, &event_idx) in cell.events.iter().enumerate() {
            if ei >= limit {
                // Overflow indicator (uses one of the available lines)
                let remaining = cell.events.len() - ei;
                let ey = events_y + ei as f32 * EVENT_LINE_H;
                draws.push(Draw::text(
                    cx + 3.0,
                    ey,
                    fmt!("+{remaining}"),
                    style!(size: 9, color: theme.text_secondary),
                ));
                break;
            }
            let event = &state.events[event_idx];
            let color = event_color(event, state);
            let ey = events_y + ei as f32 * EVENT_LINE_H;

            // Color dot (vertically centered with 9px text)
            draws.push(Draw::circle(cx + 5.0, ey + 6.5, 2.5, color));

            // Time+title or "all day" label, truncated to fit cell
            let max_chars = (col_w - 14.0) as usize * 2 / 9;
            let label = if event.all_day {
                "\u{2022} all day".to_string()
            } else {
                let full = fmt!("{} {}", event_time(event, state), event.summary);
                truncate_str(&full, max_chars).to_string()
            };
            draws.push(Draw::text(
                cx + 11.0,
                ey,
                label,
                style!(size: 9, color: text_color),
            ));
        }
    }
}

// ── Agenda views (Large / Medium / Small) ───────────────────────────

fn render_large(state: &CalendarState, size: WidgetSize) -> Node {
    let theme = active_theme();
    with_fab_overlay(
        scroll(
            "agenda",
            props!(
                background: theme.surface,
                width: size.width as f32,
                height: size.height as f32,
                padding: 16.0
            ),
            [col(props!(gap: 12.0), render_agenda_stream(state, false))],
        ),
        theme,
    )
}

fn render_medium(state: &CalendarState, size: WidgetSize) -> Node {
    let theme = active_theme();
    with_fab_overlay(
        scroll(
            "agenda",
            props!(
                background: theme.surface,
                width: size.width as f32,
                height: size.height as f32,
                padding: 12.0
            ),
            [col(props!(gap: 10.0), render_agenda_stream(state, true))],
        ),
        theme,
    )
}

fn render_small(state: &CalendarState, size: WidgetSize) -> Node {
    let theme = active_theme();
    const MAX_EVENTS: usize = 8;
    let mut sections: Vec<Node> = Vec::new();
    let mut event_count = 0;

    for group in &state.day_groups {
        if event_count >= MAX_EVENTS {
            break;
        }
        let n = group.all_day.len() + group.timed.len();
        if n == 0 {
            continue;
        }
        let remaining = MAX_EVENTS - event_count;
        sections.push(render_day_section(group, state, true, Some(remaining)));
        event_count += n.min(remaining);
    }

    if sections.is_empty() {
        return render_empty(state, size);
    }

    with_fab_overlay(
        scroll(
            "agenda",
            props!(
                background: theme.surface,
                width: size.width as f32,
                height: size.height as f32,
                padding: 10.0
            ),
            [col(props!(gap: 8.0), sections)],
        ),
        theme,
    )
}

/// Build day sections for the agenda stream (Large / Medium).
fn render_agenda_stream(state: &CalendarState, compact: bool) -> Vec<Node> {
    let mut sections: Vec<Node> = Vec::new();
    let has_today = state.day_groups.iter().any(|g| g.is_today);

    // Insert a "Today" placeholder if no events today
    if !has_today {
        let today = &state.local;
        let placeholder = DayGroup {
            year: today.year,
            month: today.month,
            day: today.day,
            weekday: today.weekday,
            is_today: true,
            all_day: Vec::new(),
            timed: Vec::new(),
        };
        sections.push(render_day_section(&placeholder, state, compact, None));
    }

    for group in &state.day_groups {
        sections.push(render_day_section(group, state, compact, None));
    }

    sections
}

/// Render a single day as a semantic group: header + events.
fn render_day_section(
    group: &DayGroup,
    state: &CalendarState,
    compact: bool,
    max_events: Option<usize>,
) -> Node {
    let wide = !compact;
    let mut section: Vec<Node> = vec![day_header(group, wide)];

    if group.is_today {
        section.push(now_indicator());
    }

    let mut events: Vec<Node> = Vec::new();
    let mut count = 0;
    let limit = max_events.unwrap_or(usize::MAX);

    for &idx in &group.all_day {
        if count >= limit {
            break;
        }
        if let Some(event) = state.events.get(idx) {
            events.push(if compact {
                event_row_compact(event, state)
            } else {
                event_row_full(event, state, true)
            });
            count += 1;
        }
    }

    for &idx in &group.timed {
        if count >= limit {
            break;
        }
        if let Some(event) = state.events.get(idx) {
            events.push(if compact {
                event_row_compact(event, state)
            } else {
                event_row_full(event, state, false)
            });
            count += 1;
        }
    }

    if !events.is_empty() {
        section.push(col(props!(gap: 2.0), events));
    }

    col(props!(), section)
}

// ── Shared components ───────────────────────────────────────────────

fn render_today_card(state: &CalendarState) -> Vec<Node> {
    let theme = active_theme();
    let mut children = Vec::new();

    // Today's date header
    let local = &state.local;
    let weekday = WEEKDAYS.get(local.weekday as usize).unwrap_or(&"???");
    let month = MONTHS
        .get(local.month.wrapping_sub(1) as usize)
        .unwrap_or(&"???");

    children.push(text(
        fmt!("{weekday}, {month} {}", local.day),
        style!(size: 24, weight: FontWeight::BOLD, color: theme.text_primary),
    ));
    children.push(text("Today", style!(size: 14, color: theme.text_primary)));

    // Count today's events
    let today_group = state.day_groups.iter().find(|g| g.is_today);
    if let Some(group) = today_group {
        let count = group.all_day.len() + group.timed.len();
        children.push(text(
            fmt!("{count} event{}", if count == 1 { "" } else { "s" }),
            style!(size: 16, color: theme.text_primary),
        ));

        // Show next upcoming event
        let now_ts = state.now.unix_secs;
        let next = group
            .timed
            .iter()
            .filter_map(|&idx| state.events.get(idx))
            .find(|e| e.end > now_ts);
        if let Some(event) = next {
            children.push(text("Next:", style!(size: 12, color: theme.text_primary)));
            let time = event_time(event, state);
            children.push(text(
                fmt!("{time} {}", event.summary),
                style!(size: 14, color: theme.text_primary),
            ));
        }
    } else {
        children.push(text(
            "No events today",
            style!(size: 16, color: theme.text_primary),
        ));
    }

    // Calendar legend
    children.push(spacer(1.0));
    let legend_items: Vec<Node> = state
        .sources
        .iter()
        .map(|source| {
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [
                    canvas(
                        props!(width: 10.0, height: 10.0),
                        [Draw::circle(5.0, 5.0, 5.0, source.color)],
                    ),
                    text(&source.label, style!(size: 12, color: theme.text_primary)),
                ],
            )
        })
        .collect();
    children.push(col(props!(gap: 4.0), legend_items));

    children
}

/// Day header (e.g. "Today • Mon, Mar 10" or "Fri, Mar 13").
fn day_header(group: &DayGroup, wide: bool) -> Node {
    let theme = active_theme();
    let weekday = WEEKDAYS.get(group.weekday as usize).unwrap_or(&"???");
    let month = MONTHS
        .get(group.month.wrapping_sub(1) as usize)
        .unwrap_or(&"???");

    let label = if group.is_today {
        fmt!("Today \u{2022} {weekday}, {month} {}", group.day)
    } else if wide {
        fmt!("{weekday}, {month} {}", group.day)
    } else {
        fmt!("{weekday} {}/{}", group.month, group.day)
    };

    let pad = if wide { 8.0 } else { 4.0 };
    let font_size = if wide { 16 } else { 14 };

    text(
        &label,
        style!(
            size: font_size,
            weight: FontWeight::BOLD,
            color: theme.text_day_header,
            padding: pad
        ),
    )
}

/// Current time indicator line — a bold red line marking "now" in the agenda.
fn now_indicator() -> Node {
    let theme = active_theme();
    row(
        props!(gap: 6.0, cross_align: CrossAlign::Center, padding: 2.0),
        [
            // Circle dot on the left
            canvas(
                props!(width: 8.0, height: 8.0),
                [Draw::circle(4.0, 4.0, 4.0, theme.now_line)],
            ),
            // Horizontal line
            canvas(
                props!(height: 2.0, flex: 1.0),
                [Draw::rect(0.0, 0.0, 9_999.0, 2.0, theme.now_line)],
            ),
        ],
    )
}

/// Full event row with time, color bar, title, optional location, and description.
fn event_row_full(event: &CalendarEvent, state: &CalendarState, is_all_day: bool) -> Node {
    let theme = active_theme();
    let color = event_color(event, state);
    let time_str = if is_all_day {
        "All Day".to_string()
    } else {
        event_time(event, state)
    };

    let is_past = event.end < state.now.unix_secs;
    let text_color = if is_past {
        theme.text_secondary
    } else {
        theme.text_primary
    };

    let mut title_row: Vec<Node> = vec![
        // Time
        text(
            &time_str,
            style!(size: 13, color: theme.text_secondary, width: 56.0),
        ),
        // Title
        text(
            &event.summary,
            style!(size: 14, color: text_color, flex: 1.0),
        ),
    ];

    // Location if available
    if let Some(loc) = &event.location {
        title_row.push(text(
            loc,
            style!(size: 12, color: theme.text_secondary, max_width: 200),
        ));
    }

    let mut content: Vec<Node> = vec![row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        title_row,
    )];

    // Description snippet — skip if it just duplicates the summary
    if let Some(desc) = &event.description {
        if desc != &event.summary {
            let snippet = truncate_str(desc, 120);
            content.push(row(
                props!(gap: 8.0),
                [
                    col(props!(width: 56.0), []),
                    text(
                        snippet,
                        style!(size: 12, color: theme.text_secondary, flex: 1.0),
                    ),
                ],
            ));
        }
    }

    row(
        props!(
            gap: 4.0,
            padding: 4.0,
            background: theme.surface_event,
            cross_align: CrossAlign::Start
        ),
        [
            // Color bar — stretch to match content height
            canvas(
                props!(width: 4.0, height: 32.0),
                [Draw::rect(0.0, 0.0, 4.0, 9_999.0, color)],
            ),
            col(props!(gap: 2.0, flex: 1.0), content),
        ],
    )
}

/// Compact event row — time + title only.
fn event_row_compact(event: &CalendarEvent, state: &CalendarState) -> Node {
    let theme = active_theme();
    let color = event_color(event, state);
    let time_str = if event.all_day {
        "All Day".to_string()
    } else {
        event_time(event, state)
    };

    let is_past = event.end < state.now.unix_secs;
    let text_color = if is_past {
        theme.text_secondary
    } else {
        theme.text_primary
    };

    row(
        props!(gap: 6.0, padding: 3.0, cross_align: CrossAlign::Center),
        [
            // Color dot
            canvas(
                props!(width: 8.0, height: 8.0),
                [Draw::circle(4.0, 4.0, 4.0, color)],
            ),
            text(
                &time_str,
                style!(size: 12, color: theme.text_secondary, width: 48.0),
            ),
            text(
                &event.summary,
                style!(size: 13, color: text_color, flex: 1.0),
            ),
        ],
    )
}

// ── Helpers ─────────────────────────────────────────────────────────

fn event_color(event: &CalendarEvent, state: &CalendarState) -> Color {
    let theme = active_theme();
    state
        .sources
        .get(event.source_idx)
        .map_or(theme.calendar_fallback, |s| s.color)
}

/// Format event time respecting the user's 24h/12h preference.
fn event_time(event: &CalendarEvent, state: &CalendarState) -> String {
    if state.use_24h {
        strftime(event.start, "%H:%M")
    } else {
        strftime(event.start, "%-I:%M %p")
    }
}

/// Weekday names rotated to start from `first_day_of_week`.
fn grid_day_names(first_day_of_week: u8) -> [&'static str; 7] {
    let mut names = [""; 7];
    for i in 0..7 {
        names[i] = WEEKDAYS[(first_day_of_week as usize + i) % 7];
    }
    names
}

/// Truncate a string to at most `max_chars` characters at a char boundary.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        return s;
    }
    let mut end = max_chars;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Convert a pixel width (f32) to u32 for `max_width`, clamping negatives to 0.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn px_to_u32(v: f32) -> u32 {
    if v <= 0.0 { 0 } else { v as u32 }
}

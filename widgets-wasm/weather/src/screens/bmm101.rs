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

//! BMM101's 480×320 frame: today's conditions beside its low, high and sun times,
//! over the forecast for the next four days.

#[expect(
    clippy::wildcard_imports,
    reason = "screen code uses the SDK's tree builders, macros, and tokens throughout"
)]
use bmc_wasm_sdk::*;
use units::units::DegreeCelsius;

use crate::display;
use crate::manifest_params::Params;
use crate::model::{Current, DayForecast, ForecastRange, Weather};
use crate::screens::{common, icons};
use crate::weather_code;

const EDGE: f32 = 16.0;
/// Figma's `normal` line box for Braiins Sans, which every slot below is measured in.
const LINE_HEIGHT: f32 = 1.3;
const TEXT_SIZE: u32 = 20;

/// The frame's inner width less the stat grid and the gap before it,
/// so a long location can never shove the grid out of the frame.
const TODAY_WIDTH: f32 = 236.0;
const LOCATION_SIZE: u32 = 14;
const LOCATION_SLOT: f32 = 18.0;
const TEMPERATURE_SIZE: u32 = 48;
const CURRENT_ICON: f32 = 64.0;
const TODAY_GAP: f32 = 8.0;

/// The stat grid's height. The today column stands 4 px taller,
/// so bottom-aligned with the grid it rises into the top edge, as designed.
const TOP_HEIGHT: f32 = 120.0;
const STAT_WIDTH: f32 = 90.0;
const STAT_COLUMN_GAP: f32 = 16.0;
const STAT_CAPTION_GAP: f32 = 2.0;
const GLYPH_GAP: f32 = 4.0;
const ARROW_BOX: f32 = 12.0;
/// The shipped 16×19 arrows at half size, as the design draws them.
const ARROW_WIDTH: f32 = 8.0;
const ARROW_HEIGHT: f32 = 9.5;
const SUN_GLYPH: f32 = 16.0;
const MERIDIEM_SIZE: u32 = 14;

const RULE_ABOVE: f32 = 12.0;
const RULE_BELOW: f32 = 15.0;

const FORECAST_DAYS: usize = 4;
const DAY_ROW_HEIGHT: f32 = 32.0;
const DAY_ROW_GAP: f32 = 4.0;
const DAY_NAME_WIDTH: f32 = 160.0;
const DAY_ICON: f32 = 32.0;
/// Fixed, so the bars line up down the rows.
/// A row, not a column, holds the value: a three-digit high
/// keeps its width and runs into the edge instead of wrapping.
const MAX_CELL_WIDTH: f32 = 34.0;
const BAR_GAP: f32 = 4.0;
const BAR_WIDTH: f32 = 180.0;
const BAR_THICKNESS: f32 = 4.0;
/// Today's marker: a white dot of radius 5 in a black ring out to 7.
const MARKER_RADIUS: f32 = 5.0;
const MARKER_RING: f32 = 7.0;
/// Figma's white at 20 % over the black frame,
/// opaque so a pill's overlapping pieces never double up.
const TRACK: Color = Color::from_hex(0x33_33_33);

fn label(value: impl Into<String>, size: u32, weight: FontWeight, color: Color) -> Node {
    text(
        value,
        style!(size: size, weight: weight, color: color, line_height: LINE_HEIGHT),
    )
}

fn gap(height: f32) -> Node {
    col(props!(height: height), [])
}

fn location(weather: &Weather) -> Node {
    col(
        props!(height: LOCATION_SLOT),
        [text(
            weather.location.display_name.clone(),
            style!(
                size: LOCATION_SIZE,
                weight: FontWeight::REGULAR,
                color: GRAY_40,
                line_height: LINE_HEIGHT,
                text_overflow: TextOverflow::Ellipsis,
            ),
        )],
    )
}

/// The icon keeps its slot while nothing is loaded, so the frame holds its shape.
fn current_icon(current: Option<&Current>) -> Node {
    current.map_or_else(
        || col(props!(width: CURRENT_ICON, height: CURRENT_ICON), []),
        |c| {
            common::weather_icon(
                weather_code::icon_id(c.weather_code, c.is_day),
                CURRENT_ICON,
            )
        },
    )
}

fn today(weather: &Weather) -> Node {
    let current = weather.current.as_ref();
    let temperature =
        display::temperature_or_placeholder(current.map(|c| c.temperature), display::temperature);
    let condition = current.map_or_else(
        || display::NOT_AVAILABLE.to_owned(),
        |c| weather_code::description(c.weather_code).to_owned(),
    );
    col(
        props!(width: TODAY_WIDTH, gap: TODAY_GAP),
        [
            location(weather),
            row(
                props!(gap: TODAY_GAP, cross_align: CrossAlign::Center),
                [
                    current_icon(current),
                    label(temperature, TEMPERATURE_SIZE, FontWeight::BOLD, WHITE),
                ],
            ),
            label(condition, TEXT_SIZE, FontWeight::REGULAR, GRAY_40),
        ],
    )
}

fn arrow(svg: &'static Svg) -> Node {
    canvas(
        props!(width: ARROW_BOX, height: ARROW_BOX),
        vec![Draw::svg(
            (ARROW_BOX - ARROW_WIDTH) / 2.0,
            (ARROW_BOX - ARROW_HEIGHT) / 2.0,
            ARROW_WIDTH,
            ARROW_HEIGHT,
            svg,
            GRAY_40,
        )],
    )
}

fn value(content: String) -> Node {
    label(content, TEXT_SIZE, FontWeight::REGULAR, GRAY_10)
}

/// A clock reading, with a smaller meridiem beside it on a 12-hour clock.
fn clock(at: Option<SystemTime>, tz: Option<&Tz>) -> Node {
    let mut parts = vec![value(display::hour_label(at, tz))];
    if let Some(meridiem) = display::clock_meridiem(at, tz) {
        parts.push(label(meridiem, MERIDIEM_SIZE, FontWeight::REGULAR, GRAY_40));
    }
    row(
        props!(gap: GLYPH_GAP, cross_align: CrossAlign::Center),
        parts,
    )
}

/// A glyphed caption over its value, both right-aligned.
fn stat(glyph: Node, caption: &str, reading: Node) -> Node {
    col(
        props!(width: STAT_WIDTH, gap: STAT_CAPTION_GAP, cross_align: CrossAlign::End),
        [
            row(
                props!(gap: GLYPH_GAP, cross_align: CrossAlign::Center),
                [
                    glyph,
                    label(caption, TEXT_SIZE, FontWeight::REGULAR, GRAY_40),
                ],
            ),
            reading,
        ],
    )
}

fn stats(weather: &Weather, tz: Option<&Tz>) -> Node {
    let daily = weather.daily.as_ref();
    let today = daily.and_then(|d| d.days.get(d.today_index));
    let degrees = |pick: fn(&DayForecast) -> DegreeCelsius| {
        display::temperature_or_placeholder(today.map(pick), display::temperature)
    };
    col(
        props!(height: TOP_HEIGHT, justify_content: Justify::SpaceBetween),
        [
            row(
                props!(gap: STAT_COLUMN_GAP),
                [
                    stat(arrow(&icons::TEMP_LOW), "Low T.", value(degrees(|d| d.min))),
                    stat(
                        arrow(&icons::TEMP_HIGH),
                        "High T.",
                        value(degrees(|d| d.max)),
                    ),
                ],
            ),
            row(
                props!(gap: STAT_COLUMN_GAP),
                [
                    stat(
                        common::glyph(&icons::SUNRISE, SUN_GLYPH, GRAY_40),
                        "Sunrise",
                        clock(daily.and_then(|d| d.today_sunrise), tz),
                    ),
                    stat(
                        common::glyph(&icons::SUNSET, SUN_GLYPH, GRAY_40),
                        "Sunset",
                        clock(daily.and_then(|d| d.today_sunset), tz),
                    ),
                ],
            ),
        ],
    )
}

/// A 4 px bar with round ends, from `x` over `width`.
fn pill(x: f32, cy: f32, width: f32, color: Color) -> [Draw; 3] {
    let width = width.max(0.0);
    let radius = (BAR_THICKNESS / 2.0).min(width / 2.0);
    [
        Draw::rect(
            x + radius,
            cy - BAR_THICKNESS / 2.0,
            width - 2.0 * radius,
            BAR_THICKNESS,
            color,
        ),
        Draw::circle(x + radius, cy, radius, color),
        Draw::circle(x + width - radius, cy, radius, color),
    ]
}

/// The day's low-to-high span on the week's track, and the current temperature on today's.
fn range_bar(range: &ForecastRange, day: &DayForecast, marker: Option<DegreeCelsius>) -> Node {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "f64 fraction (0..=1) safely narrows to f32 canvas coordinate"
    )]
    let x_of = |c: DegreeCelsius| (range.fraction(c) as f32) * BAR_WIDTH;
    let cy = MARKER_RING;
    let (low, high) = (x_of(day.min), x_of(day.max));
    let mut draws: Vec<Draw> = pill(0.0, cy, BAR_WIDTH, TRACK).into();
    draws.extend(pill(low, cy, high - low, GRAY_40));
    if let Some(current) = marker {
        let cx = x_of(current);
        draws.push(Draw::circle(cx, cy, MARKER_RING, BLACK));
        draws.push(Draw::circle(cx, cy, MARKER_RADIUS, WHITE));
    }
    canvas(props!(width: BAR_WIDTH, height: 2.0 * MARKER_RING), draws)
}

fn day_row(
    day: &DayForecast,
    is_today: bool,
    range: &ForecastRange,
    marker: Option<DegreeCelsius>,
) -> Node {
    let name = if is_today {
        "Today"
    } else {
        crate::model::weekday_name(&day.time_rfc3339).unwrap_or(display::NOT_AVAILABLE)
    };
    row(
        props!(height: DAY_ROW_HEIGHT, cross_align: CrossAlign::Center),
        [
            row(
                props!(
                    width: DAY_NAME_WIDTH,
                    cross_align: CrossAlign::Center,
                    justify_content: Justify::SpaceBetween
                ),
                [
                    label(name, TEXT_SIZE, FontWeight::SEMIBOLD, GRAY_10),
                    common::weather_icon(weather_code::icon_id(day.weather_code, true), DAY_ICON),
                ],
            ),
            spacer(1.0),
            row(
                props!(gap: BAR_GAP, cross_align: CrossAlign::Center),
                [
                    label(
                        display::temperature_bare(day.min),
                        TEXT_SIZE,
                        FontWeight::REGULAR,
                        GRAY_40,
                    ),
                    range_bar(range, day, marker),
                    row(
                        props!(width: MAX_CELL_WIDTH),
                        [value(display::temperature_bare(day.max))],
                    ),
                ],
            ),
        ],
    )
}

fn forecast(weather: &Weather) -> Node {
    let Some(daily) = &weather.daily else {
        return label(
            display::NOT_AVAILABLE,
            TEXT_SIZE,
            FontWeight::REGULAR,
            GRAY_40,
        );
    };
    let window = daily.forecast_window(FORECAST_DAYS);
    let range = ForecastRange::of(window);
    let rows: Vec<Node> = window
        .iter()
        .enumerate()
        .map(|(index, day)| {
            let is_today = index == 0;
            let marker = weather
                .current
                .as_ref()
                .filter(|_| is_today)
                .map(|c| c.temperature);
            day_row(day, is_today, &range, marker)
        })
        .collect();
    col(props!(gap: DAY_ROW_GAP), rows)
}

#[must_use]
pub fn bmm101(weather: &Weather, params: &Params) -> Node {
    let tz = display::select_tz(params.time_zone, &weather.location.timezone);
    col(
        props!(background: BLACK, flex: 1.0, padding: EDGE),
        [
            row(
                props!(
                    height: TOP_HEIGHT,
                    cross_align: CrossAlign::End,
                    justify_content: Justify::SpaceBetween
                ),
                [today(weather), stats(weather, tz.as_ref())],
            ),
            gap(RULE_ABOVE),
            col(props!(height: 1.0, background: GRAY_90), []),
            gap(RULE_BELOW),
            forecast(weather),
        ],
    )
}

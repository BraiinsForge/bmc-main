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

use crate::{
    display,
    render::{bar, icons},
    weather_code,
};
use units::units::DegreeCelsius;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

// Dark-theme design tokens, mirrored from the deckfeeder weather widget:
// `--text-primary` #f4f4f4, `--text-secondary` #c6c6c6, `--border-subtle` #525252.
pub(super) const TEXT_PRIMARY: Color = GRAY_10;
pub(super) const TEXT_SECONDARY: Color = GRAY_30;
pub(super) const BORDER: Color = GRAY_70;

fn weekday(rfc3339: &str) -> String {
    crate::model::weekday_name(rfc3339)
        .unwrap_or(display::NOT_AVAILABLE)
        .to_string()
}

/// A multi-color weather condition icon. `TRANSPARENT` keeps the SVG's
/// authored fills (cloud gray, sun yellow, rain blue) intact.
#[must_use]
pub(super) fn weather_icon(id: weather_code::IconId, size: f32) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![Draw::svg_contain(icons::icon_svg(id), size, TRANSPARENT)],
    )
}

/// A single-path glyph tinted with `color` (sun-times, temp arrows).
#[must_use]
pub(super) fn glyph(svg: &'static Svg, size: f32, color: Color) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![Draw::svg_contain(svg, size, color)],
    )
}

/// Weather-widget text: like `text()` but with deckfeeder's container-wide
/// `line-height: 1`, so large glyphs don't carry the SDK's default 1.4
/// leading (which otherwise bloats vertical spacing and overflows layouts).
#[must_use]
pub(super) fn txt(content: impl Into<String>, size: u32, weight: FontWeight, color: Color) -> Node {
    text(
        content,
        style!(size: size, weight: weight, color: color, line_height: 1.0),
    )
}

/// A fixed-width forecast temperature cell (deckfeeder's
/// `.forecast-temp { width: 4ch }`). The constant width keeps the day icons
/// and sliders aligned across rows regardless of digit count — a bare text
/// node sizes to its content, so the box has to come from a container.
/// `align_right` hugs the value to the slider (low temp); otherwise it is
/// left-aligned (high temp).
fn temp_box(value: String, align_right: bool, style: ForecastRowStyle) -> Node {
    let label = txt(
        value,
        style.temperature_size,
        FontWeight::SEMIBOLD,
        TEXT_PRIMARY,
    );
    let children = if align_right {
        vec![spacer(1.0), label]
    } else {
        vec![label, spacer(1.0)]
    };
    row(
        props!(width: style.temperature_cell_width, cross_align: CrossAlign::Center),
        children,
    )
}

/// Distribute nodes across the main axis with equal flexible gaps between
/// them, emulating CSS `justify-content: space-between`.
#[must_use]
pub(super) fn spread(cells: Vec<Node>) -> Vec<Node> {
    let last = cells.len().saturating_sub(1);
    let mut out: Vec<Node> = Vec::new();
    for (i, cell) in cells.into_iter().enumerate() {
        out.push(cell);
        if i < last {
            out.push(spacer(1.0));
        }
    }
    out
}

#[derive(Clone, Copy)]
pub struct HourStyle {
    pub label_size: u32,
    pub icon: f32,
    pub gap: f32,
    pub temperature_size: u32,
    pub temp_weight: FontWeight,
}

#[must_use]
pub fn hour_cell(entry: &crate::model::HourEntry, tz: Option<&Tz>, style: HourStyle) -> Node {
    let icon_id = weather_code::icon_id(entry.weather_code, entry.is_day);
    col(
        props!(cross_align: CrossAlign::Center, gap: style.gap),
        [
            txt(
                display::forecast_hour_label(&entry.time_rfc3339, tz),
                style.label_size,
                FontWeight::REGULAR,
                TEXT_SECONDARY,
            ),
            weather_icon(icon_id, style.icon),
            txt(
                display::temperature_bare(entry.temperature),
                style.temperature_size,
                style.temp_weight,
                TEXT_PRIMARY,
            ),
        ],
    )
}

/// A sunrise/sunset clock value: the time at `time_size`, plus a smaller
/// secondary meridiem element in 12-hour mode (`hour_label` itself never
/// carries AM/PM, so a bare "9:04" would otherwise be ambiguous).
#[must_use]
pub(super) fn time_with_meridiem(
    rfc3339: &str,
    tz: Option<&Tz>,
    time_size: u32,
    meridiem_size: u32,
    weight: FontWeight,
) -> Node {
    let mut children = vec![txt(
        display::hour_label(rfc3339, tz.cloned()),
        time_size,
        weight,
        TEXT_PRIMARY,
    )];
    if let Some(m) = display::clock_meridiem(rfc3339, tz) {
        children.push(txt(m, meridiem_size, FontWeight::REGULAR, TEXT_SECONDARY));
    }
    row(props!(cross_align: CrossAlign::Center, gap: 4.0), children)
}

#[derive(Clone, Copy)]
pub struct ForecastRowStyle {
    pub row_gap: f32,
    pub label_size: u32,
    pub icon_size: f32,
    pub temperature_size: u32,
    pub temperature_cell_width: f32,
    pub bar_width: f32,
    pub bar_height: f32,
}

impl ForecastRowStyle {
    pub const LARGE: Self = Self {
        row_gap: 12.0,
        label_size: 24,
        icon_size: 40.0,
        temperature_size: 32,
        temperature_cell_width: 80.0,
        bar_width: 140.0,
        bar_height: 16.0,
    };
}

#[must_use]
pub fn forecast_row(
    day: &crate::model::DayForecast,
    is_today: bool,
    range: &crate::model::ForecastRange,
    today_marker: Option<DegreeCelsius>,
    style: ForecastRowStyle,
) -> Node {
    let label = if is_today {
        "Today".to_string()
    } else {
        weekday(&day.time_rfc3339)
    };
    let icon = weather_code::icon_id(day.weather_code, true);
    // Day name on the left; the flexible spacer pins the icon to the right
    // edge of the label area, just before the temperatures (deckfeeder's
    // `.day-label { justify-content: space-between }`). The icon + temps +
    // bar form a fixed-width group, so the icons also line up in one column.
    row(
        props!(cross_align: CrossAlign::Center, gap: style.row_gap),
        [
            txt(label, style.label_size, FontWeight::SEMIBOLD, TEXT_PRIMARY),
            spacer(1.0),
            weather_icon(icon, style.icon_size),
            temp_box(display::temperature_bare(day.min), true, style),
            bar::forecast_bar(
                style.bar_width,
                style.bar_height,
                range,
                day.min,
                day.max,
                today_marker,
            ),
            temp_box(display::temperature_bare(day.max), false, style),
        ],
    )
}

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
    render::{
        common::{self, BORDER, HourStyle, TEXT_PRIMARY, TEXT_SECONDARY},
        icons,
    },
    weather_code, wind,
};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const HOUR_STYLE: HourStyle = HourStyle {
    label_size: 24,
    icon: 40.0,
    gap: 8.0,
    temperature_size: 32,
    temp_weight: FontWeight::REGULAR,
};

fn sun_item(svg: &'static Svg, rfc3339: &str, tz: Option<&Tz>) -> Node {
    row(
        props!(cross_align: CrossAlign::Center, gap: 8.0),
        [
            common::glyph(svg, 28.0, TEXT_SECONDARY),
            common::time_with_meridiem(rfc3339, tz, 32, 20, FontWeight::REGULAR),
        ],
    )
}

fn current_block(weather: &crate::model::Weather) -> Node {
    let current = weather.current.as_ref();
    let temp_and_icon = {
        let mut row_children: Vec<Node> = vec![common::txt(
            display::temperature_or_placeholder(
                current.map(|c| c.temperature),
                display::temperature,
            ),
            96,
            FontWeight::BOLD,
            TEXT_PRIMARY,
        )];
        if let Some(c) = current {
            row_children.push(common::weather_icon(
                weather_code::icon_id(c.weather_code, c.is_day),
                68.0,
            ));
        }
        row(
            props!(cross_align: CrossAlign::Center, gap: 24.0),
            row_children,
        )
    };

    col(
        props!(cross_align: CrossAlign::Start, gap: 12.0),
        [
            common::txt(
                weather.location.display_name.clone(),
                24,
                FontWeight::REGULAR,
                TEXT_SECONDARY,
            ),
            temp_and_icon,
            common::txt(
                current.map_or_else(
                    || display::NOT_AVAILABLE.to_string(),
                    |c| weather_code::description(c.weather_code).to_string(),
                ),
                24,
                FontWeight::REGULAR,
                TEXT_SECONDARY,
            ),
        ],
    )
}

fn weather_info(weather: &crate::model::Weather, tz: Option<&Tz>) -> Node {
    let mut children: Vec<Node> = Vec::new();

    let wind = weather
        .current
        .as_ref()
        .and_then(|c| match (c.wind_speed.as_option(), c.wind_direction.as_option()) {
            (Some(&speed), Some(&dir)) => Some(text(
                display::wind_line(wind::cardinal(dir), &display::wind_speed(speed)),
                style!(size: 24, weight: FontWeight::REGULAR, color: TEXT_SECONDARY, flex: 1.0, line_height: 1.0),
            )),
            _ => None,
        });
    match wind {
        Some(w) => children.push(w),
        None => children.push(spacer(1.0)),
    }

    if let Some(daily) = &weather.daily {
        children.push(row(
            props!(cross_align: CrossAlign::Center, gap: 24.0),
            [
                sun_item(&icons::SUNRISE, &daily.today_sunrise, tz),
                sun_item(&icons::SUNSET, &daily.today_sunset, tz),
            ],
        ));
    }

    col(
        props!(gap: 12.0),
        [
            col(props!(height: 1.0, background: BORDER), []),
            row(props!(cross_align: CrossAlign::Center, gap: 32.0), children),
        ],
    )
}

#[must_use]
pub fn full(
    weather: &crate::model::Weather,
    params: &crate::manifest_params::Params,
    _size: WidgetSize,
) -> Node {
    let tz = display::select_tz(params.time_zone, &weather.location.timezone);

    let strip: Vec<Node> = weather
        .hourly
        .as_ref()
        .map(|h| {
            h.entries
                .iter()
                .skip(h.start_index)
                .take(9)
                .map(|e| common::hour_cell(e, tz.as_ref(), HOUR_STYLE))
                .collect()
        })
        .unwrap_or_default();

    let hourly_section = col(
        props!(flex: 1.0, gap: 16.0),
        [
            row(
                props!(cross_align: CrossAlign::Center),
                common::spread(strip),
            ),
            weather_info(weather, tz.as_ref()),
        ],
    );

    let today_section = row(
        props!(cross_align: CrossAlign::Start, gap: 96.0),
        [current_block(weather), hourly_section],
    );

    let forecast = if let Some(daily) = &weather.daily {
        let window = daily.forecast_window(8);
        let range = crate::model::ForecastRange::of(window);
        let mut rows: Vec<Node> = window
            .iter()
            .enumerate()
            .map(|(i, day)| {
                let is_today = i == 0;
                let marker = if is_today {
                    weather.current.as_ref().map(|c| c.temperature)
                } else {
                    None
                };
                common::forecast_row(
                    day,
                    is_today,
                    &range,
                    marker,
                    common::ForecastRowStyle::LARGE,
                )
            })
            .collect();
        let right = rows.split_off(rows.len().min(4));
        let left = rows;
        row(
            props!(gap: 24.0),
            [
                col(props!(flex: 1.0, gap: 12.0), left),
                col(props!(width: 1.0, background: BORDER), []),
                col(props!(flex: 1.0, gap: 12.0), right),
            ],
        )
    } else {
        col(
            props!(),
            [common::txt(
                display::NOT_AVAILABLE.to_string(),
                24,
                FontWeight::REGULAR,
                TEXT_SECONDARY,
            )],
        )
    };

    col(
        props!(background: BLACK, flex: 1.0, padding: 16.0, gap: 32.0),
        [today_section, forecast],
    )
}

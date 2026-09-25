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
    screens::common::{self, BORDER, HourStyle, TEXT_PRIMARY, TEXT_SECONDARY},
    weather_code,
};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const HOUR_STYLE: HourStyle = HourStyle {
    label_size: 24,
    icon: 52.0,
    gap: 6.0,
    temperature_size: 32,
    temp_weight: FontWeight::REGULAR,
};

/// Five three-digit Fahrenheit hours would crowd the strip at full size.
const FAHRENHEIT_TEMPERATURE_SCALE: f32 = 0.875;

fn hour_style() -> HourStyle {
    if system::current().temperature_unit() == Some(system::TemperatureUnit::Fahrenheit) {
        HourStyle {
            temperature_size: scale_font(HOUR_STYLE.temperature_size, FAHRENHEIT_TEMPERATURE_SCALE),
            ..HOUR_STYLE
        }
    } else {
        HOUR_STYLE
    }
}

fn current_left(current: Option<&crate::model::Current>) -> Node {
    let mut stack: Vec<Node> = Vec::new();
    if let Some(c) = current {
        stack.push(common::weather_icon(
            weather_code::icon_id(c.weather_code, c.is_day),
            72.0,
        ));
    }
    stack.push(common::txt(
        display::temperature_or_placeholder(current.map(|c| c.temperature), display::temperature),
        64,
        FontWeight::BOLD,
        TEXT_PRIMARY,
    ));
    stack.push(common::txt(
        current.map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |c| weather_code::description(c.weather_code).to_string(),
        ),
        24,
        FontWeight::REGULAR,
        TEXT_SECONDARY,
    ));
    col(props!(cross_align: CrossAlign::Start, gap: 12.0), stack)
}

#[must_use]
pub fn medium(
    weather: &crate::model::Weather,
    params: &crate::manifest_params::Params,
    _size: WidgetSize,
) -> Node {
    let tz = display::select_tz(params.time_zone, &weather.location.timezone);
    let style = hour_style();

    let hour_cells: Vec<Node> = weather
        .hourly
        .as_ref()
        .map(|h| {
            h.entries
                .iter()
                .skip(h.start_index)
                .take(5)
                .map(|e| common::hour_cell(e, tz.as_ref(), style))
                .collect()
        })
        .unwrap_or_default();

    let right = col(
        props!(flex: 1.0, gap: 20.0),
        [
            common::location(&weather.location.display_name, 24, None),
            col(props!(height: 1.0, background: BORDER), []),
            common::hour_strip(hour_cells),
        ],
    );

    row(
        props!(background: BLACK, flex: 1.0, padding: 16.0, gap: 32.0, cross_align: CrossAlign::Center),
        [current_left(weather.current.as_ref()), right],
    )
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::system::{self, SnapshotBuilder, TemperatureUnit};

    use super::{HOUR_STYLE, hour_style};

    fn install(unit: TemperatureUnit) {
        system::set_current(SnapshotBuilder::new().temperature_unit(unit).build());
    }

    #[test]
    fn fahrenheit_shrinks_the_hourly_temperatures_to_seven_eighths() {
        install(TemperatureUnit::Fahrenheit);
        assert_eq!(hour_style().temperature_size, 28);
        install(TemperatureUnit::Celsius);
        assert_eq!(hour_style().temperature_size, HOUR_STYLE.temperature_size);
    }
}

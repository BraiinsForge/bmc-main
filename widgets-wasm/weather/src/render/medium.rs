// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::{
    display,
    render::common::{self, BORDER, HourStyle, TEXT_PRIMARY, TEXT_SECONDARY},
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

fn current_left(current: Option<&crate::model::Current>) -> Node {
    let mut stack: Vec<Node> = Vec::new();
    if let Some(c) = current {
        stack.push(common::weather_icon(
            weather_code::icon_id(c.weather_code, c.is_day),
            72.0,
        ));
    }
    stack.push(common::txt(
        display::temperature_or_placeholder(current.map(|c| c.temperature_c), display::temperature),
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

    let hour_cells: Vec<Node> = weather
        .hourly
        .as_ref()
        .map(|h| {
            h.entries
                .iter()
                .skip(h.start_index)
                .take(5)
                .map(|e| common::hour_cell(e, tz.as_ref(), HOUR_STYLE))
                .collect()
        })
        .unwrap_or_default();

    let right = col(
        props!(flex: 1.0, gap: 20.0),
        [
            common::txt(
                weather.location.display_name.clone(),
                24,
                FontWeight::REGULAR,
                TEXT_SECONDARY,
            ),
            col(props!(height: 1.0, background: BORDER), []),
            row(
                props!(cross_align: CrossAlign::Center),
                common::spread(hour_cells),
            ),
        ],
    );

    row(
        props!(background: BLACK, flex: 1.0, padding: 16.0, gap: 32.0, cross_align: CrossAlign::Center),
        [current_left(weather.current.as_ref()), right],
    )
}

// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::{
    display,
    render::{
        common::{self, BORDER, TEXT_PRIMARY, TEXT_SECONDARY},
        icons,
    },
    weather_code,
};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

fn stat_item(svg: &'static Svg, label: &str, value: Node) -> Node {
    col(
        props!(cross_align: CrossAlign::Center, gap: 6.0),
        [
            row(
                props!(cross_align: CrossAlign::Center, gap: 8.0),
                [
                    common::glyph(svg, 24.0, TEXT_SECONDARY),
                    common::txt(label.to_string(), 24, FontWeight::REGULAR, TEXT_SECONDARY),
                ],
            ),
            value,
        ],
    )
}

fn current_left(weather: &crate::model::Weather) -> Node {
    let current = weather.current.as_ref();
    let mut stack: Vec<Node> = vec![common::txt(
        weather.location.display_name.clone(),
        16,
        FontWeight::REGULAR,
        TEXT_SECONDARY,
    )];
    if let Some(c) = current {
        stack.push(common::weather_icon(
            weather_code::icon_id(c.weather_code, c.is_day),
            68.0,
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

fn stats_panel(weather: &crate::model::Weather, tz: Option<&Tz>) -> Node {
    let Some(daily) = &weather.daily else {
        return common::txt(
            display::NOT_AVAILABLE.to_string(),
            24,
            FontWeight::REGULAR,
            TEXT_SECONDARY,
        );
    };
    let today = daily.days.get(daily.today_index);
    let low = today.map_or_else(
        || display::NOT_AVAILABLE.to_string(),
        |d| display::temperature(d.min_c),
    );
    let high = today.map_or_else(
        || display::NOT_AVAILABLE.to_string(),
        |d| display::temperature(d.max_c),
    );
    col(
        props!(gap: 12.0),
        [
            row(
                props!(gap: 24.0),
                [
                    stat_item(
                        &icons::TEMP_LOW,
                        "Low T.",
                        common::txt(low, 32, FontWeight::SEMIBOLD, TEXT_PRIMARY),
                    ),
                    stat_item(
                        &icons::TEMP_HIGH,
                        "High T.",
                        common::txt(high, 32, FontWeight::SEMIBOLD, TEXT_PRIMARY),
                    ),
                ],
            ),
            row(
                props!(gap: 24.0),
                [
                    stat_item(
                        &icons::SUNRISE,
                        "Sunrise",
                        common::time_with_meridiem(
                            &daily.today_sunrise,
                            tz,
                            32,
                            FontWeight::SEMIBOLD,
                        ),
                    ),
                    stat_item(
                        &icons::SUNSET,
                        "Sunset",
                        common::time_with_meridiem(
                            &daily.today_sunset,
                            tz,
                            32,
                            FontWeight::SEMIBOLD,
                        ),
                    ),
                ],
            ),
        ],
    )
}

#[must_use]
pub fn large(
    weather: &crate::model::Weather,
    params: &crate::manifest_params::Params,
    _size: WidgetSize,
) -> Node {
    let tz = display::select_tz(params.time_zone, &weather.location.timezone);

    // Stretch the row so the stat grid can bottom-align (deckfeeder
    // `align-self: end`) against the current block via a leading spacer.
    let today = row(
        props!(cross_align: CrossAlign::Stretch, gap: 24.0),
        [
            current_left(weather),
            spacer(1.0),
            col(props!(), [spacer(1.0), stats_panel(weather, tz.as_ref())]),
        ],
    );

    let forecast = if let Some(daily) = &weather.daily {
        let n = daily.days.len().min(4);
        let window = &daily.days[..n];
        let range = crate::model::ForecastRange::of(window);
        let rows: Vec<Node> = window
            .iter()
            .enumerate()
            .map(|(i, day)| {
                let is_today = i == daily.today_index;
                let marker = if is_today {
                    weather.current.as_ref().map(|c| c.temperature_c)
                } else {
                    None
                };
                common::forecast_row(day, is_today, &range, marker)
            })
            .collect();
        col(props!(gap: 12.0), rows)
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
        props!(background: BLACK, flex: 1.0, padding: 16.0, gap: 24.0),
        [
            today,
            col(props!(height: 1.0, background: BORDER), []),
            forecast,
        ],
    )
}

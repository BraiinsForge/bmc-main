// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::{
    display,
    render::{bar, common, icons},
    weather_code,
};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const GLYPH: f32 = 24.0;
const ICON_SM: f32 = 32.0;
const BAR_W: f32 = 160.0;
const BAR_H: f32 = 20.0;

fn weekday(rfc3339: &str) -> String {
    match parse_date(rfc3339) {
        Some(unix) => strftime(unix + crate::model::offset_seconds(rfc3339), "%A"),
        None => display::NOT_AVAILABLE.to_string(),
    }
}

fn stat_item(svg: &'static Svg, value: String) -> Node {
    row(
        props!(cross_align: CrossAlign::Center, gap: 4.0),
        [
            canvas(
                props!(width: GLYPH, height: GLYPH),
                vec![Draw::svg(0.0, 0.0, GLYPH, GLYPH, svg, GRAY_60)],
            ),
            text(
                value,
                style!(size: 20, weight: FontWeight::REGULAR, color: GRAY_60),
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

    let header = text(
        weather.location.display_name.clone(),
        style!(size: 24, weight: FontWeight::REGULAR, color: GRAY_60),
    );

    let current = common::current_block(weather.current.as_ref());

    let stats_panel = if let Some(daily) = &weather.daily {
        let today = daily.days.get(daily.today_index);
        let low = today.map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |d| display::temperature(d.min_c),
        );
        let high = today.map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |d| display::temperature(d.max_c),
        );
        let sunrise = display::hour_label(&daily.today_sunrise, tz.clone());
        let sunset = display::hour_label(&daily.today_sunset, tz.clone());
        row(
            props!(gap: 16.0, cross_align: CrossAlign::Center),
            [
                stat_item(&icons::TEMP_LOW, low),
                stat_item(&icons::TEMP_HIGH, high),
                stat_item(&icons::SUNRISE, sunrise),
                stat_item(&icons::SUNSET, sunset),
            ],
        )
    } else {
        row(
            props!(),
            [text(
                display::NOT_AVAILABLE.to_string(),
                style!(size: 20, weight: FontWeight::REGULAR, color: GRAY_60),
            )],
        )
    };

    let forecast = if let Some(daily) = &weather.daily {
        let n = daily.days.len().min(4);
        let window = &daily.days[..n];
        let range = crate::model::ForecastRange::of(window);

        let rows: Vec<Node> = window
            .iter()
            .enumerate()
            .map(|(i, day)| {
                let label = if i == daily.today_index {
                    "Today".to_string()
                } else {
                    weekday(&day.time_rfc3339)
                };
                let icon_id = weather_code::icon_id(day.weather_code, true);
                let today_marker = if i == daily.today_index {
                    weather.current.as_ref().map(|c| c.temperature_c)
                } else {
                    None
                };
                row(
                    props!(cross_align: CrossAlign::Center, gap: 8.0),
                    [
                        text(
                            label,
                            style!(size: 20, weight: FontWeight::REGULAR, color: GRAY_60),
                        ),
                        canvas(
                            props!(width: ICON_SM, height: ICON_SM),
                            vec![Draw::svg(
                                0.0,
                                0.0,
                                ICON_SM,
                                ICON_SM,
                                icons::icon_svg(icon_id),
                                TRANSPARENT,
                            )],
                        ),
                        text(
                            display::temperature(day.min_c),
                            style!(size: 20, weight: FontWeight::REGULAR, color: GRAY_60),
                        ),
                        bar::forecast_bar(BAR_W, BAR_H, &range, day.min_c, day.max_c, today_marker),
                        text(
                            display::temperature(day.max_c),
                            style!(size: 20, weight: FontWeight::REGULAR, color: WHITE),
                        ),
                    ],
                )
            })
            .collect();

        col(props!(gap: 4.0), rows)
    } else {
        col(
            props!(),
            [text(
                display::NOT_AVAILABLE.to_string(),
                style!(size: 20, weight: FontWeight::REGULAR, color: GRAY_60),
            )],
        )
    };

    col(
        props!(background: BLACK, flex: 1.0, gap: 8.0, cross_align: CrossAlign::Center),
        [header, current, stats_panel, forecast],
    )
}

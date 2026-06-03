// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::{display, render::icons, weather_code};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const ICON_LG: f32 = 64.0;
const ICON_SM: f32 = 32.0;

#[must_use]
pub fn current_block(current: Option<&crate::model::Current>) -> Node {
    let mut children: Vec<Node> = Vec::new();

    if let Some(c) = current {
        let icon_id = weather_code::icon_id(c.weather_code, c.is_day);
        children.push(canvas(
            props!(width: ICON_LG, height: ICON_LG),
            vec![Draw::svg(
                0.0,
                0.0,
                ICON_LG,
                ICON_LG,
                icons::icon_svg(icon_id),
                TRANSPARENT,
            )],
        ));
    }

    children.push(text(
        display::temperature_or_placeholder(current.map(|c| c.temperature_c), display::temperature),
        style!(size: 64, weight: FontWeight::SEMIBOLD, color: WHITE),
    ));

    children.push(text(
        current.map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |c| weather_code::description(c.weather_code).to_string(),
        ),
        style!(size: 24, weight: FontWeight::REGULAR, color: GRAY_60),
    ));

    col(props!(cross_align: CrossAlign::Center), children)
}

#[must_use]
pub fn hour_cell(entry: &crate::model::HourEntry, tz: Option<Tz>) -> Node {
    let icon_id = weather_code::icon_id(entry.weather_code, entry.is_day);
    col(
        props!(cross_align: CrossAlign::Center, gap: 4.0),
        [
            text(
                display::hour_label(&entry.time_rfc3339, tz),
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
                display::temperature(entry.temperature_c),
                style!(size: 24, weight: FontWeight::REGULAR, color: WHITE),
            ),
        ],
    )
}

// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::{display, render::icons, weather_code};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const ICON_PX: f32 = 64.0;

#[must_use]
pub fn small(
    weather: &crate::model::Weather,
    _params: &crate::manifest_params::Params,
    _size: WidgetSize,
) -> Node {
    let mut children: Vec<Node> = Vec::new();

    if let Some(c) = &weather.current {
        let icon_id = weather_code::icon_id(c.weather_code, c.is_day);
        children.push(canvas(
            props!(width: ICON_PX, height: ICON_PX),
            vec![Draw::svg(
                0.0,
                0.0,
                ICON_PX,
                ICON_PX,
                icons::icon_svg(icon_id),
                TRANSPARENT,
            )],
        ));
    }

    children.push(text(
        display::temperature_or_placeholder(
            weather.current.as_ref().map(|c| c.temperature_c),
            display::temperature,
        ),
        style!(size: 64, weight: FontWeight::SEMIBOLD, color: WHITE),
    ));

    children.push(text(
        weather.current.as_ref().map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |c| weather_code::description(c.weather_code).to_string(),
        ),
        style!(size: 24, weight: FontWeight::REGULAR, color: GRAY_60),
    ));

    children.push(text(
        weather.location.display_name.clone(),
        style!(size: 24, weight: FontWeight::REGULAR, color: GRAY_60),
    ));

    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [col(props!(cross_align: CrossAlign::Center), children)],
        )],
    )
}

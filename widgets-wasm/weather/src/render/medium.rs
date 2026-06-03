// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::{display, render::common};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

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
                .take(5)
                .map(|e| common::hour_cell(e, tz.clone()))
                .collect()
        })
        .unwrap_or_default();

    let forecast_strip = row(props!(gap: 16.0), hour_cells);

    let right = col(
        props!(cross_align: CrossAlign::Center, gap: 8.0, flex: 1.0),
        [
            text(
                weather.location.display_name.clone(),
                style!(size: 24, weight: FontWeight::REGULAR, color: GRAY_60),
            ),
            forecast_strip,
        ],
    );

    row(
        props!(background: BLACK, flex: 1.0),
        [
            center(props!(), [common::current_block(weather.current.as_ref())]),
            right,
        ],
    )
}

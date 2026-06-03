// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::model::ForecastRange;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

#[must_use]
pub fn forecast_bar(
    width: f32,
    height: f32,
    range: &ForecastRange,
    min_c: f64,
    max_c: f64,
    today_marker: Option<f64>,
) -> Node {
    let y = height / 2.0;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "f64 fraction (0..=1) safely narrows to f32 canvas coordinate"
    )]
    let x_of = |c: f64| (range.fraction(c) as f32) * width;
    let mut draws = vec![
        path!(vec![(0.0, y), (width, y)], stroke: 2.0, color: GRAY_30),
        path!(vec![(x_of(min_c), y), (x_of(max_c), y)], stroke: 4.0, color: WHITE),
    ];
    if let Some(cur) = today_marker {
        draws.push(Draw::circle(x_of(cur), y, 4.0, WHITE));
    }
    canvas(props!(width: width, height: height), draws)
}

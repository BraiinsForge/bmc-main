// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::render::common;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

#[must_use]
pub fn small(
    weather: &crate::model::Weather,
    _params: &crate::manifest_params::Params,
    _size: WidgetSize,
) -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [col(
                props!(cross_align: CrossAlign::Center),
                [
                    common::current_block(weather.current.as_ref()),
                    text(
                        weather.location.display_name.clone(),
                        style!(size: 24, weight: FontWeight::REGULAR, color: GRAY_60),
                    ),
                ],
            )],
        )],
    )
}

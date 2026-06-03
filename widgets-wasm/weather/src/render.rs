// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod bar;
pub mod icons;
pub mod small;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::{manifest_params::Params, model::Weather};

#[must_use]
pub fn current_view(weather: &Weather, params: &Params, size: WidgetSize) -> Node {
    small::small(weather, params, size)
}

#[must_use]
pub fn message_view(message: &str, _size: WidgetSize) -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [text(
                message.to_string(),
                style!(size: 32, weight: FontWeight::REGULAR, color: GRAY_60),
            )],
        )],
    )
}

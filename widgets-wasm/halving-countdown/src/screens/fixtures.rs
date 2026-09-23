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

//! Predictions, states and viewports to stage the layouts at, off-device.

use bmc_wasm_sdk::{ViewportShape, WidgetViewport};
use units::availability::Availability;

use crate::manifest_params::{NumbersFontStyle, Params};
use crate::model::{Prediction, SizeBucket, Status};
use crate::screens::ViewData;

/// 22 July 2026, 13:09:35 UTC — when the capture fixture was recorded.
const NOW: i64 = 1_784_725_775;

const DAY_SECS: i64 = 86_400;

/// What Nexus served at [`NOW`]: 631 days and 90 869 blocks out.
const RECORDED: Prediction = Prediction {
    current_height: 959_131,
    target_block: 1_050_000,
    predicted_unix: 1_839_247_175,
};

/// The first block of an era: four-digit days and six-digit blocks,
/// the widest the numerals get.
const ERA_START: Prediction = Prediction {
    current_height: 840_001,
    target_block: 1_050_000,
    predicted_unix: NOW + 1_458 * DAY_SECS + 23 * 3_600 + 59 * 60,
};

/// Two blocks out: every place-value at zero but the minutes.
const IMMINENT: Prediction = Prediction {
    current_height: 1_049_998,
    target_block: 1_050_000,
    predicted_unix: NOW + 17 * 60,
};

/// The manifest default: bold numerals.
#[must_use]
pub fn default_params() -> Params {
    Params {
        numbers_font_style: NumbersFontStyle::Bold,
    }
}

#[must_use]
pub fn rectangular(width: u32, height: u32) -> WidgetViewport {
    WidgetViewport {
        width,
        height,
        shape: ViewportShape::Rectangular,
    }
}

#[must_use]
pub fn round(side: u32) -> WidgetViewport {
    WidgetViewport {
        width: side,
        height: side,
        shape: ViewportShape::Round,
    }
}

#[must_use]
pub fn at_bucket(bucket: SizeBucket) -> WidgetViewport {
    let (width, height) = bucket.design_size();
    rectangular(width, height)
}

fn view(
    viewport: WidgetViewport,
    params: Params,
    prediction: Availability<Prediction>,
    status: Status,
) -> ViewData {
    ViewData {
        viewport,
        params,
        prediction,
        status,
        now_secs: NOW,
    }
}

#[must_use]
pub fn healthy(viewport: WidgetViewport, params: Params) -> ViewData {
    view(
        viewport,
        params,
        Availability::Available(RECORDED),
        Status::Ready,
    )
}

#[must_use]
pub fn loading(viewport: WidgetViewport, params: Params) -> ViewData {
    view(viewport, params, Availability::Unavailable, Status::Ready)
}

#[must_use]
pub fn failed(viewport: WidgetViewport, params: Params) -> ViewData {
    view(viewport, params, Availability::Failed, Status::Failed)
}

#[must_use]
pub fn stale(viewport: WidgetViewport, params: Params) -> ViewData {
    // The gallery clock starts at zero, so this reads as 18 minutes elapsed.
    view(
        viewport,
        params,
        Availability::Available(RECORDED),
        Status::Stale(-18 * 60),
    )
}

#[must_use]
pub fn rate_limited(viewport: WidgetViewport, params: Params) -> ViewData {
    view(
        viewport,
        params,
        Availability::Available(RECORDED),
        Status::RateLimited,
    )
}

#[must_use]
pub fn era_start(viewport: WidgetViewport, params: Params) -> ViewData {
    view(
        viewport,
        params,
        Availability::Available(ERA_START),
        Status::Ready,
    )
}

#[must_use]
pub fn imminent(viewport: WidgetViewport, params: Params) -> ViewData {
    view(
        viewport,
        params,
        Availability::Available(IMMINENT),
        Status::Ready,
    )
}

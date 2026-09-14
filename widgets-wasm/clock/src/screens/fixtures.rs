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

//! Moments and params to stage the faces at, off-device.

use bmc_wasm_sdk::{SystemTime, ViewportShape, WidgetViewport};

use crate::manifest_params::{ClockStyle, NumbersFontStyle, Params};
use crate::model::{ClockHandTransition, SizeBucket};
use crate::screens::ViewData;

/// Wednesday the 24th of December 2025, 00:39:30 in Prague —
/// the moment the BMM101 design frame shows, with its short month and its `AM`.
pub const DESIGN_MOMENT: SystemTime = SystemTime {
    unix_secs: 1_766_533_170,
};

/// Monday the 14th of September 2026, 12:30:00 in Prague — a long month name
/// at noon, where the hour and minute hands overlap.
pub const SEPTEMBER_NOON: SystemTime = SystemTime {
    unix_secs: 1_789_381_800,
};

/// Friday the 1st of May 2026, 21:05:09 in Prague — a short month, an evening
/// with all three hands apart and a `PM`.
pub const MAY_EVENING: SystemTime = SystemTime {
    unix_secs: 1_777_662_309,
};

/// The manifest defaults: digital, semi-bold, every readout on, system timezone.
#[must_use]
pub fn default_params() -> Params {
    Params {
        clock_style: ClockStyle::Digital,
        numbers_font_style: NumbersFontStyle::SemiBold,
        show_date: true,
        show_seconds: true,
        show_timezone: true,
        timezone_override: None,
    }
}

/// `params` drawn at `now` into a rectangular viewport of `width`×`height`,
/// the hands sweeping as on a steady tick.
#[must_use]
pub fn rectangular(width: u32, height: u32, now: SystemTime, params: Params) -> ViewData {
    ViewData {
        now,
        viewport: WidgetViewport {
            width,
            height,
            shape: ViewportShape::Rectangular,
        },
        params,
        hand_transition: ClockHandTransition::Animate,
    }
}

/// `params` drawn at `now` into a round viewport of `side`×`side`,
/// the hands sweeping as on a steady tick.
#[must_use]
pub fn round(side: u32, now: SystemTime, params: Params) -> ViewData {
    ViewData {
        now,
        viewport: WidgetViewport {
            width: side,
            height: side,
            shape: ViewportShape::Round,
        },
        params,
        hand_transition: ClockHandTransition::Animate,
    }
}

/// `params` drawn at `now` into `bucket`'s design frame.
#[must_use]
pub fn at_bucket(bucket: SizeBucket, now: SystemTime, params: Params) -> ViewData {
    let (width, height) = bucket.design_size();
    rectangular(width, height, now, params)
}

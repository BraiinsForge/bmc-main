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

use crate::model::ForecastRange;
use crate::render::common::TEXT_PRIMARY;
use units::units::DegreeCelsius;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const TRACK_H: f32 = 6.0;
const DOT_OUTER_R: f32 = 8.0;
const DOT_INNER_R: f32 = 4.0;

/// A min→max temperature slider: a faint full-width track, a brighter
/// segment spanning the day's range, and (for today) a ringed dot marking
/// the current temperature. Mirrors the deckfeeder `.temp-slider`.
#[must_use]
pub fn forecast_bar(
    width: f32,
    height: f32,
    range: &ForecastRange,
    min: DegreeCelsius,
    max: DegreeCelsius,
    today_marker: Option<DegreeCelsius>,
) -> Node {
    let cy = height / 2.0;
    let track_y = cy - TRACK_H / 2.0;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "f64 fraction (0..=1) safely narrows to f32 canvas coordinate"
    )]
    let x_of = |c: DegreeCelsius| (range.fraction(c) as f32) * width;
    let lo = x_of(min);
    let hi = x_of(max);
    let mut draws = vec![
        Draw::rect(0.0, track_y, width, TRACK_H, TEXT_PRIMARY.with_alpha(0.2)),
        Draw::rect(lo, track_y, hi - lo, TRACK_H, TEXT_PRIMARY.with_alpha(0.5)),
    ];
    if let Some(cur) = today_marker {
        let cx = x_of(cur);
        draws.push(Draw::circle(cx, cy, DOT_OUTER_R, BLACK));
        draws.push(Draw::circle(cx, cy, DOT_INNER_R, TEXT_PRIMARY));
    }
    canvas(props!(width: width, height: height), draws)
}

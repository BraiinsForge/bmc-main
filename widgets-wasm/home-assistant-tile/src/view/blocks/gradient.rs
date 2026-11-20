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

//! A stepper's track painted with colour stops, such as a light's hue or colour temperature,
//! and a cursor where the value is: the colour is the value, so no number is drawn.
//! The cursor is Home Assistant's colour-temperature cursor, coloured as the stepper's buttons.
//!
//! The renderer's gradient has two stops and nothing clips a canvas to rounded corners,
//! so each pair of adjacent stops is a rectangle of its own, between two rounded caps.

use std::f32::consts::{FRAC_PI_2, PI};

use bmc_wasm_sdk::*;

use crate::view::style::{CONTROL_RADIUS, PENDING_LABEL_COLOR, TRACK_COLOR};

/// Points on each rounded corner, ends included.
const CORNER_POINTS: usize = 6;
/// How far a piece reaches under the next, so their antialiased edges leave no seam.
const SEAM: f32 = 1.0;
/// Between the track's ends and the first and last place the cursor's middle stands.
pub const CURSOR_INSET: f32 = CONTROL_RADIUS;
/// Between the cursor's ends and the track's edges, which keeps it inside the rounded ends.
const CURSOR_MARGIN: f32 = 6.0;
const CURSOR_RADIUS: f32 = 4.0;
const GRIP_WIDTH: f32 = 4.0;
/// How far [`hues`] are saturated, 0 being white and 1 the pure colour.
const HUE_SATURATION: f32 = 0.75;
/// As many as Home Assistant samples a light's range at.
const TEMPERATURE_STOPS: usize = 11;
/// Home Assistant's `box-shadow: 0 2px 5px rgba(0, 0, 0, 0.2)`, a CSS blur being twice the sigma.
const CURSOR_SHADOW: DropShadow = DropShadow {
    dx: 0.0,
    dy: 2.0,
    blur: 2.5,
    color: Color::from_rgba(0, 0, 0, 51),
};

/// The track's draws, `width` by `height`: the gradient and the cursor at `share`, where there is one,
/// its grip grey while Home Assistant has yet to confirm the colour.
#[must_use]
pub fn paint(
    stops: &[Color],
    share: Option<f32>,
    (width, height): (f32, f32),
    pending: bool,
) -> Vec<Draw> {
    let mut draws = pieces(stops, (0.0, 0.0), (width, height), CONTROL_RADIUS);
    if let Some(share) = share {
        let grip = if pending {
            PENDING_LABEL_COLOR
        } else {
            color_at(stops, share)
        };
        draws.extend(cursor(cursor_x(share, width), height, grip));
    }
    draws
}

/// The hue circle's primaries and secondaries, red round to red, between which RGB runs straight,
/// paled towards white to set the cursor apart: no longer the very colour the light takes.
#[must_use]
pub fn hues() -> [Color; 7] {
    [0.0, 60.0, 120.0, 180.0, 240.0, 300.0, 360.0]
        .map(|hue| Color::from_hsv(hue, HUE_SATURATION, 1.0))
}

/// A light's colour temperatures from `warmest` to `coolest` kelvin, evenly apart, as Home Assistant draws them.
#[must_use]
pub fn temperatures(warmest: f32, coolest: f32) -> Vec<Color> {
    #[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
    let step = (coolest - warmest) / (TEMPERATURE_STOPS - 1) as f32;
    (0..TEMPERATURE_STOPS)
        .map(|stop| {
            #[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
            let kelvin = warmest + stop as f32 * step;
            temperature_color(kelvin)
        })
        .collect()
}

/// The colour of light at `kelvin`, by Tanner Helland's fit, which the frontend's `temperature2rgb` uses.
#[must_use]
pub fn temperature_color(kelvin: f32) -> Color {
    let hundreds = kelvin / 100.0;
    let red = if hundreds <= 66.0 {
        255.0
    } else {
        329.698_73 * (hundreds - 60.0).powf(-0.133_204_76)
    };
    let green = if hundreds <= 66.0 {
        99.470_8 * hundreds.ln() - 161.119_57
    } else {
        288.122_17 * (hundreds - 60.0).powf(-0.075_514_85)
    };
    let blue = if hundreds >= 66.0 {
        255.0
    } else if hundreds <= 19.0 {
        0.0
    } else {
        138.517_73 * (hundreds - 10.0).ln() - 305.044_8
    };
    Color::from_rgb(channel(red), channel(green), channel(blue))
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to a channel first"
)]
#[must_use]
pub fn channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Where the middle of the cursor at `share` stands across a track `width` wide.
#[must_use]
pub fn cursor_x(share: f32, width: f32) -> f32 {
    CURSOR_INSET + share.clamp(0.0, 1.0) * (width - 2.0 * CURSOR_INSET).max(0.0)
}

/// The share a tap at `x` across a track `width` wide picks: the one whose cursor stands under it.
#[must_use]
pub fn share_at(x: f32, width: f32) -> f32 {
    let span = width - 2.0 * CURSOR_INSET;
    if span > 0.0 {
        ((x - CURSOR_INSET) / span).clamp(0.0, 1.0)
    } else {
        0.5
    }
}

/// The colour at `share` of the way along `stops`, spread evenly from end to end.
#[must_use]
pub fn color_at(stops: &[Color], share: f32) -> Color {
    let Some((&first, rest)) = stops.split_first() else {
        return TRACK_COLOR;
    };
    if rest.is_empty() {
        return first;
    }
    #[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
    let at = share.clamp(0.0, 1.0) * rest.len() as f32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the stops above"
    )]
    let piece = (at.floor() as usize).min(rest.len() - 1);
    #[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
    let within = at - piece as f32;
    stops[piece].mix(stops[piece + 1], within)
}

/// The stops across `(x, y)` to `(x + width, y + height)`, its ends rounded by `radius`.
/// A pill's caps are circles, which the renderer draws rounder than a polygon of the same few pixels.
#[must_use]
pub fn pieces(
    stops: &[Color],
    (x, y): (f32, f32),
    (width, height): (f32, f32),
    radius: f32,
) -> Vec<Draw> {
    let (Some(&first), Some(&last)) = (stops.first(), stops.last()) else {
        return Vec::new();
    };
    let radius = radius.min(height / 2.0).min(width / 2.0);
    let (start, end) = (x + radius, x + width - radius);
    let mut draws = if 2.0 * radius >= height {
        let middle = y + height / 2.0;
        vec![
            Draw::circle(start, middle, radius, first).with_anti_alias(),
            Draw::circle(end, middle, radius, last).with_anti_alias(),
        ]
    } else {
        vec![
            Draw::fill_path(
                outline((x, start + SEAM), (y, y + height), radius, (true, false)),
                first,
                false,
            ),
            Draw::fill_path(
                outline(
                    (end - SEAM, x + width),
                    (y, y + height),
                    radius,
                    (false, true),
                ),
                last,
                false,
            ),
        ]
    };
    if let [only] = stops {
        draws.push(Draw::rect(start, y, end - start, height, *only));
        return draws;
    }
    let count = stops.len() - 1;
    #[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
    let step = (end - start) / count as f32;
    draws.extend(stops.windows(2).enumerate().map(|(index, pair)| {
        #[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
        let left = start + index as f32 * step;
        let reach = if index + 1 == count {
            step
        } else {
            step + SEAM
        };
        Draw::rect(left, y, reach, height, Fill::linear(90.0, pair[0], pair[1]))
    }));
    draws
}

/// A rectangle's outline, clockwise from its top left, with its left or right corners rounded.
fn outline(
    (left, right): (f32, f32),
    (top, bottom): (f32, f32),
    radius: f32,
    (round_left, round_right): (bool, bool),
) -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    let mut corner = |round: bool,
                      (corner_x, corner_y): (f32, f32),
                      (center_x, center_y): (f32, f32),
                      from: f32| {
        if !round {
            points.push((corner_x, corner_y));
            return;
        }
        for step in 0..CORNER_POINTS {
            #[expect(clippy::cast_precision_loss, reason = "a handful of points")]
            let angle = from + FRAC_PI_2 * step as f32 / (CORNER_POINTS - 1) as f32;
            points.push((
                center_x + radius * angle.cos(),
                center_y + radius * angle.sin(),
            ));
        }
    };
    corner(round_left, (left, top), (left + radius, top + radius), PI);
    corner(
        round_right,
        (right, top),
        (right - radius, top + radius),
        -FRAC_PI_2,
    );
    corner(
        round_right,
        (right, bottom),
        (right - radius, bottom - radius),
        0.0,
    );
    corner(
        round_left,
        (left, bottom),
        (left + radius, bottom - radius),
        FRAC_PI_2,
    );
    points
}

/// A bar across the track, a quarter as wide as the track is high, coloured as the buttons,
/// with a grip half its height in its middle; its shadow sets it apart from the gradient.
fn cursor(x: f32, height: f32, grip_color: Color) -> [Draw; 4] {
    let (width, length) = (height / 4.0, height - 2.0 * CURSOR_MARGIN);
    let (grip, middle, round) = (length / 2.0, height / 2.0, GRIP_WIDTH / 2.0);
    let (grip_top, grip_bottom) = (middle - grip / 2.0 + round, middle + grip / 2.0 - round);
    [
        Draw::fill_path(
            outline(
                (x - width / 2.0, x + width / 2.0),
                (CURSOR_MARGIN, CURSOR_MARGIN + length),
                CURSOR_RADIUS,
                (true, true),
            ),
            TRACK_COLOR,
            false,
        )
        .with_drop_shadow(CURSOR_SHADOW),
        // A pill this thin comes out pointed as a polygon; circles cap it round.
        Draw::rect(
            x - round,
            grip_top,
            GRIP_WIDTH,
            grip_bottom - grip_top,
            grip_color,
        ),
        Draw::circle(x, grip_top, round, grip_color).with_anti_alias(),
        Draw::circle(x, grip_bottom, round, grip_color).with_anti_alias(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const RED: Color = Color::from_rgb(255, 0, 0);
    const BLUE: Color = Color::from_rgb(0, 0, 255);

    #[wasm_test]
    fn a_cursor_and_the_tap_under_it_agree() {
        for share in [0.0, 0.25, 0.5, 1.0] {
            let x = cursor_x(share, 300.0);
            assert!((share_at(x, 300.0) - share).abs() < 1e-6, "{share}");
        }
        assert_eq!(share_at(0.0, 300.0), 0.0, "within the rounded end");
        assert_eq!(share_at(300.0, 300.0), 1.0);
    }

    #[wasm_test]
    fn the_colour_at_a_share_lies_between_the_stops_around_it() {
        assert_eq!(color_at(&[RED, BLUE], 0.0), RED);
        assert_eq!(color_at(&[RED, BLUE], 1.0), BLUE);
        assert_eq!(color_at(&[RED], 0.7), RED);
        assert_eq!(color_at(&[RED, BLUE, RED], 0.5), BLUE);
    }

    #[wasm_test]
    fn the_hues_run_red_round_to_red_paled_towards_white() {
        let rgb = |color: Color| (color.red(), color.green(), color.blue());
        let hues = hues();
        assert_eq!(rgb(hues[0]), (255, 63, 63));
        assert_eq!(rgb(hues[4]), (63, 63, 255), "blue");
        assert_eq!(hues[6], hues[0]);
    }

    #[wasm_test]
    fn temperatures_run_from_warm_to_cool() {
        let rgb = |color: Color| (color.red(), color.green(), color.blue());
        let stops = temperatures(2000.0, 6500.0);
        assert_eq!(stops.len(), TEMPERATURE_STOPS);
        assert_eq!(rgb(stops[0]), (255, 137, 14), "2000 K");
        assert_eq!(rgb(stops[10]), (255, 254, 250), "6500 K");
        assert_eq!(
            rgb(temperature_color(10_000.0)),
            (202, 218, 255),
            "above 6600 K, blue"
        );
    }

    #[wasm_test]
    fn a_track_without_a_value_draws_no_cursor() {
        let size = (300.0, 64.0);
        let track = pieces(&[RED, BLUE], (0.0, 0.0), size, CONTROL_RADIUS).len();
        let drawn = |share| paint(&[RED, BLUE], share, size, false).len();
        assert_eq!(drawn(None), track);
        assert!(drawn(Some(0.5)) > track, "a value adds the cursor");
    }

    #[wasm_test]
    fn each_pair_of_stops_is_one_piece_between_two_caps() {
        let count = |stops: &[Color]| pieces(stops, (0.0, 0.0), (300.0, 64.0), 16.0).len();
        assert_eq!(count(&[]), 0);
        assert_eq!(count(&[RED]), 3);
        assert_eq!(count(&[RED, BLUE]), 3);
        assert_eq!(count(&[RED, BLUE, RED, BLUE, RED, BLUE, RED]), 8);
    }

    #[wasm_test]
    fn a_rounded_outline_stays_within_its_rectangle() {
        let points = outline((10.0, 110.0), (0.0, 64.0), 16.0, (true, true));
        assert_eq!(points.len(), 4 * CORNER_POINTS);
        for (x, y) in points {
            assert!((10.0 - 1e-3..=110.0 + 1e-3).contains(&x), "{x}");
            assert!((-1e-3..=64.0 + 1e-3).contains(&y), "{y}");
        }
    }
}

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

//! A light's tile: the card turns it on and off, and its blocks set what the light takes.

use bmc_gallery::prelude::*;
use home_assistant::CallId;
use home_assistant::domain::light::{
    Brightness, ColorTemperature, DEFAULT_KELVIN_RANGE, Dimming, HueSaturation, Look,
};
use home_assistant_tile::fixtures;
use home_assistant_tile::interaction::gesture::{Gesture, OnTap};
use home_assistant_tile::interaction::pending::{Pending, Request, Timing};
use home_assistant_tile::present::light::{BRIGHTNESS, State, Tile};
use home_assistant_tile::present::{Position, Presenter, Reach};
use home_assistant_tile::view::card;
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Light" }

/// What the light takes, besides on and off: `(dims, colour temperature, colour)`.
const KINDS: [((bool, bool, bool), &str); 4] = [
    ((true, true, true), "Colour"),
    ((true, true, false), "White"),
    ((true, false, false), "Dimmer"),
    ((false, false, false), "Switch"),
];
const POSITIONS: [(Position, &str); 3] = [
    (Position::On, "On"),
    (Position::Off, "Off"),
    (Position::Unknown, "Unknown"),
];
const BRIGHTNESSES: [(Option<u8>, &str); 4] = [
    (Some(60), "60%"),
    (Some(5), "5%"),
    (Some(100), "100%"),
    (None, "Unknown"),
];
const READINESS: [(Reach, &str); 3] = [
    (Reach::Ready, "Ready"),
    (Reach::Unavailable, "Unavailable"),
    (Reach::Reconnecting, "Reconnecting"),
];
const SETTINGS: [(OnTap, &str); 2] = [(OnTap::Fires, "Tap"), (OnTap::Nothing, "Neither")];
const NAMES: [(&str, &str); 2] = [
    ("Kitchen", "Short"),
    ("Living room ceiling light by the window", "Long"),
];

fn select<T: Copy>(ctx: &mut SceneCtx, name: &str, options: &[(T, &str)]) -> T {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    options[ctx.select(name, &labels, 0)].0
}

#[scene(default)]
fn light(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Light");
    let (dims, warmth, colours) = select(ctx, "Kind", &KINDS);
    let position = select(ctx, "Position", &POSITIONS);
    let on = position == Position::On;
    let brightness = select(ctx, "Brightness", &BRIGHTNESSES);
    let showing_colour = ctx.toggle("Showing a colour", false);
    let pale = ctx.toggle("Pale colour", false);
    let asked = ctx.toggle("Brightness on its way", false);
    let reach = select(ctx, "Readiness", &READINESS);
    let tap = select(ctx, "Settings", &SETTINGS);
    let name = select(ctx, "Name", &NAMES);
    let card_pressed = ctx.toggle("Card pressed", false);
    let size_labels = fixtures::SIZES.map(|(_, label)| label);
    let size = fixtures::SIZES[ctx.select("Size", &size_labels, fixtures::DEFAULT_SIZE)].0;

    let look = if colours && showing_colour {
        Look::Colour(HueSaturation {
            hue: 210.0,
            saturation: if pale { 30.0 } else { 100.0 },
        })
    } else if warmth && !showing_colour {
        Look::White(ColorTemperature::from_kelvin(2700.0))
    } else {
        Look::Unknown
    };
    let state = State {
        position,
        brightness: brightness
            .filter(|_| on && dims)
            .map(Brightness::from_percent),
        dimming: dims.then_some(Dimming {
            warmth: warmth.then_some(DEFAULT_KELVIN_RANGE),
            colour: colours,
        }),
        look,
    };
    let mut pending = Pending::default();
    if asked {
        pending.submit(
            Request::number(
                BRIGHTNESS,
                80.0,
                home_assistant::domain::light::turn_on_at_percent(Brightness::from_percent(80)),
                Timing::AfterTaps,
            ),
            |_, _| Ok(CallId::from_raw(1)),
        );
    }
    let gesture = Gesture::new(tap, None);
    ctx.wrapping_flow(
        ui,
        [titled("Tile", size, move || {
            let context = fixtures::context(&pending, &gesture, name, (None, reach), size);
            let tile = Tile::new(&state, &context);
            let touches = if card_pressed {
                card::hold()
            } else {
                Touches::default()
            };
            tile.view(&touches)
        })],
    );
}

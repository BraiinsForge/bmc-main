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

//! A switch's tile, and those of the other entities that are on or off, at each size.

use bmc_gallery::prelude::*;
use home_assistant::CallId;
use home_assistant::domain::switch::DeviceClass;
use home_assistant_tile::fixtures;
use home_assistant_tile::interaction::gesture::Gesture;
use home_assistant_tile::interaction::pending::Pending;
use home_assistant_tile::present::switch::{Kind, Position, State, Tile};
use home_assistant_tile::present::{Presenter, Reach};
use home_assistant_tile::view::card;
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Switch" }

const KINDS: [(Kind, &str); 5] = [
    (Kind::Switch, "Switch"),
    (Kind::InputBoolean, "Input boolean"),
    (Kind::Automation, "Automation"),
    (Kind::Siren, "Siren"),
    (Kind::Remote, "Remote"),
];
const POSITIONS: [(Position, &str); 3] = [
    (Position::On, "On"),
    (Position::Off, "Off"),
    (Position::Unknown, "Unknown"),
];
const DEVICE_CLASSES: [(Option<DeviceClass>, &str); 2] =
    [(None, "None"), (Some(DeviceClass::Outlet), "Outlet")];
const READINESS: [(Reach, &str); 2] =
    [(Reach::Ready, "Ready"), (Reach::Unavailable, "Unavailable")];
const NAMES: [(&str, &str); 2] = [
    ("Kettle", "Short"),
    ("Living room floor lamp by the window", "Long"),
];

fn select<T: Copy>(ctx: &mut SceneCtx, name: &str, options: &[(T, &str)]) -> T {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    options[ctx.select(name, &labels, 0)].0
}

fn select_size(ctx: &mut SceneCtx) -> (u32, u32) {
    let labels = fixtures::SIZES.map(|(_, label)| label);
    fixtures::SIZES[ctx.select("Size", &labels, fixtures::DEFAULT_SIZE)].0
}

#[scene(default)]
fn switch(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Switch");
    let kind = select(ctx, "Kind", &KINDS);
    let position = select(ctx, "Position", &POSITIONS);
    let device_class = select(ctx, "Device class", &DEVICE_CLASSES);
    let reach = select(ctx, "Readiness", &READINESS);
    let (tap, hold_ms) = select(ctx, "Settings", &fixtures::SETTINGS);
    let half_held = ctx.toggle("Held half way", false);
    let flipping = ctx.toggle("Flip on its way", false);
    let name = select(ctx, "Name", &NAMES);
    let told = select(ctx, "Line", &fixtures::LINES).over(reach);
    let card_pressed = ctx.toggle("Card pressed", false);
    let size = select_size(ctx);

    let mut gesture = Gesture::new(tap, hold_ms);
    if half_held {
        fixtures::hold_half_way(&mut gesture);
    }
    let state = State {
        position,
        device_class,
    };
    let mut pending = Pending::default();
    if flipping {
        pending.submit(kind.request(position), |_, _| Ok(CallId::from_raw(1)));
    }
    ctx.wrapping_flow(
        ui,
        [titled("Tile", size, move || {
            let tile = Tile::new(
                kind,
                &state,
                &fixtures::context(&pending, &gesture, name, told, size),
            );
            // A hold fills only under a finger, which presses the card.
            let touches = if card_pressed || half_held {
                card::hold()
            } else {
                Touches::default()
            };
            tile.view(&touches)
        })],
    );
}

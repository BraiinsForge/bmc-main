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

//! A button's, a scene's or a script's tile: its one action, under each tap and hold setting.

use bmc_gallery::prelude::*;
use bmc_wasm_sdk::SystemTime;
use home_assistant::CallId;
use home_assistant_tile::fixtures;
use home_assistant_tile::interaction::gesture::Gesture;
use home_assistant_tile::interaction::pending::Pending;
use home_assistant_tile::present::momentary::{Kind, State, Tile};
use home_assistant_tile::present::{Presenter, Reach};
use home_assistant_tile::view::card;
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Momentary" }

const KINDS: [(Kind, &str); 4] = [
    (Kind::Button, "Button"),
    (Kind::Scene, "Scene"),
    (Kind::Script, "Script"),
    (Kind::InputButton, "Input button"),
];
/// Whether a script runs, and whether the tile can act.
const READINESS: [((bool, Reach), &str); 3] = [
    ((false, Reach::Ready), "Ready"),
    ((true, Reach::Ready), "Running"),
    ((false, Reach::Unavailable), "Unavailable"),
];
/// How long ago it was last used, in seconds: the gallery's clock reads 0, so the time is that far before it.
const LAST_USED: [(Option<i64>, &str); 3] = [
    (Some(300), "5 min ago"),
    (None, "Never"),
    (Some(3 * 24 * 3600), "3 days ago"),
];
const NAMES: [(&str, &str); 2] = [
    ("Gate", "Short"),
    (
        "Open the garage gate and switch the driveway lights on",
        "Long",
    ),
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
fn momentary(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Momentary");
    let kind = select(ctx, "Kind", &KINDS);
    let (running, reach) = select(ctx, "Readiness", &READINESS);
    let (tap, hold_ms) = select(ctx, "Settings", &fixtures::SETTINGS);
    let half_held = ctx.toggle("Held half way", false);
    let on_its_way = ctx.toggle("Press on its way", false);
    let last_used = select(ctx, "Last pressed", &LAST_USED);
    let name = select(ctx, "Name", &NAMES);
    let told = select(ctx, "Line", &fixtures::LINES).over(reach);
    let card_pressed = ctx.toggle("Card pressed", false);
    let size = select_size(ctx);

    let mut gesture = Gesture::new(tap, hold_ms);
    if half_held {
        fixtures::hold_half_way(&mut gesture);
    }
    let state = if running {
        State::Running
    } else {
        State::Idle {
            last_used: last_used.map(|ago| SystemTime { unix_secs: -ago }),
        }
    };
    let mut pending = Pending::default();
    if on_its_way {
        pending.submit(kind.request(&state), |_| Some(CallId::from_raw(1)));
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

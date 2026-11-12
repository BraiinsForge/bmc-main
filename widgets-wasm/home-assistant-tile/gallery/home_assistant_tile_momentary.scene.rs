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

use std::num::NonZeroU32;

use bmc_gallery::prelude::*;
use bmc_wasm_sdk::{SystemTime, WidgetSize};
use home_assistant::{CallFailure, CallId, HaError};
use home_assistant_tile::interaction::gesture::{Availability, Gesture, OnTap, Touch};
use home_assistant_tile::interaction::pending::Pending;
use home_assistant_tile::present::momentary::{Kind, State, Tile};
use home_assistant_tile::present::{self, Context, Presenter, Reach, Room};
use home_assistant_tile::view::card;
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Momentary" }

/// The sizes a tile takes on a BMC100: the combined scene's slots, and a fullscreen scene.
const SIZES: [((u32, u32), &str); 4] = [
    ((317, 238), "1x1"),
    ((638, 238), "2x1"),
    ((638, 480), "2x2"),
    ((1280, 480), "Full"),
];
/// 2x1, which the other knobs are captured at.
const DEFAULT_SIZE: usize = 1;

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
const HOLD: Option<NonZeroU32> = NonZeroU32::new(2_000);
const SETTINGS: [((OnTap, Option<NonZeroU32>), &str); 4] = [
    ((OnTap::Fires, None), "Tap"),
    ((OnTap::Nothing, HOLD), "Hold 2 s"),
    ((OnTap::Fires, HOLD), "Tap or hold"),
    ((OnTap::Nothing, None), "Neither"),
];
/// Half of the 2 s hold the settings offer.
const HALF_HOLD_MS: u32 = 1_000;
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
/// A call Home Assistant refused, for the longest notice a failed call shows.
static REFUSED: CallFailure = CallFailure::Rejected(HaError {
    code: String::new(),
    message: String::new(),
});
const LINES: [&str; 4] = ["Status", "Notice", "Refused", "Reconnecting"];
const HELD: Touch = Touch {
    held: true,
    released: false,
};

fn select<T: Copy>(ctx: &mut SceneCtx, name: &str, options: &[(T, &str)]) -> T {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    options[ctx.select(name, &labels, 0)].0
}

fn context<'a>(
    pending: &'a Pending,
    gesture: &'a Gesture,
    name: &'a str,
    (notice, reach): (Option<&'a str>, Reach),
    (width, height): (u32, u32),
) -> Context<'a> {
    Context {
        pending,
        gesture,
        name,
        notice,
        reach,
        room: Room {
            width: px(width),
            height: px(height),
            size: WidgetSize::from_dimensions(width, height).variant,
        },
    }
}

#[scene(default)]
fn momentary(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Momentary");
    let kind = select(ctx, "Kind", &KINDS);
    let (running, reach) = select(ctx, "Readiness", &READINESS);
    let (tap, hold_ms) = select(ctx, "Settings", &SETTINGS);
    let half_held = ctx.toggle("Held half way", false);
    let on_its_way = ctx.toggle("Press on its way", false);
    let last_used = select(ctx, "Last pressed", &LAST_USED);
    let name = select(ctx, "Name", &NAMES);
    let told = match LINES[ctx.select("Line", &LINES, 0)] {
        "Notice" => (Some("Released too early"), reach),
        "Refused" => (Some(present::failure_notice(&REFUSED)), reach),
        "Reconnecting" => (None, Reach::Reconnecting),
        _ => (None, reach),
    };
    let card_pressed = ctx.toggle("Card pressed", false);
    let size_labels = SIZES.map(|(_, label)| label);
    let size = SIZES[ctx.select("Size", &size_labels, DEFAULT_SIZE)].0;

    let mut gesture = Gesture::new(tap, hold_ms);
    if half_held {
        gesture.frame(0, HELD, Availability::Ready);
        gesture.frame(HALF_HOLD_MS, HELD, Availability::Ready);
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
            let tile = Tile::new(kind, &state, &context(&pending, &gesture, name, told, size));
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

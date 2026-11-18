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

//! What the tile shows while it has no entity to draw: its skeleton while the entity loads, or why there is none.

use bmc_gallery::prelude::*;
use home_assistant_tile::fixtures;
use home_assistant_tile::present::momentary::Kind;
use home_assistant_tile::present::{self, Group, Problem, UNNAMED};
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Screens" }

/// The tile's title: a configured name, or the entity ID where none is set.
const NAMES: [&str; 2] = ["Gate", "button.gate"];

fn screen_labels() -> Vec<String> {
    let mut labels = vec!["Loading".to_owned()];
    labels.extend(Problem::ALL.map(|problem| format!("{problem:?}")));
    labels
}

#[scene(default)]
fn screens(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Screens");
    let labels = screen_labels();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let screen = ctx.select("Screen", &labels, 0);
    let name = NAMES[ctx.select("Name", &NAMES, 0)];
    let size_labels = fixtures::SIZES.map(|(_, label)| label);
    let size = fixtures::SIZES[ctx.select("Size", &size_labels, fixtures::DEFAULT_SIZE)].0;
    let room = fixtures::room(size);
    let card = match screen.checked_sub(1) {
        None => present::loading(Group::Momentary(Kind::Button), name, room),
        Some(index) => {
            let problem = Problem::ALL[index];
            let name = if problem == Problem::NoEntity {
                UNNAMED
            } else {
                name
            };
            present::problem(problem, name, room)
        }
    };
    ctx.wrapping_flow(
        ui,
        [titled("Tile", size, move || {
            card.centred(&Touches::default())
        })],
    );
}

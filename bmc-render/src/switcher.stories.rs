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

use crate::prelude::*;

const GRID: Svg = include_svg!("widgets-wasm/fleet-management/assets/icons/dashboard.svg");
const LIST: Svg = include_svg!("widgets-wasm/fleet-management/assets/icons/list.svg");

const GRID_KEY: &str = "switcher::grid";
const LIST_KEY: &str = "switcher::list";

story_meta! { title: "Switcher" }

#[story(default)]
fn interactive(c: &mut StoryCtx) {
    let list_active = c.toggle("List view", false);
    let disabled = c.toggle("Disabled", false);

    // Log tab taps in the Actions panel and let them drive the "List view" knob.
    c.action_with_key("Grid selected", GRID_KEY);
    c.action_with_key("List selected", LIST_KEY);
    c.bind(GRID_KEY, "", list_active.set(false));
    c.bind(LIST_KEY, "", list_active.set(true));

    let tabs = [
        Tab {
            icon: &GRID,
            click_id: GRID_KEY,
        },
        Tab {
            icon: &LIST,
            click_id: LIST_KEY,
        },
    ];

    c.ui.div(
        (240, 72),
        switcher(usize::from(list_active.get()), disabled.get(), &tabs),
    );
}

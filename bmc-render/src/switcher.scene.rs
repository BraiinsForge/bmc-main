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

use bmc_gallery::prelude::*;

const GRID: Svg = include_svg!("widgets-wasm/fleet-management/assets/icons/dashboard.svg");
const LIST: Svg = include_svg!("widgets-wasm/fleet-management/assets/icons/list.svg");

const GRID_KEY: &str = "switcher::grid";
const LIST_KEY: &str = "switcher::list";

scene_meta! { title: "Components / Controls / Switcher" }

#[scene(default)]
fn interactive(ctx: &mut SceneCtx, ui: &mut Ui) {
    let list_active = ctx.toggle("List view", false);
    let disabled = ctx.toggle("Disabled", false);

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

    let fired = ctx.node_stage_input(ui, (240_u32, 72_u32), || {
        switcher(usize::from(list_active), disabled, &tabs)
    });

    // A tab tap drives the knob that selects the tab, so the switcher and the
    // controls panel never disagree about which view is active.
    for (key, active, what) in [
        (GRID_KEY, false, "Grid selected"),
        (LIST_KEY, true, "List selected"),
    ] {
        if fired.clicked(key) {
            ctx.set_toggle("List view", active);
            action(what);
        }
    }
}

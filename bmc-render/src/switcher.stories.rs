// Copyright (C) 2026  Braiins Systems s.r.o.

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

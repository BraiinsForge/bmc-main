// Copyright (C) 2026  Braiins Systems s.r.o.

//! Fleet screen stories, rendered natively (same path as the wasm widget).

use crate::prelude::*;
use fleet_management::screens;

story_meta! { title: "widgets/fleet" }

#[story(default)]
fn dashboard(ctx: &mut StoryCtx) {
    ctx.ui.div(
        Full,
        screens::dashboard::dashboard(&screens::fixtures::sample_dashboard()),
    );
}

#[story]
fn table(ctx: &mut StoryCtx) {
    // Twelve mock models paginate four-per-page; the pager clicks nudge the knob.
    let page = ctx.slider("Page", 0.0, 0.0, 2.0);
    ctx.action_with_key("Page up", screens::table::PAGE_UP_ID);
    ctx.action_with_key("Page down", screens::table::PAGE_DOWN_ID);
    ctx.bind(screens::table::PAGE_UP_ID, "", page.nudge(-1.0));
    ctx.bind(screens::table::PAGE_DOWN_ID, "", page.nudge(1.0));
    ctx.ui.div(
        Full,
        screens::table::table(&screens::fixtures::sample_table_page(page.get_usize())),
    );
}

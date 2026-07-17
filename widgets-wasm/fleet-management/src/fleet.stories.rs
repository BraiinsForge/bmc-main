// Copyright (C) 2026  Braiins Systems s.r.o.

//! Fleet screen stories, rendered natively (same path as the wasm widget).

use crate::prelude::*;
use fleet_management::screens;
use fleet_management::view::{PageTurn, PagerScope, pager_click_id};

story_meta! { title: "widgets/fleet" }

#[story(order = 1, default)]
fn dashboard(ctx: &mut StoryCtx) {
    ctx.ui.div(
        Full,
        screens::dashboard::dashboard_view(&screens::fixtures::sample_dashboard()),
    );
}

#[story(order = 2)]
fn table(ctx: &mut StoryCtx) {
    // Twelve mock models paginate four-per-page; the pager clicks nudge the knob.
    let prev = pager_click_id(PagerScope::Fleet, PageTurn::Prev);
    let next = pager_click_id(PagerScope::Fleet, PageTurn::Next);
    let page = ctx.slider("Page", 0.0, 0.0, 2.0);
    ctx.action_with_key("Page up", prev);
    ctx.action_with_key("Page down", next);
    ctx.bind(prev, "", page.nudge(-1.0));
    ctx.bind(next, "", page.nudge(1.0));
    ctx.ui.div(
        Full,
        screens::table::table_view(&screens::fixtures::sample_table_page(page.get_usize())),
    );
}

#[story(order = 3)]
fn model_detail(ctx: &mut StoryCtx) {
    // Ten device rows span three pages; the pager nudges the knob, Back just logs.
    let prev = pager_click_id(PagerScope::ModelDetail, PageTurn::Prev);
    let next = pager_click_id(PagerScope::ModelDetail, PageTurn::Next);
    let page = ctx.slider("Page", 0.0, 0.0, 2.0);
    ctx.action_with_key("Prev page", prev);
    ctx.action_with_key("Next page", next);
    ctx.action_with_key("Back", "back");
    ctx.bind(prev, "", page.nudge(-1.0));
    ctx.bind(next, "", page.nudge(1.0));
    ctx.ui.div(
        Full,
        screens::model_detail::model_detail_view(&screens::fixtures::sample_model_detail_view(
            page.get_usize(),
        )),
    );
}

#[story(order = 4)]
fn device_detail(ctx: &mut StoryCtx) {
    // Multi-sensor miner: the temp tile shows Avg/Min/Max. Back just logs.
    ctx.action_with_key("Back", "back");
    ctx.ui.div(
        Full,
        screens::device_detail::device_detail_view(&screens::fixtures::sample_device_detail()),
    );
}

#[story(order = 5)]
fn device_detail_single(ctx: &mut StoryCtx) {
    // Single-sensor miner (uBOS): one temp value, no MAC.
    ctx.action_with_key("Back", "back");
    ctx.ui.div(
        Full,
        screens::device_detail::device_detail_view(
            &screens::fixtures::sample_device_detail_single(),
        ),
    );
}

#[story(order = 6)]
fn device_detail_error(ctx: &mut StoryCtx) {
    // Present over mDNS but not answering (a 503 API): the State tile shows the
    // "API error" glyph and the metrics fall back to zero.
    ctx.action_with_key("Back", "back");
    ctx.ui.div(
        Full,
        screens::device_detail::device_detail_view(
            &screens::fixtures::sample_device_detail_error(),
        ),
    );
}

#[story(order = 7)]
fn searching(ctx: &mut StoryCtx) {
    // Empty state before the first miner answers: the indeterminate bar animates.
    ctx.ui.div(Full, screens::searching());
}

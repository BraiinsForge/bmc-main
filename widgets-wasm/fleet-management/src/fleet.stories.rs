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

//! Fleet screen stories, rendered natively (same path as the wasm widget).

use crate::prelude::*;
use fleet_management::screens;
use fleet_management::view::{PageTurn, PagerScope, pager_click_id};

story_meta! { title: "widgets/fleet" }

#[story(order = 1, default)]
fn dashboard(ctx: &mut StoryCtx) {
    // The auth-fail count only surfaces in Fleet Status when non-zero.
    let auth = ctx.int_slider("Auth fails", 0.0, 0.0, 8.0);
    ctx.ui.div(
        Full,
        screens::dashboard::dashboard_view(&screens::fixtures::sample_dashboard(auth.get_usize())),
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

#[story(order = 8)]
fn no_credentials(ctx: &mut StoryCtx) {
    // Found BOS miners but can't authenticate: scan the QR
    // (or join the network and open the link)
    // to add credentials in the Deck web app.
    ctx.ui.div(
        Full,
        screens::no_credentials::no_credentials_view(&screens::fixtures::sample_no_credentials()),
    );
}

#[story(order = 9)]
fn status_tags(ctx: &mut StoryCtx) {
    // Catalog of the inline device-status tags, one per variant.
    ctx.ui.div(Auto, screens::parts::status_tag_catalog());
}

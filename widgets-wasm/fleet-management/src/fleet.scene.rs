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

//! Fleet screen scenes, rendered natively (same path as the wasm widget).

use bmc_gallery::prelude::*;
use fleet_management::screens;
use fleet_management::view::{PageTurn, PagerScope, pager_click_id};

scene_meta! { title: "Widgets / Fleet" }

/// Step the "Page" knob when a pager arrow is hit, and report the turn.
fn turn_page(ctx: &mut SceneCtx, fired: &Fired, scope: PagerScope, page: f32, last: f32) {
    for (turn, step, what) in [
        (PageTurn::Prev, -1.0, "Prev page"),
        (PageTurn::Next, 1.0, "Next page"),
    ] {
        if fired.clicked(pager_click_id(scope, turn)) {
            ctx.set_slider("Page", (page + step).clamp(0.0, last));
            action(what);
        }
    }
}

/// Log a Back tap; the scenes have nowhere to go back to.
fn log_back(fired: &Fired) {
    if fired.clicked("back") {
        action("Back");
    }
}

#[scene(order = 1, default)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole-step slider bounded to 0..=8"
)]
fn dashboard(ctx: &mut SceneCtx, ui: &mut Ui) {
    // The auth-fail count only surfaces in Fleet Status when non-zero.
    let auth = ctx.slider("Auth fails", 0.0, 0.0, 8.0, 1.0) as usize;
    ctx.node_stage(ui, Full, || {
        screens::dashboard::dashboard_view(&screens::fixtures::sample_dashboard(auth))
    });
}

#[scene(order = 2)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole-step page slider bounded to 0..=2"
)]
fn table(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Twelve mock models paginate four-per-page; the pager clicks drive the knob.
    let page = ctx.slider("Page", 0.0, 0.0, 2.0, 1.0);
    let fired = ctx.node_stage_input(ui, Full, || {
        screens::table::table_view(&screens::fixtures::sample_table_page(page as usize))
    });
    turn_page(ctx, &fired, PagerScope::Fleet, page, 2.0);
}

#[scene(order = 3)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole-step page slider bounded to 0..=2"
)]
fn model_detail(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Ten device rows span three pages; the pager drives the knob, Back just logs.
    let page = ctx.slider("Page", 0.0, 0.0, 2.0, 1.0);
    let fired = ctx.node_stage_input(ui, Full, || {
        screens::model_detail::model_detail_view(&screens::fixtures::sample_model_detail_view(
            page as usize,
        ))
    });
    turn_page(ctx, &fired, PagerScope::ModelDetail, page, 2.0);
    log_back(&fired);
}

#[scene(order = 4)]
fn device_detail(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Multi-sensor miner: the temp tile shows Avg/Min/Max. Back just logs.
    let fired = ctx.node_stage_input(ui, Full, || {
        screens::device_detail::device_detail_view(&screens::fixtures::sample_device_detail())
    });
    log_back(&fired);
}

#[scene(order = 5)]
fn device_detail_single(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Single-sensor miner (uBOS): one temp value, no MAC.
    let fired =
        ctx.node_stage_input(ui, Full, || {
            screens::device_detail::device_detail_view(
                &screens::fixtures::sample_device_detail_single(),
            )
        });
    log_back(&fired);
}

#[scene(order = 6)]
fn device_detail_error(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Present over mDNS but not answering (a 503 API): the State tile shows the
    // "API error" glyph and the metrics fall back to zero.
    let fired = ctx.node_stage_input(ui, Full, || {
        screens::device_detail::device_detail_view(&screens::fixtures::sample_device_detail_error())
    });
    log_back(&fired);
}

#[scene(order = 7)]
fn searching(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Empty state before the first miner answers: the indeterminate bar animates.
    ctx.node_stage(ui, Full, screens::searching);
}

#[scene(order = 8)]
fn no_credentials(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Found BOS miners but can't authenticate: scan the QR
    // (or join the network and open the link)
    // to add credentials in the Deck web app.
    ctx.node_stage(ui, Full, || {
        screens::no_credentials::no_credentials_view(&screens::fixtures::sample_no_credentials())
    });
}

#[scene(order = 9)]
fn status_tags(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Catalog of the inline device-status tags, one per variant.
    ctx.node_stage(ui, Auto, screens::parts::status_tag_catalog);
}

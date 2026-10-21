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
use fleet_management::screens::dashboard::DashboardViewData;
use fleet_management::screens::device_detail::DeviceDetailData;
use fleet_management::screens::model_detail::ModelDetailViewData;
use fleet_management::screens::table::TableViewData;
use fleet_management::screens::{self, Layout, bmm101::Bmm101};
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
    // Present over mDNS but not delivering telemetry: the State tile shows
    // why, and the metrics fall back to the unavailable marker.
    let errors = screens::fixtures::sample_device_detail_errors();
    let labels = errors.each_ref().map(|(label, _)| *label);
    let pick = ctx.radio("State", &labels, 1);
    let (_, detail) = errors
        .get(pick)
        .expect("BUG: the radio offers only these states");
    let fired = ctx.node_stage_input(ui, Full, || {
        screens::device_detail::device_detail_view(detail)
    });
    log_back(&fired);
}

#[scene(order = 7)]
fn searching(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Empty state before the first miner answers: the indeterminate bar animates.
    ctx.node_stage(ui, Full, screens::searching);
}

#[scene(order = 8)]
fn status_tags(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Catalog of the inline device-status tags, one per variant.
    ctx.node_stage(ui, Auto, screens::parts::status_tag_catalog);
}

const BMM101: Layout = Layout::Bmm101(Bmm101::NATIVE);
const BMM101_VIEWPORT: (u32, u32) = (Bmm101::DESIGN_WIDTH, Bmm101::DESIGN_HEIGHT);
const BMM100_VIEWPORT: (u32, u32) = (320, 240);

#[scene("BMM101 Dashboard", order = 9)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole-step slider bounded to 0..=8"
)]
fn bmm101_dashboard(ctx: &mut SceneCtx, ui: &mut Ui) {
    let auth = ctx.slider("Auth fails", 0.0, 0.0, 8.0, 1.0) as usize;
    ctx.node_stage(ui, BMM101_VIEWPORT, || {
        BMM101.dashboard(&screens::fixtures::sample_dashboard(auth))
    });
}

#[scene("BMM101 Table", order = 10)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole-step page slider bounded to 0..=2"
)]
fn bmm101_table(ctx: &mut SceneCtx, ui: &mut Ui) {
    let page = ctx.slider("Page", 0.0, 0.0, 2.0, 1.0);
    let fired = ctx.node_stage_input(ui, BMM101_VIEWPORT, || {
        BMM101.table(&screens::fixtures::sample_table_page(page as usize))
    });
    turn_page(ctx, &fired, PagerScope::Fleet, page, 2.0);
}

#[scene("BMM101 Model Detail", order = 11)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole-step page slider bounded to 0..=2"
)]
fn bmm101_model_detail(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Pages two and three carry the unreachable and API-error rows.
    let page = ctx.slider("Page", 0.0, 0.0, 2.0, 1.0);
    let fired = ctx.node_stage_input(ui, BMM101_VIEWPORT, || {
        BMM101.model_detail(&screens::fixtures::sample_model_detail_view(page as usize))
    });
    turn_page(ctx, &fired, PagerScope::ModelDetail, page, 2.0);
    log_back(&fired);
}

#[scene("BMM101 Device Detail", order = 12)]
fn bmm101_device_detail(ctx: &mut SceneCtx, ui: &mut Ui) {
    let errors = screens::fixtures::sample_device_detail_errors();
    let labels: Vec<&str> = ["OK"]
        .into_iter()
        .chain(errors.iter().map(|(label, _)| *label))
        .collect();
    let pick = ctx.radio("State", &labels, 0);
    let detail = match pick.checked_sub(1) {
        None => screens::fixtures::sample_device_detail(),
        Some(at) => {
            errors
                .into_iter()
                .nth(at)
                .expect("BUG: the radio offers only these states")
                .1
        }
    };
    let fired = ctx.node_stage_input(ui, BMM101_VIEWPORT, || BMM101.device_detail(&detail));
    log_back(&fired);
}

#[scene("BMM101 Searching", order = 13)]
fn bmm101_searching(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.node_stage(ui, BMM101_VIEWPORT, || BMM101.searching());
}

const LONG_FLEET: &str = "Dominika's Mining Rig in the Basement Under the Stairs";
const LONG_MODEL: &str = "Braiins Mini Miner BMM 101 Rev. B";
const LONG_HOST: &str = "bmm-living-room-shelf-left-0123456789";

#[scene("BMM101 Long Names", order = 14)]
fn bmm101_long_names(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Every title and leading cell at a length the panel must cut short.
    let mut dashboard = screens::fixtures::sample_dashboard(3);
    LONG_FLEET.clone_into(&mut dashboard.title);
    let mut table = screens::fixtures::sample_table_page(0);
    LONG_FLEET.clone_into(&mut table.title);
    LONG_MODEL.clone_into(&mut table.rows[0].name);
    let mut model_detail = screens::fixtures::sample_model_detail_view(0);
    LONG_MODEL.clone_into(&mut model_detail.title);
    LONG_HOST.clone_into(&mut model_detail.rows[0].hostname);
    let mut device = screens::fixtures::sample_device_detail();
    LONG_MODEL.clone_into(&mut device.model);
    LONG_HOST.clone_into(&mut device.hostname);
    level_stages(
        ctx,
        ui,
        BMM101_VIEWPORT,
        Levels {
            dashboard,
            table,
            model_detail,
            device,
        },
    );
}

/// The 320×240 panel has no design of its own: it draws BMM101's frame, shrunk to fit.
#[scene("BMM100", order = 15)]
fn bmm100(ctx: &mut SceneCtx, ui: &mut Ui) {
    level_stages(
        ctx,
        ui,
        BMM100_VIEWPORT,
        Levels {
            dashboard: screens::fixtures::sample_dashboard(0),
            table: screens::fixtures::sample_table_page(0),
            model_detail: screens::fixtures::sample_model_detail_view(1),
            device: screens::fixtures::sample_device_detail(),
        },
    );
}

type StageBuild = Box<dyn FnOnce() -> Node>;

struct Levels {
    dashboard: DashboardViewData,
    table: TableViewData,
    model_detail: ModelDetailViewData,
    device: DeviceDetailData,
}

/// One stage per level, side by side, in the layout the viewport takes.
fn level_stages(ctx: &mut SceneCtx, ui: &mut Ui, viewport: (u32, u32), levels: Levels) {
    let layout = Layout::of(viewport.0, viewport.1);
    let Levels {
        dashboard,
        table,
        model_detail,
        device,
    } = levels;
    let stages: [(&str, StageBuild); 4] = [
        ("Dashboard", Box::new(move || layout.dashboard(&dashboard))),
        ("Table", Box::new(move || layout.table(&table))),
        (
            "Model Detail",
            Box::new(move || layout.model_detail(&model_detail)),
        ),
        (
            "Device Detail",
            Box::new(move || layout.device_detail(&device)),
        ),
    ];
    ctx.wrapping_flow(
        ui,
        stages
            .into_iter()
            .map(|(label, build)| titled(label, viewport, build)),
    );
}

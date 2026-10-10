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
//! Each device's scenes lay every case out side by side,
//! so a review is one scroll, not a knob hunt.

use bmc_gallery::prelude::*;
use bmc_wasm_sdk::types;
use fleet_management::screens::table::TableViewData;
use fleet_management::screens::{self, Layout, bmm101::Bmm101};

scene_meta! { title: "Widgets / Fleet" }

const DECK_VIEWPORT: (u32, u32) = (1_280, 480);
const BMM101_VIEWPORT: (u32, u32) = (Bmm101::DESIGN_WIDTH, Bmm101::DESIGN_HEIGHT);
const BMM100_VIEWPORT: (u32, u32) = (320, 240);

#[scene("Deck", order = 1, default)]
fn deck(ctx: &mut SceneCtx, ui: &mut Ui) {
    flow(ctx, ui, levels(DECK_VIEWPORT));
}

#[scene("Deck Extremes", order = 2)]
fn deck_extremes(ctx: &mut SceneCtx, ui: &mut Ui) {
    flow(ctx, ui, extremes(DECK_VIEWPORT));
}

#[scene("BMM101", order = 3)]
fn bmm101(ctx: &mut SceneCtx, ui: &mut Ui) {
    flow(ctx, ui, levels(BMM101_VIEWPORT));
}

#[scene("BMM101 Extremes", order = 4)]
fn bmm101_extremes(ctx: &mut SceneCtx, ui: &mut Ui) {
    flow(ctx, ui, extremes(BMM101_VIEWPORT));
}

/// The 320×240 panel has no design of its own: it draws BMM101's frame, shrunk to fit.
#[scene("BMM100", order = 5)]
fn bmm100(ctx: &mut SceneCtx, ui: &mut Ui) {
    flow(ctx, ui, levels(BMM100_VIEWPORT));
}

const FLEET_STATUS_CASES: [(&str, [usize; 3], usize); 6] = [
    ("Zeros", [0, 0, 0], 0),
    ("One digit", [1, 0, 1], 0),
    ("Typical", [43, 4, 7], 0),
    ("Three digits", [143, 12, 107], 0),
    ("Four digits", [1_043, 104, 1_007], 0),
    ("Auth failures", [43, 4, 7], 3),
];

/// The dashboard at every Fleet Status case, one device after another.
#[scene("Fleet Status Cases", order = 6)]
fn fleet_status_cases(ctx: &mut SceneCtx, ui: &mut Ui) {
    let mut stages = Vec::new();
    for viewport in [DECK_VIEWPORT, BMM101_VIEWPORT, BMM100_VIEWPORT] {
        for (label, [ok, degraded, off], auth) in FLEET_STATUS_CASES {
            let mut dashboard = screens::fixtures::sample_dashboard(auth);
            (dashboard.ok, dashboard.degraded, dashboard.off) = (ok, degraded, off);
            stages.push(stage(label, viewport, move |l| l.dashboard(&dashboard)));
        }
    }
    flow(ctx, ui, stages);
}

#[scene(order = 7)]
fn status_tags(ctx: &mut SceneCtx, ui: &mut Ui) {
    // Catalog of the inline device-status tags, one per variant.
    ctx.node_stage(ui, Auto, screens::parts::status_tag_catalog);
}

type Stage = (&'static str, (u32, u32), Box<dyn FnOnce() -> Node>);

fn stage(
    label: &'static str,
    viewport: (u32, u32),
    build: impl FnOnce(Layout) -> Node + 'static,
) -> Stage {
    let layout = Layout::of(viewport.0, viewport.1);
    (label, viewport, Box::new(move || build(layout)))
}

fn flow(ctx: &mut SceneCtx, ui: &mut Ui, stages: Vec<Stage>) {
    ctx.wrapping_flow(
        ui,
        stages
            .into_iter()
            .map(|(label, viewport, build)| titled(label, viewport, build)),
    );
}

/// Every level and the states that change what it shows, in navigation order.
fn levels(viewport: (u32, u32)) -> Vec<Stage> {
    let mut stages = vec![
        stage("Dashboard", viewport, |l| {
            l.dashboard(&screens::fixtures::sample_dashboard(0))
        }),
        stage("Auth failures", viewport, |l| {
            l.dashboard(&screens::fixtures::sample_dashboard(3))
        }),
        stage("Searching", viewport, Layout::searching),
        stage("Models, page 1", viewport, |l| {
            l.table(&screens::fixtures::sample_table_page(0))
        }),
        stage("Models, page 3", viewport, |l| {
            l.table(&screens::fixtures::sample_table_page(2))
        }),
        stage("Devices, page 1", viewport, |l| {
            l.model_detail(&screens::fixtures::sample_model_detail_view(0))
        }),
        stage("Devices, unreachable", viewport, |l| {
            l.model_detail(&screens::fixtures::sample_model_detail_view(1))
        }),
        stage("Devices, API error", viewport, |l| {
            l.model_detail(&screens::fixtures::sample_model_detail_view(2))
        }),
        stage("Devices, all errors", viewport, |l| {
            l.model_detail(&screens::fixtures::sample_model_detail_all_errors())
        }),
        stage("Device", viewport, |l| {
            l.device_detail(&screens::fixtures::sample_device_detail())
        }),
        stage("Device, one sensor", viewport, |l| {
            l.device_detail(&screens::fixtures::sample_device_detail_single())
        }),
    ];
    for (label, detail) in screens::fixtures::sample_device_detail_errors() {
        stages.push(stage(label, viewport, move |l| l.device_detail(&detail)));
    }
    stages.extend(long_titles(viewport));
    stages
}

const SHORT_NAMES: [&str; 4] = ["S21", "BMM", "T21", "S19"];
const LONG_NAMES: [&str; 4] = [
    "Braiins Mini Miner BMM 101 Rev. B",
    "Braiins Forge Miner x4 in the Garage",
    "AxeOS NerdQAxe++ on the Kitchen Shelf",
    "Antminer S21 Hydro Behind the Boiler",
];
const LONG_FLEET: &str = "Dominika's Mining Rig in the Basement Under the Stairs";
const LONG_HOST: &str = "bmm-living-room-shelf-left-0123456789";

/// Every title at a length the panel must cut short, the fleet's counts at three digits.
fn long_titles(viewport: (u32, u32)) -> [Stage; 3] {
    let mut dashboard = screens::fixtures::sample_dashboard(3);
    LONG_FLEET.clone_into(&mut dashboard.title);
    (dashboard.ok, dashboard.degraded, dashboard.off) = (143, 12, 107);
    let mut devices = screens::fixtures::sample_model_detail_view(0);
    LONG_NAMES[0].clone_into(&mut devices.title);
    LONG_HOST.clone_into(&mut devices.rows[0].hostname);
    let mut device = screens::fixtures::sample_device_detail();
    LONG_NAMES[0].clone_into(&mut device.model);
    LONG_HOST.clone_into(&mut device.hostname);
    [
        stage("Long fleet, big counts", viewport, move |l| {
            l.dashboard(&dashboard)
        }),
        stage("Long model and host", viewport, move |l| {
            l.model_detail(&devices)
        }),
        stage("Long device title", viewport, move |l| {
            l.device_detail(&device)
        }),
    ]
}

#[derive(Clone, Copy)]
enum Names {
    Short,
    Mixed,
    Long,
}

#[derive(Clone, Copy)]
enum Rate {
    Unavailable,
    Short,
    Wide,
}

/// The model table with every row at `digits`-digit counts,
/// the given names and hashrate, cut to `rows`.
fn extreme_table(rows: usize, digits: u32, names: Names, rate: Rate) -> TableViewData {
    let mut table = screens::fixtures::sample_table_page(0);
    table.rows.truncate(rows);
    table.page_count = 1;
    let (widest, smallest) = (10_usize.pow(digits) - 1, 10_usize.pow(digits - 1));
    for (i, row) in table.rows.iter_mut().enumerate() {
        (row.ok, row.degraded, row.off) = (widest - i, smallest, widest - 2 * i);
        match names {
            Names::Short => SHORT_NAMES[i].clone_into(&mut row.name),
            Names::Long => LONG_NAMES[i].clone_into(&mut row.name),
            Names::Mixed => {}
        }
        row.hashrate = match rate {
            Rate::Unavailable => None,
            Rate::Short => Some(types::Hashrate::from_terahashes_per_second(0.83)),
            Rate::Wide => Some(types::Hashrate::from_terahashes_per_second(1_234.0)),
        };
    }
    table
}

/// The model table alone, at one size, so the cases line up:
/// each stretches or squeezes a column.
fn extremes(viewport: (u32, u32)) -> Vec<Stage> {
    let table = |label, table: TableViewData| stage(label, viewport, move |l| l.table(&table));
    vec![
        table("Typical", extreme_table(4, 2, Names::Mixed, Rate::Short)),
        table(
            "Three digits",
            extreme_table(4, 3, Names::Mixed, Rate::Short),
        ),
        table("Widest", extreme_table(4, 4, Names::Long, Rate::Wide)),
        table(
            "Narrowest",
            extreme_table(4, 1, Names::Short, Rate::Unavailable),
        ),
        table("One row", extreme_table(1, 2, Names::Mixed, Rate::Short)),
    ]
}

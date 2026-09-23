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

//! Every state of the forecast at every design size,
//! and on BMM100, which has no bucket of its own.

use bmc_gallery::prelude::*;
use weather::model::SizeBucket;
use weather::screens::fixtures::{self, StateFixture};
use weather::screens::weather_view;
use weather::{Params, TimeZone};

scene_meta! { title: "Widgets / Weather" }

const BUCKETS: [(SizeBucket, &str); 5] = [
    (SizeBucket::Full, "Fullscreen"),
    (SizeBucket::Large, "Large"),
    (SizeBucket::Medium, "Medium"),
    (SizeBucket::Small, "Small"),
    (SizeBucket::Bmm101, "BMM101"),
];

const BMM100_VIEWPORT: (u32, u32) = (320, 240);

const STATES: [(&str, StateFixture); 8] = [
    ("Healthy", fixtures::healthy),
    ("Loading", fixtures::loading),
    ("Failed", fixtures::failed),
    ("Stale", fixtures::stale),
    ("Location Not Found", fixtures::bad_location),
    ("No Location", fixtures::no_location),
    ("Frost", fixtures::frost),
    ("Long Location", fixtures::long_location),
];

fn only_size(ctx: &mut SceneCtx) -> Option<SizeBucket> {
    let labels: Vec<&str> = ["All"]
        .into_iter()
        .chain(BUCKETS.iter().map(|(_, label)| *label))
        .collect();
    let choice = ctx.select("Size", &labels, 0);
    choice.checked_sub(1).map(|at| BUCKETS[at].0)
}

/// The widget's own params, as a knob group.
fn widget_params(ctx: &mut SceneCtx) -> Params {
    ctx.group("Widget settings");
    let labels: Vec<&str> = TimeZone::ALL
        .iter()
        .map(|zone| zone.as_manifest_label())
        .collect();
    let zone = ctx.select("Time zone", &labels, 0);
    Params {
        time_zone: TimeZone::ALL[zone],
        ..fixtures::default_params()
    }
}

fn pick_state(ctx: &mut SceneCtx) -> StateFixture {
    let labels: Vec<&str> = STATES.iter().map(|(label, _)| *label).collect();
    STATES[ctx.select("State", &labels, 0)].1
}

/// What `matrix_with` measures its columns from: each bucket's design size.
fn matrix_sizes(buckets: &[(SizeBucket, &str)]) -> Vec<egui::Vec2> {
    buckets
        .iter()
        .map(|(bucket, _)| {
            let (width, height) = bucket.design_size();
            let side = |px: u32| {
                f32::from(u16::try_from(px).expect("BUG: a design frame is a few hundred pixels"))
            };
            egui::vec2(side(width), side(height))
        })
        .collect()
}

fn bucket_stage(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    (bucket, label): (SizeBucket, &str),
    state: StateFixture,
    params: &Params,
) {
    // A grid counts every widget as a column, so the heading rides with its
    // stage as one block.
    ui.vertical(|ui| {
        ui.heading(label);
        let view = state(fixtures::at_bucket(bucket), params.clone());
        ctx.node_stage(ui, bucket.design_size(), move || weather_view(&view));
    });
}

/// One stage per bucket, each at its design size, in as many columns as fit.
/// Fullscreen goes on its own: the grid measures its columns from the widest
/// stage, and 1280 would hold the rest to one column.
fn state_stages(ctx: &mut SceneCtx, ui: &mut Ui, state: StateFixture) {
    let only = only_size(ctx);
    let params = widget_params(ctx);
    system_settings(ctx);
    let (fullscreen, rest): (Vec<_>, Vec<_>) = BUCKETS
        .into_iter()
        .filter(|(bucket, _)| only.is_none_or(|wanted| wanted == *bucket))
        .partition(|(bucket, _)| *bucket == SizeBucket::Full);
    for stage in fullscreen {
        bucket_stage(ctx, ui, stage, state, &params);
    }
    ctx.matrix_with(ui, &matrix_sizes(&rest), |ctx, ui, at| {
        bucket_stage(ctx, ui, rest[at], state, &params);
    });
}

#[scene(default)]
fn healthy(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::healthy);
}

#[scene]
fn loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::loading);
}

#[scene]
fn failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::failed);
}

#[scene]
fn stale(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::stale);
}

#[scene]
fn location_not_found(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::bad_location);
}

#[scene]
fn no_location(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::no_location);
}

#[scene]
fn frost(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.label("Every temperature 25 °C lower: the widest the figures get.");
    state_stages(ctx, ui, fixtures::frost);
}

#[scene]
fn long_location(ctx: &mut SceneCtx, ui: &mut Ui) {
    state_stages(ctx, ui, fixtures::long_location);
}

/// The 320×240 panel, which has no bucket: it draws the Small layout.
#[scene("BMM100")]
fn bmm100(ctx: &mut SceneCtx, ui: &mut Ui) {
    let state = pick_state(ctx);
    let params = widget_params(ctx);
    system_settings(ctx);
    let (width, height) = BMM100_VIEWPORT;
    let view = state(fixtures::rectangular(width, height), params);
    ctx.node_stage(ui, BMM100_VIEWPORT, move || weather_view(&view));
}

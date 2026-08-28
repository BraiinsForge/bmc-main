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

use bitcoin_mining_data::model::{SizeBucket, size_bucket};
use bitcoin_mining_data::screens::{bitcoin_mining_view, fixtures};
use bmc_gallery::prelude::*;

scene_meta! { title: "Widgets / Bitcoin Mining Data" }

const BUCKETS: [(SizeBucket, &str); 4] = [
    (SizeBucket::Full, "Fullscreen"),
    (SizeBucket::Large, "Large"),
    (SizeBucket::Medium, "Medium"),
    (SizeBucket::Small, "Small"),
];

const DEVICE_VIEWPORTS: [(u32, u32, &str); 2] = [(320, 240, "BMM100"), (480, 320, "BMM101")];

fn only_size(ctx: &mut SceneCtx) -> Option<SizeBucket> {
    match ctx.select(
        "Size",
        &["All", "Fullscreen", "Large", "Medium", "Small"],
        0,
    ) {
        1 => Some(SizeBucket::Full),
        2 => Some(SizeBucket::Large),
        3 => Some(SizeBucket::Medium),
        4 => Some(SizeBucket::Small),
        _ => None,
    }
}

fn size_stages<Build: FnOnce() -> Node>(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    mut view: impl FnMut(SizeBucket) -> Build,
) {
    let only = only_size(ctx);
    system_settings(ctx);
    for (bucket, label) in BUCKETS {
        if only.is_some_and(|wanted| wanted != bucket) {
            continue;
        }
        ui.heading(label);
        let build = view(bucket);
        ctx.node_stage(ui, bucket.design_size(), build);
    }
}

#[scene]
fn supported_devices(ctx: &mut SceneCtx, ui: &mut Ui) {
    let selected = ctx.select("Viewport", &["All", "BMM100", "BMM101"], 0);
    system_settings(ctx);
    for (index, (width, height, label)) in DEVICE_VIEWPORTS.into_iter().enumerate() {
        if selected != 0 && selected != index + 1 {
            continue;
        }
        ui.heading(label);
        let bucket = size_bucket(width, height);
        ctx.node_stage(ui, (width, height), move || {
            bitcoin_mining_view(&fixtures::healthy(bucket))
        });
    }
}

#[scene(default)]
fn healthy(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::healthy(bucket))
    });
}

#[scene]
fn loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::loading(bucket))
    });
}

#[scene]
fn failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::failed(bucket))
    });
}

#[scene]
fn stale(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::stale(bucket))
    });
}

#[scene]
fn rate_limited(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::rate_limited(bucket))
    });
}

#[scene]
fn extremes(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::extremes(bucket))
    });
}

#[scene]
fn unit_rollover(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.label("Values just below unit boundaries must promote when rounding would display 1000 of the lower unit.");
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::unit_rollover(bucket))
    });
}

#[scene]
fn flat_history(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.label("Multi-sample flat histories must stay vertically centered.");
    size_stages(ctx, ui, |bucket| {
        move || bitcoin_mining_view(&fixtures::flat_history(bucket))
    });
}

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

//! Braiins Pool screen scenes, rendered natively over fixture data.
//! Every scene shows all four design sizes as stacked stages.

use bmc_gallery::prelude::*;
use braiins_pool::model::SizeBucket;
use braiins_pool::screens::{big_chart, fixtures, overview};

scene_meta! { title: "Widgets / Braiins Pool" }

const BUCKETS: [(SizeBucket, &str); 4] = [
    (SizeBucket::Full, "Fullscreen"),
    (SizeBucket::Large, "Large"),
    (SizeBucket::Medium, "Medium"),
    (SizeBucket::Small, "Small"),
];

/// Each design size on its own stage, at the width and height the fixture
/// reports rather than a preset, so the frame is the one the design specifies.
///
/// The callback hands back the frame plus a closure that *builds* the tree,
/// run inside the stage where the asset registrars are live:
/// these screens draw SVG icons, and building above the stage
/// registers them against nothing.
fn size_stages<Build: FnOnce() -> Node>(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    mut view: impl FnMut(SizeBucket) -> (f32, f32, Build),
) {
    for (bucket, label) in BUCKETS {
        let (width, height, build) = view(bucket);
        ui.heading(label);
        ctx.node_stage(ui, (width, height), build);
    }
}

/// How many decades the hashrate history climbs from its 5 TH/s baseline
/// within the chart window (0 = flat single-miner chart, ~5 = the design's
/// few-hundred-PH/s scale, 9 = ends in ZH/s), stress-testing the axis when
/// one range crosses many SI prefixes.
fn spread_knob(ctx: &mut SceneCtx) -> f64 {
    f64::from(ctx.slider("Hashrate climb (decades)", 5.0, 0.0, 9.0, 0.1))
}

#[scene(default)]
fn overview(ctx: &mut SceneCtx, ui: &mut Ui) {
    let worker_states = ctx.toggle("Worker states", true);
    let spread = spread_knob(ctx);
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_overview(bucket, worker_states, spread);
        (view.width, view.height, move || {
            overview::overview_view(&view)
        })
    });
}

#[scene]
fn big_chart(ctx: &mut SceneCtx, ui: &mut Ui) {
    let worker_states = ctx.toggle("Worker states", true);
    let spread = spread_knob(ctx);
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_big_chart(bucket, worker_states, spread);
        (view.width, view.height, move || {
            big_chart::big_chart_view(&view)
        })
    });
}

#[scene]
fn overview_unbound(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_overview_unbound(bucket);
        (view.width, view.height, move || {
            overview::overview_view(&view)
        })
    });
}

#[scene]
fn overview_loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_overview_loading(bucket);
        (view.width, view.height, move || {
            overview::overview_view(&view)
        })
    });
}

#[scene]
fn overview_empty(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_overview_empty(bucket);
        (view.width, view.height, move || {
            overview::overview_view(&view)
        })
    });
}

#[scene]
fn overview_failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_overview_failed(bucket);
        (view.width, view.height, move || {
            overview::overview_view(&view)
        })
    });
}

#[scene]
fn overview_denied(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_overview_denied(bucket);
        (view.width, view.height, move || {
            overview::overview_view(&view)
        })
    });
}

#[scene]
fn big_chart_unbound(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_big_chart_unbound(bucket);
        (view.width, view.height, move || {
            big_chart::big_chart_view(&view)
        })
    });
}

#[scene]
fn big_chart_loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_big_chart_loading(bucket);
        (view.width, view.height, move || {
            big_chart::big_chart_view(&view)
        })
    });
}

#[scene]
fn big_chart_empty(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_big_chart_empty(bucket);
        (view.width, view.height, move || {
            big_chart::big_chart_view(&view)
        })
    });
}

#[scene]
fn big_chart_failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_big_chart_failed(bucket);
        (view.width, view.height, move || {
            big_chart::big_chart_view(&view)
        })
    });
}

#[scene]
fn big_chart_denied(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        let view = fixtures::sample_big_chart_denied(bucket);
        (view.width, view.height, move || {
            big_chart::big_chart_view(&view)
        })
    });
}

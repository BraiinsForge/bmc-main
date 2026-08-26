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
//! Every scene shows each device frame as a stacked stage.

use bmc_gallery::prelude::*;
use braiins_pool::model::{SizeBucket, size_bucket};
use braiins_pool::screens::big_chart::BigChartViewData;
use braiins_pool::screens::overview::OverviewViewData;
use braiins_pool::screens::{big_chart, fixtures, overview};

scene_meta! { title: "Widgets / Braiins Pool" }

/// The device frames this widget is staged in: every one the gallery
/// knows of, less the round face, which the manifest does not admit.
fn viewports() -> impl Iterator<Item = DeviceViewport> {
    DEVICE_VIEWPORTS
        .into_iter()
        .filter(|viewport| !viewport.size.is_round())
}

/// Which viewport to stage, as an index into [`viewports`],
/// `None` for every one of them.
///
/// Stacking them all is what a scene is worth looking at for,
/// so that is the default; a capture recipe pins one and must —
/// six stages stacked overrun the renderer's texture bound.
fn only_size(ctx: &mut SceneCtx) -> Option<usize> {
    let mut labels = vec!["All"];
    labels.extend(viewports().map(|viewport| viewport.label));
    ctx.select("Size", &labels, 0).checked_sub(1)
}

/// Each device frame on its own stage, its size handed to the callback:
/// these screens bake chart geometry into canvas draw lists at build time,
/// so a view's own width and height have to be the stage's.
///
/// The callback hands back a closure that *builds* the tree,
/// run inside the stage where the asset registrars are live:
/// these screens draw SVG icons, and building above the stage
/// registers them against nothing.
fn size_stages<Build: FnOnce() -> Node>(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    mut view: impl FnMut(SizeBucket, f32, f32) -> Build,
) {
    let only = only_size(ctx);
    for (index, viewport) in viewports().enumerate() {
        if only.is_some_and(|wanted| wanted != index) {
            continue;
        }
        let (width, height) = viewport.pixels();
        let build = view(
            size_bucket(width, height),
            viewport.size.layout_width(),
            viewport.size.layout_height(),
        );
        ui.heading(viewport.label);
        ctx.node_stage(ui, viewport.size, build);
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
    size_stages(ctx, ui, |bucket, width, height| {
        let view = OverviewViewData {
            width,
            height,
            ..fixtures::sample_overview(bucket, worker_states, spread)
        };
        move || overview::overview_view(&view)
    });
}

#[scene]
fn big_chart(ctx: &mut SceneCtx, ui: &mut Ui) {
    let worker_states = ctx.toggle("Worker states", true);
    let spread = spread_knob(ctx);
    size_stages(ctx, ui, |bucket, width, height| {
        let view = BigChartViewData {
            width,
            height,
            ..fixtures::sample_big_chart(bucket, worker_states, spread)
        };
        move || big_chart::big_chart_view(&view)
    });
}

#[scene]
fn overview_unbound(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = OverviewViewData {
            width,
            height,
            ..fixtures::sample_overview_unbound(bucket)
        };
        move || overview::overview_view(&view)
    });
}

#[scene]
fn overview_loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = OverviewViewData {
            width,
            height,
            ..fixtures::sample_overview_loading(bucket)
        };
        move || overview::overview_view(&view)
    });
}

#[scene]
fn overview_empty(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = OverviewViewData {
            width,
            height,
            ..fixtures::sample_overview_empty(bucket)
        };
        move || overview::overview_view(&view)
    });
}

#[scene]
fn overview_failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = OverviewViewData {
            width,
            height,
            ..fixtures::sample_overview_failed(bucket)
        };
        move || overview::overview_view(&view)
    });
}

#[scene]
fn overview_denied(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = OverviewViewData {
            width,
            height,
            ..fixtures::sample_overview_denied(bucket)
        };
        move || overview::overview_view(&view)
    });
}

#[scene]
fn big_chart_unbound(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = BigChartViewData {
            width,
            height,
            ..fixtures::sample_big_chart_unbound(bucket)
        };
        move || big_chart::big_chart_view(&view)
    });
}

#[scene]
fn big_chart_loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = BigChartViewData {
            width,
            height,
            ..fixtures::sample_big_chart_loading(bucket)
        };
        move || big_chart::big_chart_view(&view)
    });
}

#[scene]
fn big_chart_empty(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = BigChartViewData {
            width,
            height,
            ..fixtures::sample_big_chart_empty(bucket)
        };
        move || big_chart::big_chart_view(&view)
    });
}

#[scene]
fn big_chart_failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = BigChartViewData {
            width,
            height,
            ..fixtures::sample_big_chart_failed(bucket)
        };
        move || big_chart::big_chart_view(&view)
    });
}

#[scene]
fn big_chart_denied(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket, width, height| {
        let view = BigChartViewData {
            width,
            height,
            ..fixtures::sample_big_chart_denied(bucket)
        };
        move || big_chart::big_chart_view(&view)
    });
}

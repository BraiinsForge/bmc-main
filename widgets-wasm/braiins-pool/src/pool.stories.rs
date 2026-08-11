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

//! Braiins Pool screen stories, rendered natively over fixture data.
//! Every story shows all four design sizes as stacked frames.

use braiins_pool::model::SizeBucket;
use braiins_pool::screens::{big_chart, fixtures, overview};

use crate::prelude::*;

story_meta! { title: "widgets/braiins-pool" }

const BUCKETS: [(SizeBucket, &str); 4] = [
    (SizeBucket::Full, "Fullscreen"),
    (SizeBucket::Large, "Large"),
    (SizeBucket::Medium, "Medium"),
    (SizeBucket::Small, "Small"),
];

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "design sizes are small positive integers"
)]
fn design_frame(width: f32, height: f32) -> FrameSize {
    FrameSize::Custom(width as usize, DivHeight::Px(height as usize))
}

fn size_frames(ctx: &mut StoryCtx, mut view: impl FnMut(SizeBucket) -> (Node, f32, f32)) {
    for (bucket, label) in BUCKETS {
        let (node, width, height) = view(bucket);
        ctx.ui.header(label, "");
        ctx.ui.div(design_frame(width, height), node);
    }
}

/// How many decades the hashrate history climbs from its 5 TH/s baseline
/// within the chart window (0 = flat single-miner chart, ~5 = the design's
/// few-hundred-PH/s scale, 9 = ends in ZH/s), stress-testing the axis when
/// one range crosses many SI prefixes.
fn spread_knob(ctx: &mut StoryCtx) -> f64 {
    f64::from(ctx.slider("Hashrate climb (decades)", 5.0, 0.0, 9.0).get())
}

#[story(default)]
fn overview(ctx: &mut StoryCtx) {
    let worker_states = ctx.toggle("Worker states", true).get();
    let spread = spread_knob(ctx);
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_overview(bucket, worker_states, spread);
        (overview::overview_view(&view), view.width, view.height)
    });
}

#[story]
fn big_chart(ctx: &mut StoryCtx) {
    let worker_states = ctx.toggle("Worker states", true).get();
    let spread = spread_knob(ctx);
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_big_chart(bucket, worker_states, spread);
        (big_chart::big_chart_view(&view), view.width, view.height)
    });
}

#[story]
fn overview_unbound(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_overview_unbound(bucket);
        (overview::overview_view(&view), view.width, view.height)
    });
}

#[story]
fn overview_loading(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_overview_loading(bucket);
        (overview::overview_view(&view), view.width, view.height)
    });
}

#[story]
fn overview_empty(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_overview_empty(bucket);
        (overview::overview_view(&view), view.width, view.height)
    });
}

#[story]
fn overview_failed(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_overview_failed(bucket);
        (overview::overview_view(&view), view.width, view.height)
    });
}

#[story]
fn overview_denied(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_overview_denied(bucket);
        (overview::overview_view(&view), view.width, view.height)
    });
}

#[story]
fn big_chart_unbound(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_big_chart_unbound(bucket);
        (big_chart::big_chart_view(&view), view.width, view.height)
    });
}

#[story]
fn big_chart_loading(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_big_chart_loading(bucket);
        (big_chart::big_chart_view(&view), view.width, view.height)
    });
}

#[story]
fn big_chart_empty(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_big_chart_empty(bucket);
        (big_chart::big_chart_view(&view), view.width, view.height)
    });
}

#[story]
fn big_chart_failed(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_big_chart_failed(bucket);
        (big_chart::big_chart_view(&view), view.width, view.height)
    });
}

#[story]
fn big_chart_denied(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        let view = fixtures::sample_big_chart_denied(bucket);
        (big_chart::big_chart_view(&view), view.width, view.height)
    });
}

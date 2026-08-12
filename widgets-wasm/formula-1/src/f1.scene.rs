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

//! Formula 1 screen scenes, rendered natively over fixture data.
//! Every scene shows all four design sizes as stacked stages.

use bmc_gallery::prelude::*;
use formula_1::model::SizeBucket;
use formula_1::screens::{fixtures, next_race, standings};

scene_meta! { title: "Widgets / Formula 1" }

const BUCKETS: [(SizeBucket, &str); 4] = [
    (SizeBucket::Full, "Fullscreen"),
    (SizeBucket::Large, "Large"),
    (SizeBucket::Medium, "Medium"),
    (SizeBucket::Small, "Small"),
];

/// Each design size on its own stage, at the size the design gives it.
///
/// The callback hands back a closure that *builds* the tree rather than the
/// tree itself: these screens draw SVG icons, and the registrars are only
/// live inside the stage.
fn size_stages<Build: FnOnce() -> Node>(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    mut view: impl FnMut(SizeBucket) -> Build,
) {
    for (bucket, label) in BUCKETS {
        let build = view(bucket);
        ui.heading(label);
        ctx.node_stage(ui, bucket.design_size(), build);
    }
}

#[scene(default)]
fn standings(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || standings::standings_view(&fixtures::standings(bucket))
    });
}

/// Nothing stored yet: first reply outstanding, or a cold server 503ing.
#[scene]
fn standings_empty(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || standings::standings_view(&fixtures::standings_empty(bucket))
    });
}

/// Opening weekend, before anyone has scored.
#[scene]
fn standings_season_start(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || standings::standings_view(&fixtures::standings_season_start(bucket))
    });
}

/// The longest names and largest scores the columns have to seat.
#[scene]
fn standings_widest(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || standings::standings_view(&fixtures::standings_widest(bucket))
    });
}

#[story]
fn next_race(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        next_race::next_race_view(&fixtures::next_race(bucket))
    });
}

/// Between seasons, or before the first reply has landed.
#[story]
fn next_race_unavailable(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        next_race::next_race_view(&fixtures::next_race_unavailable(bucket))
    });
}

/// A weekend announced before any of its detail was.
#[story]
fn next_race_sparse(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        next_race::next_race_view(&fixtures::next_race_sparse(bucket))
    });
}

/// The longest names the rows have to seat, on a sprint weekend.
#[story]
fn next_race_widest(ctx: &mut StoryCtx) {
    size_frames(ctx, |bucket| {
        next_race::next_race_view(&fixtures::next_race_widest(bucket))
    });
}

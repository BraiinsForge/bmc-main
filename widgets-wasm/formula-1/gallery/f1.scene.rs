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
use formula_1::model::{SizeBucket, size_bucket};
use formula_1::screens::{driver, fixtures, live, next_race, standings};

scene_meta! { title: "Widgets / Formula 1" }

/// The device frames this widget is staged in: every one the gallery knows
/// of, less the round face, which the manifest does not admit.
fn viewports() -> impl Iterator<Item = DeviceViewport> {
    DEVICE_VIEWPORTS
        .into_iter()
        .filter(|viewport| !viewport.size.is_round())
}

/// The artwork the scenes draw with, kept here rather than in the widget.
/// The gallery alone compiles this file, so a sample image kept here has
/// no route into the binary a deck runs.
const ARTWORK: &[(fixtures::AssetName, &[u8])] = &[
    (fixtures::CIRCUIT, include_bytes!("artwork/circuit.png")),
    ("flag-arg", include_bytes!("artwork/flags/arg.png")),
    ("flag-aus", include_bytes!("artwork/flags/aus.png")),
    ("flag-bra", include_bytes!("artwork/flags/bra.png")),
    ("flag-can", include_bytes!("artwork/flags/can.png")),
    ("flag-esp", include_bytes!("artwork/flags/esp.png")),
    ("flag-fin", include_bytes!("artwork/flags/fin.png")),
    ("flag-fra", include_bytes!("artwork/flags/fra.png")),
    ("flag-gbr", include_bytes!("artwork/flags/gbr.png")),
    ("flag-ger", include_bytes!("artwork/flags/ger.png")),
    ("flag-ita", include_bytes!("artwork/flags/ita.png")),
    ("flag-mex", include_bytes!("artwork/flags/mex.png")),
    ("flag-mon", include_bytes!("artwork/flags/mon.png")),
    ("flag-ned", include_bytes!("artwork/flags/ned.png")),
    ("flag-nzl", include_bytes!("artwork/flags/nzl.png")),
    ("flag-tha", include_bytes!("artwork/flags/tha.png")),
    ("headshot-01", include_bytes!("artwork/headshots/01.gif")),
    ("headshot-02", include_bytes!("artwork/headshots/02.gif")),
    ("headshot-03", include_bytes!("artwork/headshots/03.gif")),
    ("headshot-04", include_bytes!("artwork/headshots/04.gif")),
    ("headshot-05", include_bytes!("artwork/headshots/05.gif")),
    ("headshot-06", include_bytes!("artwork/headshots/06.gif")),
    ("headshot-07", include_bytes!("artwork/headshots/07.gif")),
    ("headshot-08", include_bytes!("artwork/headshots/08.gif")),
    ("headshot-09", include_bytes!("artwork/headshots/09.gif")),
    ("headshot-10", include_bytes!("artwork/headshots/10.gif")),
    ("headshot-11", include_bytes!("artwork/headshots/11.gif")),
    ("headshot-12", include_bytes!("artwork/headshots/12.gif")),
    ("headshot-13", include_bytes!("artwork/headshots/13.gif")),
    ("headshot-14", include_bytes!("artwork/headshots/14.gif")),
    ("headshot-15", include_bytes!("artwork/headshots/15.gif")),
    ("headshot-16", include_bytes!("artwork/headshots/16.gif")),
    ("headshot-17", include_bytes!("artwork/headshots/17.gif")),
    ("headshot-18", include_bytes!("artwork/headshots/18.gif")),
    ("headshot-19", include_bytes!("artwork/headshots/19.gif")),
    ("headshot-20", include_bytes!("artwork/headshots/20.gif")),
    ("headshot-21", include_bytes!("artwork/headshots/21.gif")),
    ("headshot-22", include_bytes!("artwork/headshots/22.gif")),
    ("logo-01", include_bytes!("artwork/logos/01.png")),
    ("logo-02", include_bytes!("artwork/logos/02.png")),
    ("logo-03", include_bytes!("artwork/logos/03.png")),
    ("logo-04", include_bytes!("artwork/logos/04.png")),
    ("logo-05", include_bytes!("artwork/logos/05.png")),
    ("logo-06", include_bytes!("artwork/logos/06.png")),
    ("logo-07", include_bytes!("artwork/logos/07.png")),
    ("logo-08", include_bytes!("artwork/logos/08.png")),
    ("logo-09", include_bytes!("artwork/logos/09.png")),
    ("logo-10", include_bytes!("artwork/logos/10.png")),
    ("logo-11", include_bytes!("artwork/logos/11.png")),
];

/// Put the fixtures' sample images where the screens restore images
/// from, so the seeded ones render and the rest hold placeholders.
fn seed_images() {
    for (kind, url, asset) in fixtures::image_seeds() {
        let Some((_, bytes)) = ARTWORK.iter().find(|(name, _)| *name == asset) else {
            continue;
        };
        let (width, height) = kind.decode_size();
        seed_image(
            &formula_1::images::tag_for(kind, &url),
            bytes,
            width,
            height,
            url.as_str().as_bytes(),
        );
    }
}

/// Which viewport to stage, as an index into [`viewports`],
/// `None` for every one of them.
///
/// Stacking them all is what a scene is worth looking at for,
/// so that is the default; a capture recipe pins one
/// and gets a frame per shot rather than a column of six.
fn only_size(ctx: &mut SceneCtx) -> Option<usize> {
    let mut labels = vec!["All"];
    labels.extend(viewports().map(|viewport| viewport.label));
    ctx.select("Size", &labels, 0).checked_sub(1)
}

/// Each design size on its own stage, under whichever settings the knobs
/// hold — the distances, clocks and separators all read from them.
///
/// The callback hands back a closure that *builds* the tree rather than the
/// tree itself: these screens draw SVG icons, and the registrars are only
/// live inside the stage.
fn size_stages<Build: FnOnce() -> Node>(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    mut view: impl FnMut(SizeBucket) -> Build,
) {
    // Ahead of the settings group, which everything after it joins.
    let only = only_size(ctx);
    deck_settings(ctx);
    seed_images();
    for (index, viewport) in viewports().enumerate() {
        if only.is_some_and(|wanted| wanted != index) {
            continue;
        }
        let (width, height) = viewport.pixels();
        let build = view(size_bucket(width, height));
        ui.heading(viewport.label);
        ctx.node_stage(ui, viewport.size, build);
    }
}

#[scene(default)]
fn standings(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || standings::standings_view(&fixtures::standings(bucket))
    });
}

/// A card whose payload named no championship rank.
#[scene]
fn driver_unranked(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || driver::driver_view(&fixtures::driver_unranked(bucket))
    });
}

/// Cars a lap down: `+1 LAP`, the widest gap text the server sends.
#[scene]
fn live_lapped(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || live::race_view(&fixtures::live_lapped(bucket))
    });
}

/// A race whose payload named no places and no lap count.
#[scene]
fn live_unranked(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || live::race_view(&fixtures::live_unranked(bucket))
    });
}

/// Rows the payload gave no place, which zero cannot stand in for.
#[scene]
fn standings_unranked(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || standings::standings_view(&fixtures::standings_unranked(bucket))
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

#[scene]
fn next_race(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || next_race::next_race_view(&fixtures::next_race(bucket))
    });
}

/// Between seasons, or before the first reply has landed.
#[scene]
fn next_race_unavailable(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || next_race::next_race_view(&fixtures::next_race_unavailable(bucket))
    });
}

/// A weekend announced before any of its detail was.
#[scene]
fn next_race_sparse(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || next_race::next_race_view(&fixtures::next_race_sparse(bucket))
    });
}

/// The longest names the rows have to seat, on a sprint weekend.
#[scene]
fn next_race_widest(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || next_race::next_race_view(&fixtures::next_race_widest(bucket))
    });
}

/// The race board mid-flight: a fastest lap, a car in the pits,
/// one retired, and the sector colours all mixed.
#[scene]
fn live_race(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || live::race_view(&fixtures::live(bucket, "Race"))
    });
}

/// Qualifying, whose widest frame splits the whole field in two.
#[scene]
fn live_quali(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || live::quali_view(&fixtures::live(bucket, "Q3"))
    });
}

/// Practice, which trades the interval for a best lap and an out lap.
#[scene]
fn live_practice(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || live::practice_view(&fixtures::live(bucket, "Practice 2"))
    });
}

/// The quiet week: every board says which session it is waiting for.
#[scene]
fn live_idle(ctx: &mut SceneCtx, ui: &mut Ui) {
    let only = only_size(ctx);
    deck_settings(ctx);
    seed_images();
    for (index, viewport) in viewports().enumerate() {
        if only.is_some_and(|wanted| wanted != index) {
            continue;
        }
        let (width, height) = viewport.pixels();
        let bucket = size_bucket(width, height);
        ui.heading(viewport.label);
        ui.label("race, quali, practice");
        for view in [live::race_view, live::quali_view, live::practice_view] {
            ctx.node_stage(ui, viewport.size, move || {
                view(&fixtures::live_idle(bucket))
            });
        }
    }
}

#[scene]
fn driver(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || driver::driver_view(&fixtures::driver(bucket))
    });
}

/// A rookie the upstream knows least about: no engineer, no debut year.
#[scene]
fn driver_sparse(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || driver::driver_view(&fixtures::driver_sparse(bucket))
    });
}

/// Every value the payload may leave out, left out at once, so the card
/// shows each placeholder beside the few values that always arrive.
#[scene]
fn driver_placeholders(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || {
            let mut stats = fixtures::drivers()
                .into_iter()
                .next()
                .expect("BUG: fixtures name a driver");
            stats.number = None;
            stats.ranking = None;
            stats.gp_wins = None;
            stats.world_titles = None;
            stats.age = None;
            stats.weight = None;
            stats.height = None;
            stats.race_engineer = None;
            stats.debut_year = None;
            driver::driver_view(&fixtures::driver_card(bucket, Some(stats)))
        }
    });
}

/// A constructor's name rather than filler, since a proportional font
/// makes the glyph mix, not the count, decide what fits.
const RULER_SOURCE: &str = "Scuderia Ferrari Racing Team Alpha Romeo Sauber";

/// Text of exactly `chars` characters, cut from [`RULER_SOURCE`].
fn ruler(chars: usize) -> String {
    assert!(
        chars <= RULER_SOURCE.chars().count(),
        "a {chars}-character ruler needs a longer source than {RULER_SOURCE:?}",
    );
    RULER_SOURCE.chars().take(chars).collect()
}

/// Drag until the column cuts — that length is the frame's budget.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a character count over a fixed sample string, and the slider \
              is bounded to it"
)]
fn ruler_length(ctx: &mut SceneCtx) -> usize {
    let longest = RULER_SOURCE.chars().count() as f32;
    ctx.slider("Length", 20.0, 1.0, longest, 1.0) as usize
}

/// The value column at a chosen length, to see where it cuts.
#[scene]
fn driver_widest(ctx: &mut SceneCtx, ui: &mut Ui) {
    let chars = ruler_length(ctx);
    size_stages(ctx, ui, move |bucket| {
        move || driver::driver_view(&fixtures::driver_widest(bucket, &ruler(chars)))
    });
}

/// The name and team columns at a chosen length.
#[scene]
fn standings_ruler(ctx: &mut SceneCtx, ui: &mut Ui) {
    let chars = ruler_length(ctx);
    size_stages(ctx, ui, move |bucket| {
        move || standings::standings_view(&fixtures::standings_ruler(bucket, &ruler(chars)))
    });
}

/// The info column at a chosen length.
#[scene]
fn next_race_ruler(ctx: &mut SceneCtx, ui: &mut Ui) {
    let chars = ruler_length(ctx);
    size_stages(ctx, ui, move |bucket| {
        move || next_race::next_race_view(&fixtures::next_race_ruler(bucket, &ruler(chars)))
    });
}

/// Between seasons, or before the first reply has landed.
#[scene]
fn driver_unavailable(ctx: &mut SceneCtx, ui: &mut Ui) {
    size_stages(ctx, ui, |bucket| {
        move || driver::driver_view(&fixtures::driver_unavailable(bucket))
    });
}

/// Every driver the season knows, each drawn by the view the widget
/// ships — so one scene covers the field, not the one a param picks.
#[scene]
fn driver_grid(ctx: &mut SceneCtx, ui: &mut Ui) {
    deck_settings(ctx);
    seed_images();
    for stats in fixtures::drivers() {
        let (name, team) = (stats.name.clone(), stats.team.clone());
        let view = fixtures::driver_card(SizeBucket::Full, Some(stats));
        ui.heading(&name);
        ui.label(&team);
        ctx.node_stage(ui, SizeBucket::Full.design_size(), move || {
            driver::driver_view(&view)
        });
    }
}

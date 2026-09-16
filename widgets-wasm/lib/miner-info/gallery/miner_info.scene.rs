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

use bmc_gallery::prelude::*;
use bmc_wasm_sdk::{ViewportShape, WidgetViewport};
use miner_info::face;
use miner_info::face::RenderSize;
use miner_info::fixtures::{DEFAULT_TARGET_THS, PriceMove, Reported, miner, public};
use miner_info::layout;

scene_meta! { title: "Widgets / Miner Info" }

/// The panels that share the small rectangular faces.
const SMALL_VIEWPORTS: [(u32, u32, &str); 2] = [(317, 238, "BMC100 slot"), (320, 240, "BMM100")];

const BMM101_VIEWPORT: (u32, u32) = (480, 320);

const PRICE_MOVES: [(&str, PriceMove); 3] = [
    ("Up", PriceMove::Up),
    ("Down", PriceMove::Down),
    ("No history", PriceMove::NoHistory),
];

/// Diameter of the BFM100 face. Staged as [`Round`] rather than a square
/// so the mask shows what the bezel cuts — the faces lay out in bands,
/// and whether a band clears the circle is the thing worth looking at.
const ROUND_DIAMETER: usize = 480;

/// Hashrates that land on each `GaugeState`, given [`DEFAULT_TARGET_THS`]
/// and the +/-5% good band. `None` leaves the reading unavailable.
const GAUGE_STATES: [(&str, Option<f64>); 5] = [
    ("Good", Some(1.0)),
    ("Overclocked", Some(1.2)),
    ("Underclocked", Some(0.8)),
    ("Off", Some(0.0)),
    ("Unavailable", None),
];

fn rectangular_panel(viewport: (u32, u32)) -> layout::Panel {
    layout::classify(WidgetViewport {
        width: viewport.0,
        height: viewport.1,
        shape: ViewportShape::Rectangular,
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a display dimension, and `RenderSize` carries the viewport as u32"
)]
fn round_size() -> RenderSize {
    let side = ROUND_DIAMETER as u32;
    RenderSize {
        width: side,
        height: side,
    }
}

/// How many stages of `stage_width` fit across the visible canvas.
///
/// The scene canvas is an `egui::ScrollArea::both`, so `available_width`
/// is the scrollable extent and never bounds anything — `horizontal_wrapped`
/// therefore never wraps. `clip_rect` is the part actually on screen,
/// which is what a column count has to be measured against.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "screen widths are exact in f32 at this magnitude, and the column count they yield is small and non-negative"
)]
fn columns_across(ui: &Ui, stage_width: usize) -> usize {
    let gap = ui.spacing().item_spacing.x;
    let visible = ui.clip_rect().width();
    ((visible / (stage_width as f32 + gap)).floor() as usize).max(1)
}

fn reported(ctx: &mut SceneCtx) -> Reported {
    match ctx.select(
        "Data",
        &["Populated", "Without board readings", "Unavailable"],
        0,
    ) {
        1 => Reported::WithoutBoards,
        2 => Reported::Nothing,
        _ => Reported::All,
    }
}

fn price_move(ctx: &mut SceneCtx) -> PriceMove {
    let labels: Vec<&str> = PRICE_MOVES.iter().map(|(label, _)| *label).collect();
    PRICE_MOVES[ctx.select("Price", &labels, 0)].1
}

/// Which of the three widgets to draw; the same pick serves every panel.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Face {
    Mining,
    Geek,
    InfoOverload,
}

fn face_pick(ctx: &mut SceneCtx) -> Face {
    match ctx.select("Face", &["Mining", "Geek", "Info Overload"], 0) {
        1 => Face::Geek,
        2 => Face::InfoOverload,
        _ => Face::Mining,
    }
}

#[scene]
fn rectangular(ctx: &mut SceneCtx, ui: &mut Ui) {
    let mut labels = vec!["All"];
    labels.extend(SMALL_VIEWPORTS.iter().map(|(.., label)| *label));
    let selected = ctx.select("Viewport", &labels, 0);
    let face = face_pick(ctx);
    let shown = reported(ctx);
    let price = price_move(ctx);
    system_settings(ctx);

    // Laid out across rather than stacked: the frames are small enough that
    // a wide window fits several side by side, which is how you compare them.
    let staged: Vec<_> = SMALL_VIEWPORTS
        .into_iter()
        .enumerate()
        .filter(|(index, _)| selected == 0 || selected == index + 1)
        .map(|(_, viewport)| viewport)
        .collect();
    let widest = staged.iter().map(|(width, ..)| *width).max().unwrap_or(1);
    let per_row = columns_across(ui, widest as usize);

    for row in staged.chunks(per_row) {
        ui.horizontal_top(|ui| {
            for &(width, height, label) in row {
                ui.vertical(|ui| {
                    ui.heading(label);
                    ctx.node_stage(ui, (width, height), move || {
                        let data = miner(shown, Some(1.02));
                        let market = public(shown, price);
                        let panel = rectangular_panel((width, height));
                        match face {
                            Face::Mining => face::mining(panel, &data),
                            Face::Geek => face::geek(panel, &data, &market),
                            Face::InfoOverload => face::info_overload(panel, &data, &market),
                        }
                    });
                });
            }
        });
    }
}

/// The BMM101 faces at their own size.
#[scene]
fn bmm101(ctx: &mut SceneCtx, ui: &mut Ui) {
    let face = face_pick(ctx);
    let shown = reported(ctx);
    let price = price_move(ctx);
    system_settings(ctx);

    ctx.node_stage(ui, BMM101_VIEWPORT, move || {
        let data = miner(shown, Some(1.02));
        let market = public(shown, price);
        let panel = rectangular_panel(BMM101_VIEWPORT);
        match face {
            Face::Mining => face::mining(panel, &data),
            Face::Geek => face::geek(panel, &data, &market),
            Face::InfoOverload => face::bmm101::info_overload(&data, &market),
        }
    });
}

/// The round Mining and Geek faces across every gauge state,
/// which is the one thing the rectangular faces cannot show at all.
#[scene]
fn round_gauge(ctx: &mut SceneCtx, ui: &mut Ui) {
    let geek = ctx.select("Face", &["Mining", "Geek"], 0) == 1;
    // The gauge states vary only the hashrate, so the quadrants stay populated
    // even where the ring reads nothing; this drops them too.
    let shown = reported(ctx);
    system_settings(ctx);

    let per_row = columns_across(ui, ROUND_DIAMETER);
    for row in GAUGE_STATES.chunks(per_row) {
        ui.horizontal_top(|ui| {
            for &(label, hashrate) in row {
                ui.vertical(|ui| {
                    ui.heading(label);
                    ctx.node_stage(ui, Round(ROUND_DIAMETER), move || {
                        let data = miner(shown, hashrate);
                        let market = public(shown, PriceMove::Up);
                        let at = round_size();
                        if geek {
                            face::round::geek(at, &data, &market, false)
                        } else {
                            face::round::mining(at, &data, false)
                        }
                    });
                });
            }
        });
    }
}

#[scene]
fn round_info_overload(ctx: &mut SceneCtx, ui: &mut Ui) {
    let shown = reported(ctx);
    let price = price_move(ctx);
    system_settings(ctx);
    ctx.node_stage(ui, Round(ROUND_DIAMETER), move || {
        face::round::info_overload(&miner(shown, Some(1.02)), &public(shown, price))
    });
}

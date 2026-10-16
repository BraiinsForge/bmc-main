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
use miner_info::face;
use miner_info::face::RenderSize;
use miner_info::fixtures::{DEFAULT_TARGET_THS, PriceMove, Reported, miner, public};
use miner_info::layout::Panel;

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

/// Hashrates that land on each `GaugeState`, given the fixture's tuner target
/// and the +/-5% good band. `None` leaves the reading unavailable.
const GAUGE_STATES: [(&str, Option<f64>); 5] = [
    ("Good", Some(DEFAULT_TARGET_THS)),
    ("Overclocked", Some(1.2 * DEFAULT_TARGET_THS)),
    ("Underclocked", Some(0.8 * DEFAULT_TARGET_THS)),
    ("Off", Some(0.0)),
    ("Unavailable", None),
];

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

    ctx.wrapping_flow(
        ui,
        SMALL_VIEWPORTS
            .into_iter()
            .enumerate()
            .filter(|(index, _)| selected == 0 || selected == index + 1)
            .map(|(_, (width, height, label))| {
                titled(label, (width, height), move || {
                    let data = miner(shown, Some(1.02));
                    let market = public(shown, price);
                    match face {
                        Face::Mining => face::mining(&data),
                        Face::Geek => face::geek(Panel::Small, &data, &market),
                        Face::InfoOverload => face::info_overload(&data, &market),
                    }
                })
            }),
    );
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
        match face {
            Face::Mining => face::bmm101::mining(&data),
            Face::Geek => face::geek(Panel::Bmm101, &data, &market),
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

    ctx.wrapping_flow(
        ui,
        GAUGE_STATES.into_iter().map(|(label, hashrate)| {
            titled(label, Round(ROUND_DIAMETER), move || {
                let data = miner(shown, hashrate);
                let market = public(shown, PriceMove::Up);
                let at = round_size();
                if geek {
                    face::round::geek(at, &data, &market, false)
                } else {
                    face::round::mining(at, &data, false)
                }
            })
        }),
    );
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

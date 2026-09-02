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

use core::time::Duration;

use bmc_gallery::prelude::*;
use bmc_wasm_sdk::{
    BitcoinAmount, ElectricPower, Hashrate, Hashvalue, MiningEfficiency, Ratio, Svg, Temperature,
    include_svg,
};
use miner_info::availability::Availability;
use miner_info::face;
use miner_info::face::RenderSize;
use miner_info::model::{
    Constraints, Currency, Hashprice, MinerData, Money, PublicData, TargetRange, TemperatureRange,
};

scene_meta! { title: "Widgets / Miner Info" }

// `include_svg!` resolves against the crate hosting the file, so the gallery
// keeps a copy of its own to hand to the faces that draw it.
const CHIP_ICON: Svg = include_svg!("assets/chip.svg");

const RECT_VIEWPORTS: [(u32, u32, &str); 3] = [
    (317, 238, "BMC100 slot"),
    (320, 240, "BMM100"),
    (480, 320, "BMM101"),
];

/// Diameter of the BFM100 face. Staged as [`Round`] rather than a square
/// so the mask shows what the bezel cuts — the faces lay out in bands,
/// and whether a band clears the circle is the thing worth looking at.
const ROUND_DIAMETER: usize = 480;

/// Tuner target the gauge sweeps anchor against. A healthy miner is tuned to
/// the default, so `hashrate` relative to it decides the gauge state.
const DEFAULT_TARGET_THS: f64 = 1.0;

/// Hashrates that land on each `GaugeState`, given [`DEFAULT_TARGET_THS`]
/// and the +/-5% good band. `None` leaves the reading unavailable.
const GAUGE_STATES: [(&str, Option<f64>); 5] = [
    ("Good", Some(1.0)),
    ("Overclocked", Some(1.2)),
    ("Underclocked", Some(0.8)),
    ("Off", Some(0.0)),
    ("Unavailable", None),
];

/// How much of itself a miner reports.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Reported {
    /// Everything the faces can draw.
    All,
    /// No board sensor and no per-board rates, which is all it takes to lose
    /// the temperature row and the mining-mode ratio: each needs a pair of
    /// readings, and half a pair reads as nothing.
    WithoutBoards,
    /// Nothing at all — the placeholder pass every face has to survive
    /// without collapsing its layout.
    Nothing,
}

fn miner(reported: Reported, hashrate_ths: Option<f64>) -> MinerData {
    if reported == Reported::Nothing {
        return MinerData::default();
    }
    let boards = reported == Reported::All;
    MinerData {
        hashrate: hashrate_ths
            .map(Hashrate::from_terahashes_per_second)
            .into(),
        temperature: boards
            .then(|| TemperatureRange {
                board: Temperature::from_celsius(61.0),
                chip: Temperature::from_celsius(74.0),
            })
            .into(),
        power: Availability::Available(ElectricPower::from_watts(41.0)),
        efficiency: Availability::Available(MiningEfficiency::from_joules_per_terahash(21.5)),
        mcr: boards.then(|| Ratio::from_percent(98.0)).into(),
        fan_speed: Availability::Available(Ratio::from_percent(72.0)),
        uptime: Availability::Available(Duration::from_hours(2 * 24 + 3) + Duration::from_mins(57)),
        ip_address: Availability::Available("192.168.23.1".to_owned()),
        chip_type: Availability::Available("BM1370".to_owned()),
        chip_count: Availability::Available(108),
        constraints: Constraints {
            hashrate: Some(TargetRange {
                min: 0.5,
                default: DEFAULT_TARGET_THS,
                max: 1.4,
            }),
            power: None,
        },
    }
}

/// The market fixture, or a fully-unavailable one. Drops alongside the miner
/// half: a face reading only one of the two would otherwise still look full.
fn public(reported: Reported) -> PublicData {
    if reported == Reported::Nothing {
        return PublicData::default();
    }
    PublicData {
        btc_price: Availability::Available(Money::new(101_754.0, Currency::Usd)),
        btc_change_24h: Availability::Available(Ratio::from_percent(6.25)),
        prev_diff_adjust: Availability::Available(Ratio::from_fraction(-0.021)),
        est_diff_adjust: Availability::Available(Ratio::from_fraction(-0.045)),
        epoch_progress: Availability::Available(Ratio::from_fraction(0.87)),
        avg_fee: Availability::Available(BitcoinAmount::from_bitcoin(0.055)),
        avg_fee_share: Availability::Available(Ratio::from_percent(12.1)),
        block_height: Availability::Available(880_123),
        hashvalue: Availability::Available(Hashvalue::from_satoshis_per_terahash_day(70.0)),
        btc_price_history: (0..64).map(|i| 100_000.0 + f64::from(i) * 30.0).collect(),
    }
}

fn size(viewport: (u32, u32)) -> RenderSize {
    RenderSize {
        width: viewport.0,
        height: viewport.1,
    }
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

#[scene]
fn rectangular(ctx: &mut SceneCtx, ui: &mut Ui) {
    let selected = ctx.select("Viewport", &["All", "BMC100 slot", "BMM100", "BMM101"], 0);
    let face_pick = ctx.select("Face", &["Mining", "Geek", "Info Overload"], 0);
    let shown = reported(ctx);
    system_settings(ctx);

    // Laid out across rather than stacked: the frames are small enough that
    // a wide window fits several side by side, which is how you compare them.
    let staged: Vec<_> = RECT_VIEWPORTS
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
                        let market = public(shown);
                        let at = size((width, height));
                        match face_pick {
                            1 => face::geek(at, &data, &market),
                            2 => face::info_overload(at, &data, &market),
                            _ => face::mining(at, &data),
                        }
                    });
                });
            }
        });
    }
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
                        let market = public(shown);
                        let at = round_size();
                        if geek {
                            face::round::geek(at, &data, &market, false, &CHIP_ICON)
                        } else {
                            face::round::mining(at, &data, false, &CHIP_ICON)
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
    system_settings(ctx);
    ctx.node_stage(ui, Round(ROUND_DIAMETER), move || {
        face::round::info_overload(&miner(shown, Some(1.02)), &public(shown))
    });
}

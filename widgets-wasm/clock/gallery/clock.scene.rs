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

//! The clock's three faces at every design size, and on the two panels
//! that have no bucket of their own.

use bmc_gallery::prelude::*;
use clock::model::SizeBucket;
use clock::screens::{clock_view, fixtures};
use clock::{ClockStyle, NumbersFontStyle, Params};

scene_meta! { title: "Widgets / Clock" }

const BUCKETS: [(SizeBucket, &str); 5] = [
    (SizeBucket::Full, "Fullscreen"),
    (SizeBucket::Large, "Large"),
    (SizeBucket::Medium, "Medium"),
    (SizeBucket::Small, "Small"),
    (SizeBucket::Bmm101, "BMM101"),
];

const BMM100_VIEWPORT: (u32, u32) = (320, 240);
const BFM100_DIAMETER: u32 = 480;

/// The evening first: its hands stand apart, where noon stacks all three on the caption.
const MOMENTS: [(SystemTime, &str); 3] = [
    (fixtures::MAY_EVENING, "Fri 1 May 2026, 21:05:09"),
    (fixtures::SEPTEMBER_NOON, "Mon 14 Sep 2026, 12:30:00"),
    (fixtures::DESIGN_MOMENT, "Wed 24 Dec 2025, 00:39:30"),
];

/// Zones an operator may type over the system one, ending on a half-typed name
/// so the red `(unknown)` caption has a knob.
const OVERRIDES: [(Option<&str>, &str); 5] = [
    (None, "System"),
    (Some("Europe/Helsinki"), "Helsinki"),
    (Some("America/New_York"), "New York"),
    (Some("Asia/Kolkata"), "Kolkata"),
    (Some("Europe/Helsi"), "Helsi (unknown)"),
];

fn only_size(ctx: &mut SceneCtx) -> Option<SizeBucket> {
    let labels: Vec<&str> = ["All"]
        .into_iter()
        .chain(BUCKETS.iter().map(|(_, label)| *label))
        .collect();
    let choice = ctx.select("Size", &labels, 0);
    choice.checked_sub(1).map(|at| BUCKETS[at].0)
}

/// The widget's own params and the moment shown, as a knob group.
fn clock_knobs(ctx: &mut SceneCtx, style: ClockStyle) -> (Params, SystemTime) {
    ctx.group("Widget settings");
    let numerals = ctx.select("Numerals", &["Regular", "Semi-bold", "Bold"], 1);
    let show_date = ctx.toggle("Show date", true);
    let show_seconds = ctx.toggle("Show seconds", true);
    let show_timezone = ctx.toggle("Show timezone", true);
    let override_labels: Vec<&str> = OVERRIDES.iter().map(|(_, label)| *label).collect();
    let timezone_override = OVERRIDES[ctx.select("Timezone override", &override_labels, 0)].0;
    let moment_labels: Vec<&str> = MOMENTS.iter().map(|(_, label)| *label).collect();
    let now = MOMENTS[ctx.select("Moment", &moment_labels, 0)].0;
    let params = Params {
        clock_style: style,
        numbers_font_style: match numerals {
            0 => NumbersFontStyle::Regular,
            2 => NumbersFontStyle::Bold,
            _ => NumbersFontStyle::SemiBold,
        },
        show_date,
        show_seconds,
        show_timezone,
        timezone_override: timezone_override.map(str::to_owned),
    };
    (params, now)
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
    now: SystemTime,
    params: &Params,
) {
    // A grid counts every widget as a column, so the heading rides with its
    // stage as one block.
    ui.vertical(|ui| {
        ui.heading(label);
        let view = fixtures::at_bucket(bucket, now, params.clone());
        ctx.node_stage(ui, bucket.design_size(), move || clock_view(&view));
    });
}

/// One stage per bucket, each at its design size, in as many columns as fit.
/// Fullscreen goes on its own: the grid measures its columns from the widest
/// stage, and 1280 would hold the rest to one column.
fn face_stages(ctx: &mut SceneCtx, ui: &mut Ui, style: ClockStyle) {
    let only = only_size(ctx);
    let (params, now) = clock_knobs(ctx, style);
    system_settings(ctx);
    let (fullscreen, rest): (Vec<_>, Vec<_>) = BUCKETS
        .into_iter()
        .filter(|(bucket, _)| only.is_none_or(|wanted| wanted == *bucket))
        .partition(|(bucket, _)| *bucket == SizeBucket::Full);
    for stage in fullscreen {
        bucket_stage(ctx, ui, stage, now, &params);
    }
    ctx.matrix_with(ui, &matrix_sizes(&rest), |ctx, ui, at| {
        bucket_stage(ctx, ui, rest[at], now, &params);
    });
}

#[scene(default)]
fn digital(ctx: &mut SceneCtx, ui: &mut Ui) {
    face_stages(ctx, ui, ClockStyle::Digital);
}

#[scene("Analog Rect")]
fn analog_rect(ctx: &mut SceneCtx, ui: &mut Ui) {
    face_stages(ctx, ui, ClockStyle::AnalogRect);
}

#[scene("Analog Round")]
fn analog_round(ctx: &mut SceneCtx, ui: &mut Ui) {
    face_stages(ctx, ui, ClockStyle::AnalogRound);
}

/// The 320×240 panel, which has no bucket: it draws the Small layout scaled down.
#[scene("BMM100")]
fn bmm100(ctx: &mut SceneCtx, ui: &mut Ui) {
    let style = match ctx.select("Face", &["Digital", "Analog Rect", "Analog Round"], 0) {
        1 => ClockStyle::AnalogRect,
        2 => ClockStyle::AnalogRound,
        _ => ClockStyle::Digital,
    };
    let (params, now) = clock_knobs(ctx, style);
    system_settings(ctx);
    let (width, height) = BMM100_VIEWPORT;
    let view = fixtures::rectangular(width, height, now, params);
    ctx.node_stage(ui, BMM100_VIEWPORT, move || clock_view(&view));
}

/// The round panel, which draws the round dial whatever the configured style.
#[scene("BFM100")]
fn bfm100(ctx: &mut SceneCtx, ui: &mut Ui) {
    let (params, now) = clock_knobs(ctx, ClockStyle::AnalogRound);
    system_settings(ctx);
    let view = fixtures::round(BFM100_DIAMETER, now, params);
    let diameter =
        usize::try_from(BFM100_DIAMETER).expect("BUG: a round frame is a few hundred pixels");
    ctx.node_stage(ui, Round(diameter), move || clock_view(&view));
}

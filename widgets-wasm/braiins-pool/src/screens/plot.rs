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

//! The hashrate/workers line plot: one canvas-bearing fragment whose
//! geometry — gutters, tick labels, time band, baseline style, payout
//! markers — comes entirely from a per-layout [`ChartSpec`].

use bmc_wasm_sdk::types::{Hashrate, SiPrefix};
#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::chart;
use crate::model::{PayoutKind, Series};
use crate::screens::icons;
use crate::screens::parts::{self, color, font, space};

/// The chart lines' stroke width, and the plot's vertical inset when no
/// y labels claim one.
const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;

/// How the y ticks sit and read, per design frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickSpec {
    /// Smaller than body text, so the ticks fit their gutters.
    pub font: u32,
    /// Vertical plot inset while ticks are on: half a label line plus
    /// a little margin, so the top and bottom labels stay inside the canvas.
    pub inset: f32,
    /// Flush with the canvas edges, rather than a gap off the plot.
    pub at_edges: bool,
    /// Every hashrate tick in the axis top's prefix, rather than each lettering its own ("390P"):
    /// bare where the hero names that unit ("900"), lettered where it does not ("3T … 0T").
    pub shared_hashrate_scale: bool,
    /// The unit the hero line names.
    pub hero_unit: Option<SiPrefix>,
    /// Every worker tick in `k` once the axis top reaches a thousand
    /// ("3k … 0k"), rather than each condensing on its own.
    pub shared_worker_scale: bool,
}

pub const DECK_TICKS: TickSpec = TickSpec {
    font: font::TICK,
    inset: 16.0,
    at_edges: false,
    shared_hashrate_scale: false,
    hero_unit: None,
    shared_worker_scale: false,
};

/// Per-layout plot geometry; every field mirrors
/// a knob the design varies between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartSpec {
    /// Horizontal room for the hashrate ticks left of the plot ("390P")
    /// and the worker counts right of it ("2,5k"); zero bleeds the plot
    /// to that edge.
    pub left_gutter: f32,
    pub right_gutter: f32,
    /// Hashrate values on the left edge.
    pub hashrate_ticks: bool,
    /// Active-worker counts on the right edge.
    pub workers_ticks: bool,
    /// Vertical room under the plot for the x-axis time labels,
    /// `None` for no band.
    pub x_band: Option<f32>,
    /// The design draws the zero baseline solid on labelled plots and
    /// dashed on the bare ones (Small chart, Overview sparkline).
    pub solid_baseline: bool,
    /// Gridline intervals: 3 gives the labelled thirds, 2 the bare
    /// plots' halves. Tick labels require thirds.
    pub grid_steps: usize,
    pub ticks: TickSpec,
    /// Payout icon size, centered on the baseline; `None` for none.
    pub marker_size: Option<f32>,
}

/// The hashrate line and optionally the active-workers line, overlaid on
/// one plot with dashed gridlines; each series carries its own y scale,
/// labelled at the gridlines so labels always align. `x_labels`
/// (time-fraction → text) draw in the band under the plot when the spec
/// has one. Geometry comes from [`crate::chart`].
#[must_use]
pub fn line_chart(
    hashrate: &Series,
    workers: Option<&Series>,
    width: f32,
    height: f32,
    spec: &ChartSpec,
    x_labels: &[(f32, String)],
    payout_markers: &[(f32, PayoutKind)],
) -> Node {
    debug_assert!(
        (!spec.hashrate_ticks && !spec.workers_ticks) || spec.grid_steps == 3,
        "BUG: tick labels assume thirds gridlines"
    );
    let plot_w = width - spec.left_gutter - spec.right_gutter;
    // One line point per two pixels saturates the stroke; a flat cap would
    // hand the smallest tile the Fullscreen point budget, and the dense
    // 7-day series (~2000 slots) then sinks the frame rate.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a halved pixel count is small and non-negative"
    )]
    let max_points = (plot_w / 2.0).max(2.0) as usize;
    let plot_h = height - spec.x_band.unwrap_or(0.0);
    let inset_v = if spec.hashrate_ticks || spec.workers_ticks {
        spec.ticks.inset
    } else {
        CHART_INSET
    };

    let grid_y = |fraction: f32| plot_h - inset_v - (plot_h - 2.0 * inset_v) * fraction;
    let plot_x = |fraction: f32| spec.left_gutter + plot_w * fraction;
    let mut draws: Vec<Draw> = Vec::new();
    #[expect(
        clippy::cast_precision_loss,
        reason = "two or three grid steps are exact in f32"
    )]
    for step in 0..=spec.grid_steps {
        let fraction = step as f32 / spec.grid_steps as f32;
        let y = grid_y(fraction);
        let line = vec![(plot_x(0.0), y), (plot_x(1.0), y)];
        draws.push(if step == 0 && spec.solid_baseline {
            path!(line, stroke: 1.0, color: color::GRID)
        } else {
            path!(line, stroke: 1.0, color: color::GRID, dashed: (4.0, 4.0))
        });
    }

    let into_plot = |points: Vec<(f32, f32)>| -> Vec<(f32, f32)> {
        points
            .into_iter()
            .map(|(x, y)| (x + spec.left_gutter, y))
            .collect()
    };

    let hashrate_top = hashrate_axis_top(series_max(hashrate), &spec.ticks);
    let points = into_plot(chart::decimate(
        chart::series_points(hashrate, hashrate_top, plot_w, plot_h, inset_v),
        max_points,
    ));
    if points.len() >= 2 {
        draws.push(path!(points, stroke: CHART_STROKE, color: color::HASHRATE, smooth));
    }
    if spec.hashrate_ticks {
        draws.extend(hashrate_tick_draws(spec, hashrate_top, grid_y));
    }

    if let Some(workers) = workers {
        let workers_top = chart::y_axis_max(series_max(workers), true);
        let points = into_plot(chart::decimate(
            chart::series_points(workers, workers_top, plot_w, plot_h, inset_v),
            max_points,
        ));
        if points.len() >= 2 {
            draws.push(path!(points, stroke: CHART_STROKE, color: color::WORKERS, smooth));
        }
        if spec.workers_ticks {
            draws.extend(workers_tick_draws(
                spec,
                workers_top,
                width,
                plot_x(1.0),
                grid_y,
            ));
        }
    }

    if spec.x_band.is_some() {
        draws.extend(x_label_draws(x_labels, spec.left_gutter, plot_w, plot_h));
    }

    if let Some(size) = spec.marker_size {
        for (fraction, kind) in payout_markers {
            draws.push(payout_marker_draw(
                plot_x(*fraction),
                grid_y(0.0),
                size,
                *kind,
            ));
        }
    }

    canvas(props!(width: width, height: height), draws)
}

/// The hashrate ticks down the left side, each on its gridline.
fn hashrate_tick_draws(spec: &ChartSpec, top: f64, grid_y: impl Fn(f32) -> f32) -> Vec<Draw> {
    let (x, align) = if spec.ticks.at_edges {
        (0.0, TextAlign::Left)
    } else {
        (spec.left_gutter - space::GAP, TextAlign::Right)
    };
    let scale = hashrate_scale(&spec.ticks, top);
    axis_levels(top)
        .into_iter()
        .map(|(fraction, value)| {
            tick_text(
                x,
                grid_y(fraction),
                hashrate_tick(value, scale),
                align,
                spec.ticks.font,
            )
        })
        .collect()
}

/// The worker ticks down the right side, each on its gridline,
/// off the canvas's right edge or the plot's.
fn workers_tick_draws(
    spec: &ChartSpec,
    top: f64,
    canvas_right: f32,
    plot_right: f32,
    grid_y: impl Fn(f32) -> f32,
) -> Vec<Draw> {
    let (x, align) = if spec.ticks.at_edges {
        (canvas_right, TextAlign::Right)
    } else {
        (plot_right + space::GAP, TextAlign::Left)
    };
    axis_levels(top)
        .into_iter()
        .map(|(fraction, value)| {
            tick_text(
                x,
                grid_y(fraction),
                workers_tick(value, top, spec.ticks.shared_worker_scale),
                align,
                spec.ticks.font,
            )
        })
        .collect()
}

/// Time labels in the band under the plot.
fn x_label_draws(
    x_labels: &[(f32, String)],
    left_gutter: f32,
    plot_w: f32,
    plot_h: f32,
) -> Vec<Draw> {
    let mut draws = Vec::new();
    for (fraction, label) in x_labels {
        // Endpoint labels align inward so they never clip at the edges.
        let align = if *fraction <= 0.0 {
            TextAlign::Left
        } else if *fraction >= 1.0 {
            TextAlign::Right
        } else {
            TextAlign::Center
        };
        draws.push(Draw::text(
            left_gutter + plot_w * fraction,
            plot_h + space::GAP,
            label.as_str(),
            style!(size: font::BODY, color: color::TEXT_MUTED, family: FontFamily::DeckSans, align: align),
        ));
    }
    draws
}

/// A y-axis tick label, vertically centered on its gridline.
fn tick_text(x: f32, y: f32, tick: String, align: TextAlign, size: u32) -> Draw {
    Draw::text(
        x,
        y,
        tick,
        style!(size: size, color: color::TEXT_MUTED, family: FontFamily::DeckSans, align: align, valign: VerticalAlign::Center),
    )
}

/// A payout's icon centered on the given baseline point.
fn payout_marker_draw(cx: f32, cy: f32, size: f32, kind: PayoutKind) -> Draw {
    let icon = match kind {
        PayoutKind::Onchain => &icons::PAYOUT_BTC,
        PayoutKind::Lightning => &icons::PAYOUT_LN,
    };
    Draw::svg(
        cx - size / 2.0,
        cy - size / 2.0,
        size,
        size,
        icon,
        Color::default(),
    )
    .with_anti_alias()
}

fn series_max(series: &Series) -> f64 {
    series
        .samples
        .iter()
        .map(|s| s.value)
        .fold(0.0_f64, f64::max)
}

/// Gridline fractions paired with the axis value at each: 0 at the bottom,
/// the axis top at the top, thirds between.
fn axis_levels(top: f64) -> [(f32, f64); 4] {
    [
        (0.0, 0.0),
        (1.0 / 3.0, top / 3.0),
        (2.0 / 3.0, 2.0 * top / 3.0),
        (1.0, top),
    ]
}

/// The SI prefix a hashrate reads in at the hero's four significant figures.
#[must_use]
pub fn si_prefix(hashrate: Hashrate) -> Option<SiPrefix> {
    let (_, unit) = hashrate.format_si_parts(4);
    SiPrefix::from_symbol(unit.strip_suffix("H/s")?)
}

/// What prefix the hashrate ticks read in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HashrateScale {
    OwnPrefix,
    Shared { prefix: SiPrefix, lettered: bool },
}

/// The hashrate axis top.
/// Rounding the peak up to thirds can carry it into the next prefix,
/// 995 TH/s topping out at 1,02 PH/s, whose thirds need two decimals;
/// a shared scale rounds once more, to 1,2 PH/s.
fn hashrate_axis_top(max_terahashes: f64, ticks: &TickSpec) -> f64 {
    let top = chart::y_axis_max(max_terahashes, false);
    // Prefixes step every thousand, and terahashes sit on one.
    let prefix_step = |terahashes: f64| (terahashes.log10() / 3.0).floor();
    if ticks.shared_hashrate_scale
        && max_terahashes > 0.0
        && prefix_step(top) > prefix_step(max_terahashes)
    {
        chart::y_axis_max(top, false)
    } else {
        top
    }
}

/// The ticks share the axis top's prefix rather than the hero's:
/// in the hero's, an idle 0 H/s over the flat 3 TH/s axis counts trillions.
fn hashrate_scale(ticks: &TickSpec, top_terahashes: f64) -> HashrateScale {
    if !ticks.shared_hashrate_scale {
        return HashrateScale::OwnPrefix;
    }
    match si_prefix(Hashrate::from_terahashes_per_second(top_terahashes)) {
        Some(prefix) => HashrateScale::Shared {
            prefix,
            lettered: ticks.hero_unit != Some(prefix),
        },
        None => HashrateScale::OwnPrefix,
    }
}

/// A hashrate tick, no unit per the design (H/s is implied),
/// its prefix letter on the number where no hero names it ("390P", "3,9Z"),
/// mirroring the worker side's "2,5k".
fn hashrate_tick(terahashes: f64, scale: HashrateScale) -> String {
    let hashrate = Hashrate::from_terahashes_per_second(terahashes);
    match scale {
        HashrateScale::Shared { prefix, lettered } => {
            let number = trimmed_number(hashrate.as_si(prefix));
            if lettered {
                fmt!("{number}{}", prefix.symbol())
            } else {
                number
            }
        }
        HashrateScale::OwnPrefix => {
            let (number, unit) = hashrate.format_si_parts(3);
            let prefix = unit.strip_suffix("H/s").unwrap_or(&unit);
            fmt!("{number}{prefix}")
        }
    }
}

/// Worker counts label: plain up to a thousand, "2,5k" above, per the design.
fn workers_count_label(value: f64) -> String {
    parts::condensed_count(value, 1_000.0)
}

fn workers_tick(value: f64, top: f64, shared_scale: bool) -> String {
    if shared_scale && top >= 1_000.0 {
        fmt!("{}k", trimmed_number(value / 1_000.0))
    } else {
        workers_count_label(value)
    }
}

/// A tick value with only the decimals it needs, at most two: "5", "2,5", "0,32".
fn trimmed_number(value: f64) -> String {
    let whole = |value: f64| (value - value.round()).abs() < 1e-6;
    let decimals = if whole(value) {
        0
    } else if whole(value * 10.0) {
        1
    } else {
        2
    };
    format_number!(value, decimals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sample;

    const LABELLED: ChartSpec = ChartSpec {
        left_gutter: 64.0,
        right_gutter: 68.0,
        hashrate_ticks: true,
        workers_ticks: true,
        x_band: Some(56.0),
        solid_baseline: true,
        grid_steps: 3,
        ticks: DECK_TICKS,
        marker_size: Some(36.0),
    };
    const BARE: ChartSpec = ChartSpec {
        left_gutter: 0.0,
        right_gutter: 0.0,
        hashrate_ticks: false,
        workers_ticks: false,
        x_band: None,
        solid_baseline: false,
        grid_steps: 2,
        ticks: DECK_TICKS,
        marker_size: None,
    };
    const AT_EDGES: ChartSpec = ChartSpec {
        left_gutter: 36.0,
        right_gutter: 40.0,
        hashrate_ticks: true,
        workers_ticks: true,
        x_band: None,
        solid_baseline: false,
        grid_steps: 3,
        ticks: TickSpec {
            font: 14,
            inset: 8.0,
            at_edges: true,
            shared_hashrate_scale: true,
            hero_unit: Some(SiPrefix::Tera),
            shared_worker_scale: true,
        },
        marker_size: None,
    };

    #[expect(
        clippy::cast_precision_loss,
        reason = "eleven tiny sample indices are exact in f64"
    )]
    fn series() -> Series {
        Series {
            from: Some(0),
            to: Some(600),
            samples: (0..=10)
                .map(|i| Sample {
                    at: i * 60,
                    value: 300.0 + i as f64,
                })
                .collect(),
        }
    }

    #[test]
    fn chart_assembles_with_and_without_workers() {
        bmc_wasm_sdk::assets::init_test_registrars();
        let x_labels = [(0.0, "00:00".to_owned()), (1.0, "12:00".to_owned())];
        let markers = [(0.25, PayoutKind::Onchain), (0.8, PayoutKind::Lightning)];
        let _ = line_chart(&series(), None, 620.0, 200.0, &BARE, &[], &[]);
        let _ = line_chart(
            &series(),
            Some(&series()),
            620.0,
            200.0,
            &LABELLED,
            &x_labels,
            &markers,
        );
        let _ = line_chart(
            &Series::default(),
            None,
            620.0,
            200.0,
            &LABELLED,
            &x_labels,
            &markers,
        );
        let _ = line_chart(
            &series(),
            Some(&series()),
            448.0,
            212.0,
            &AT_EDGES,
            &[],
            &[],
        );
    }

    const fn shared(prefix: SiPrefix, lettered: bool) -> HashrateScale {
        HashrateScale::Shared { prefix, lettered }
    }

    #[test]
    fn hashrate_ticks_the_hero_names_are_bare_numbers() {
        let peta = shared(SiPrefix::Peta, false);
        assert_eq!(hashrate_tick(900_000.0, peta), "900");
        assert_eq!(hashrate_tick(0.0, peta), "0");
        assert_eq!(
            hashrate_tick(400_000.0, shared(SiPrefix::Exa, false)),
            "0,4"
        );
        assert_eq!(
            hashrate_tick(320_000.0, shared(SiPrefix::Exa, false)),
            "0,32"
        );
    }

    #[test]
    fn hashrate_ticks_letter_the_prefix_no_hero_names() {
        let tera = shared(SiPrefix::Tera, true);
        assert_eq!(hashrate_tick(3.0, tera), "3T");
        assert_eq!(hashrate_tick(0.0, tera), "0T");
        assert_eq!(hashrate_tick(390_000.0, HashrateScale::OwnPrefix), "390P");
    }

    #[test]
    fn a_top_rounded_into_the_next_prefix_rounds_again_on_a_shared_scale() {
        let top = |max, ticks| hashrate_axis_top(max, ticks);
        let near = |got: f64, want: f64| (got - want).abs() < 1e-6;
        assert!(near(top(995.0, &AT_EDGES.ticks), 1_200.0), "1,02 PH/s");
        assert!(
            near(top(997_000.0, &AT_EDGES.ticks), 1_200_000.0),
            "1,02 EH/s"
        );
        assert!(
            near(top(530.0, &AT_EDGES.ticks), 540.0),
            "no prefix crossed"
        );
        assert!(near(top(0.0, &AT_EDGES.ticks), 3.0), "an idle account");
        assert!(
            near(top(995.0, &DECK_TICKS), 1_020.0),
            "the Deck letters each tick, so keeps its top"
        );
    }

    #[test]
    fn a_shared_hashrate_scale_reads_in_the_axis_top_prefix() {
        let ticks = |hero_unit| TickSpec {
            hero_unit,
            ..AT_EDGES.ticks
        };
        assert_eq!(
            hashrate_scale(&ticks(Some(SiPrefix::Peta)), 540_000.0),
            shared(SiPrefix::Peta, false)
        );
        assert_eq!(
            hashrate_scale(&ticks(Some(SiPrefix::One)), 3.0),
            shared(SiPrefix::Tera, true),
            "an idle hero over the flat axis"
        );
        assert_eq!(
            hashrate_scale(&ticks(Some(SiPrefix::Peta)), 1_200_000.0),
            shared(SiPrefix::Exa, true),
            "an axis past the hero's prefix"
        );
        assert_eq!(
            hashrate_scale(&ticks(None), 540_000.0),
            shared(SiPrefix::Peta, true)
        );
        assert_eq!(
            hashrate_scale(&DECK_TICKS, 540_000.0),
            HashrateScale::OwnPrefix
        );
    }

    #[test]
    fn worker_ticks_on_a_shared_scale_all_take_k_past_a_thousand() {
        let ticks: Vec<String> = [3_000.0, 2_000.0, 1_000.0, 0.0]
            .into_iter()
            .map(|value| workers_tick(value, 3_000.0, true))
            .collect();
        assert_eq!(ticks, ["3k", "2k", "1k", "0k"]);
        assert_eq!(workers_tick(1_500.0, 4_500.0, true), "1,5k");
        assert_eq!(workers_tick(300.0, 900.0, true), "300");
        assert_eq!(workers_tick(0.0, 3_000.0, false), "0");
    }

    #[test]
    fn worker_counts_label_switches_to_k_above_a_thousand() {
        // The digits come from the host number format (locale-dependent
        // decimal mark); only the scaling and suffix are ours to assert.
        assert_eq!(workers_count_label(750.0), "750");
        let scaled = workers_count_label(2_500.0);
        assert!(scaled.ends_with('k'), "{scaled}");
        assert!(scaled.starts_with('2'), "{scaled}");
    }
}

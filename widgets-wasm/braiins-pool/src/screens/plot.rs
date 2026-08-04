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
use crate::screens::parts::{color, font, space};

/// The chart lines' stroke width, and the plot's vertical inset when no
/// y labels claim one.
const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;

/// Vertical plot inset when y labels are on: half a label line plus a
/// little margin, so the top and bottom labels stay inside the canvas.
const Y_LABEL_INSET: f32 = 16.0;

/// Per-layout plot geometry; every field mirrors a knob the design
/// varies between frames.
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
    /// Y-tick font size; smaller than body text so the ticks fit their
    /// gutters with a margin off the frame edge.
    pub tick_font: u32,
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
        Y_LABEL_INSET
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

    let hashrate_top = chart::y_axis_max(series_max(hashrate), false);
    let points = into_plot(chart::decimate(
        chart::series_points(hashrate, hashrate_top, plot_w, plot_h, inset_v),
        max_points,
    ));
    if points.len() >= 2 {
        draws.push(path!(points, stroke: CHART_STROKE, color: color::HASHRATE, smooth));
    }
    if spec.hashrate_ticks {
        // Each tick is its own magnitude's SI prefix letter on the number
        // ("390P", "3,9Z"), mirroring the worker side's "2,5k" — no unit,
        // per the design; H/s is implied.
        for (fraction, value) in axis_levels(hashrate_top) {
            let (number, unit) = Hashrate::from_terahashes_per_second(value).format_si_parts(3);
            let prefix = unit.strip_suffix("H/s").unwrap_or(&unit);
            draws.push(tick_text(
                spec.left_gutter - space::GAP,
                grid_y(fraction),
                fmt!("{number}{prefix}"),
                TextAlign::Right,
                spec.tick_font,
            ));
        }
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
            for (fraction, value) in axis_levels(workers_top) {
                draws.push(tick_text(
                    plot_x(1.0) + space::GAP,
                    grid_y(fraction),
                    workers_count_label(value),
                    TextAlign::Left,
                    spec.tick_font,
                ));
            }
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

/// Worker counts label: plain up to a thousand, "2.5k" above, per the design.
fn workers_count_label(value: f64) -> String {
    if value >= 1_000.0 {
        fmt!("{}k", format_number!(value / 1_000.0, 1))
    } else {
        format_number!(value, 0)
    }
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
        tick_font: 20,
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
        tick_font: 20,
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

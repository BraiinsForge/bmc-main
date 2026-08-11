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

//! Reusable performance overlay — FPS counter, stacked timing chart, and legend.
//!
//! Feature-gated behind `perf-overlay`. The testbed enables it transitively via
//! the `testbed` feature; on-device hosts can opt in with `features = ["perf-overlay"]`.

#![allow(clippy::wildcard_imports)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::integer_division,
    clippy::must_use_candidate
)]

use std::fmt;
use std::time::Instant;

use bmc_render::FrameTimings;
use bmc_render::colors::*;
use bmc_render::renderer::Renderer;

use bmc_wasm_protocol::colors::Color;

// Component colors for timing breakdown
const COL_WASM: Color = Color::from_hex(0x6A_9F_D8); // blue — wasmi interpreter
const COL_TREE: Color = Color::from_hex(0xE0_9A_50); // orange — tree deserialization/parsing
const COL_LAYOUT: Color = Color::from_hex(0xCC_CC_50); // yellow — Taffy layout
const COL_RENDER: Color = Color::from_hex(0x50_CC_50); // green — tree render
const COL_FLUSH: Color = Color::from_hex(0xCC_50_CC); // purple — GPU flush

const HISTORY_LEN: usize = 120;

#[derive(Clone, Copy)]
struct FrameSample {
    us: u32,
    rendered: bool,
    timings: FrameTimings,
}

impl Default for FrameSample {
    fn default() -> Self {
        Self {
            us: 16_000,
            rendered: false,
            timings: FrameTimings::default(),
        }
    }
}

/// Performance overlay that tracks frame timings and draws a stacked bar chart.
///
/// Usage from an on-device host:
/// ```ignore
/// use bmc_wasm_runtime::perf_overlay::PerfOverlay;
///
/// let mut overlay = PerfOverlay::new();
/// // in render loop:
/// overlay.tick(frame_us, rendered, runtime.last_timings());
/// overlay.draw(renderer, w, h, 0.0);
/// ```
pub struct PerfOverlay {
    last_update: Instant,
    loop_count: u32,
    render_count: u32,
    display_render: u32,
    history: [FrameSample; HISTORY_LEN],
    history_idx: usize,
    /// Cached averages — held when idle so the legend doesn't decay to zero.
    cached_avg_us: u32,
    cached_avg_timings: FrameTimings,
}

impl fmt::Debug for PerfOverlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PerfOverlay")
            .field("display_render", &self.display_render)
            .field("history_idx", &self.history_idx)
            .finish_non_exhaustive()
    }
}

impl PerfOverlay {
    pub fn new() -> Self {
        Self {
            last_update: Instant::now(),
            loop_count: 0,
            render_count: 0,
            display_render: 0,
            history: [FrameSample::default(); HISTORY_LEN],
            history_idx: 0,
            cached_avg_us: 0,
            cached_avg_timings: FrameTimings::default(),
        }
    }

    /// Record a frame sample.
    ///
    /// All frames are pushed to history (idle frames show as gray bars in the
    /// chart). Cached averages are updated only when rendered frames exist in
    /// the window, so the legend holds last-known values during idle instead
    /// of decaying to zero.
    pub fn tick(&mut self, us: u32, rendered: bool, timings: FrameTimings) {
        self.loop_count += 1;
        if rendered {
            self.render_count += 1;
        }
        self.history[self.history_idx] = FrameSample {
            us,
            rendered,
            timings,
        };
        self.history_idx = (self.history_idx + 1) % self.history.len();

        // Update cached averages only when there's rendered data in the window
        let live_avg = self.compute_avg_us();
        if live_avg > 0 {
            self.cached_avg_us = live_avg;
            self.cached_avg_timings = self.compute_avg_timings();
        }

        if self.last_update.elapsed().as_secs() >= 1 {
            self.display_render = self.render_count;
            self.loop_count = 0;
            self.render_count = 0;
            self.last_update = Instant::now();
        }
    }

    /// Draw the full overlay (FPS text + stacked bar chart + legend).
    ///
    /// `y_offset` shifts everything down (e.g. to leave room for a button above).
    #[expect(clippy::cast_precision_loss)]
    pub fn draw(&self, renderer: &mut dyn Renderer, w: f32, h: f32, y_offset: f32) {
        let pad = 8.0;
        let mut y = y_offset;

        // ── Row: avg ms + fps ──
        let avg_us = self.avg_render_us();
        let avg_ms = avg_us as f32 / 1_000.0;
        let ms_color = if avg_us > 16_000 { RED_50 } else { GREEN_30 };
        let ms_text = format!("{avg_ms:.1}ms");
        renderer.draw_text(&ms_text, pad, y, 13.0, ms_color);
        let ms_w = renderer.measure_text(&ms_text, 13.0);
        if self.display_render > 0 {
            let fps_text = format!("{}fps", self.display_render);
            renderer.draw_text(&fps_text, pad + ms_w + 8.0, y, 13.0, GRAY_40);
        } else {
            renderer.draw_text("idle", pad + ms_w + 8.0, y, 13.0, GRAY_60);
        }
        y += 18.0;

        // ── Chart ──
        let chart_x = pad;
        let chart_top = y;
        let legend_font = 12.0;
        let legend_h = legend_font + 6.0;
        let chart_bottom = h - pad - legend_h;
        let chart_h = chart_bottom - chart_top;
        let chart_w = w - pad * 2.0;
        let bar_w = chart_w / self.history.len() as f32;
        let scale_us = 20_000.0_f32;

        // Gridlines (drawn before bars so bars paint over them)
        let axis_font = 10.0;
        for &grid_us in &[4_000, 8_000, 16_000] {
            let gy = chart_top + chart_h - (grid_us as f32 * chart_h / scale_us);
            if gy > chart_top && gy < chart_bottom {
                renderer.fill_rect(chart_x, gy, chart_w, 1.0, GRAY_90);
            }
        }

        // Bars — snap to pixel grid to avoid subpixel gaps
        for (i, sample) in self.samples().enumerate() {
            let bx = (chart_x + i as f32 * bar_w).floor();
            let bx_next = (chart_x + (i as f32 + 1.0) * bar_w)
                .floor()
                .min(chart_x + chart_w);
            let bw = bx_next - bx;
            if !sample.rendered {
                let bh = (sample.us as f32 * chart_h / scale_us).min(chart_h);
                renderer.fill_rect(bx, chart_top + chart_h - bh, bw, bh, GRAY_80);
                continue;
            }
            let t = &sample.timings;
            let segments: [(u32, Color); 5] = [
                (t.flush_us, COL_FLUSH),
                (t.render_us, COL_RENDER),
                (t.layout_us, COL_LAYOUT),
                (t.deserialize_us, COL_TREE),
                (
                    t.wasm_us
                        .saturating_sub(t.deserialize_us + t.layout_us + t.render_us),
                    COL_WASM,
                ),
            ];
            let mut y_off = 0.0_f32;
            for (us, col) in segments {
                let bh = (us as f32 * chart_h / scale_us).min(chart_h - y_off);
                if bh > 0.5 {
                    renderer.fill_rect(bx, chart_top + chart_h - y_off - bh, bw, bh, col);
                }
                y_off += bh;
            }
        }

        // Axis tick labels (drawn on top of bars with black background)
        let tick_pad = 2.0;
        for &grid_us in &[4_000, 8_000, 16_000] {
            let gy = chart_top + chart_h - (grid_us as f32 * chart_h / scale_us);
            if gy > chart_top && gy < chart_bottom {
                let label = format!("{}", grid_us / 1_000);
                let lw = renderer.measure_text(&label, axis_font);
                let lx = chart_x + chart_w - lw - tick_pad;
                let ly = gy - axis_font - 1.0;
                renderer.fill_rect(
                    lx - tick_pad,
                    ly,
                    lw + tick_pad * 2.0,
                    axis_font + 2.0,
                    BLACK,
                );
                renderer.draw_text(&label, lx, ly, axis_font, GRAY_60);
            }
        }

        // ── Legend (below chart) ──
        let avg = self.avg_timings();
        let legend_y = chart_bottom + 4.0;
        let mut lx = pad;
        for (label, us, col) in [
            ("WASM", avg.wasm_us, COL_WASM),
            ("Tree", avg.deserialize_us, COL_TREE),
            ("Lay", avg.layout_us, COL_LAYOUT),
            ("RNDR", avg.render_us, COL_RENDER),
            ("GPU", avg.flush_us, COL_FLUSH),
        ] {
            let txt = format!("{label} {:.1}", us as f32 / 1_000.0);
            renderer.draw_text(&txt, lx, legend_y, legend_font, col);
            lx += renderer.measure_text(&txt, legend_font) + 6.0;
        }
    }

    /// Average render time in microseconds (holds last value when idle).
    pub fn avg_render_us(&self) -> u32 {
        self.cached_avg_us
    }

    /// Average per-component timings (holds last values when idle).
    pub fn avg_timings(&self) -> FrameTimings {
        self.cached_avg_timings
    }

    /// Live average render time from the history window.
    fn compute_avg_us(&self) -> u32 {
        let (sum, count) = self
            .history
            .iter()
            .filter(|s| s.rendered)
            .fold((0_u32, 0_u32), |(sum, count), s| (sum + s.us, count + 1));
        sum.checked_div(count).unwrap_or(0)
    }

    /// Live average per-component timings from the history window.
    fn compute_avg_timings(&self) -> FrameTimings {
        let rendered: Vec<_> = self.history.iter().filter(|s| s.rendered).collect();
        let n = rendered.len() as u32;
        if n == 0 {
            return FrameTimings::default();
        }
        FrameTimings {
            wasm_us: rendered.iter().map(|s| s.timings.wasm_us).sum::<u32>() / n,
            deserialize_us: rendered
                .iter()
                .map(|s| s.timings.deserialize_us)
                .sum::<u32>()
                / n,
            layout_us: rendered.iter().map(|s| s.timings.layout_us).sum::<u32>() / n,
            render_us: rendered.iter().map(|s| s.timings.render_us).sum::<u32>() / n,
            flush_us: rendered.iter().map(|s| s.timings.flush_us).sum::<u32>() / n,
        }
    }

    /// FPS counter for display (updated once per second).
    pub fn display_fps(&self) -> u32 {
        self.display_render
    }

    /// Iterate samples in chronological order.
    fn samples(&self) -> impl Iterator<Item = &FrameSample> {
        let (a, b) = self.history.split_at(self.history_idx);
        b.iter().chain(a.iter())
    }

    /// Access the most recent sample (for perf report collection).
    pub fn last_sample_timings(&self) -> FrameTimings {
        let idx = (self.history_idx + self.history.len() - 1) % self.history.len();
        self.history[idx].timings
    }
}

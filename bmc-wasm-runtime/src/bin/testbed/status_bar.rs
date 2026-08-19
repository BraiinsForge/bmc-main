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

//! Bottom status bar: what the host spends on a frame.
//!
//! Only figures that belong to the whole testbed. A per-frame breakdown
//! belongs to one view, and with every supported device open no single view
//! can speak for the rest — those numbers go on the views themselves.

use super::{TestbedApp, paint_timing_chart, paint_timing_legend, view::DeviceView};

/// One row of readouts, at an 11 pt figure's own line and no more: stacked,
/// any slack here reads as a gap between the two rows rather than as air.
const READOUT_ROW_H: f32 = 16.0;

/// Set by the sparkline, not by the rows: the chart needs height to draw a
/// widget's few hundred µs against a frame budget, and the rows centre in
/// whatever that leaves.
pub(super) const STATUS_H: f32 = 40.0;

/// Width of the sparkline that closes the row.
const SPARK_W: f32 = 180.0;

/// The most samples the sparkline can draw — one per column. Anything older
/// scrolls off, so a run that keeps no report has no reason to hold it.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a positive ratio of two literals"
)]
pub(super) const SPARK_SAMPLES: usize = (SPARK_W / super::paint::CHART_COL_W) as usize;

/// Vertical air around the sparkline inside the bar.
const SPARK_INSET: f32 = 4.0;

/// The chart's key, shown on hovering it.
const LEGEND_W: f32 = 300.0;
const LEGEND_H: f32 = 14.0;

/// Between a caption and its values — tighter than the row's own spacing,
/// so a value belongs to the caption on its left and not to the next group.
const VALUE_GAP: f32 = 6.0;

/// One row of the stacked readouts, its figures centred in it.
fn readout_row(column: &mut egui::Ui, groups: impl FnOnce(&mut egui::Ui)) {
    let width = column.available_width();
    column.allocate_ui(egui::vec2(width, READOUT_ROW_H), |slot| {
        slot.horizontal_centered(groups);
    });
}

/// A caption and the figures it names, kept together.
fn stat_group(row: &mut egui::Ui, caption: &str, values: &[String]) {
    row.scope(|group| {
        group.spacing_mut().item_spacing.x = VALUE_GAP;
        group.add(super::ui_helpers::key_caption(caption));
        // Values are padded to a fixed width, so a number widening between
        // frames does not shuffle the rest of the row along.
        let strong = group.visuals().strong_text_color();
        for value in values {
            group.label(
                egui::RichText::new(value)
                    .font(egui::FontId::monospace(11.0))
                    .color(strong),
            );
        }
    });
}

impl TestbedApp {
    pub(super) fn paint_status_bar(&mut self, root_ui: &mut egui::Ui) {
        let palette = self.theme.palette(root_ui.ctx());
        // Chrome, like the toolbar: the panel only reserves space and the
        // foreground area paints it, so canvas windows pass underneath.
        let panel = egui::TopBottomPanel::bottom("status")
            .exact_height(STATUS_H)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show_inside(root_ui, |_| {});
        let rect = panel.response.rect;
        egui::Area::new(egui::Id::new("status_chrome"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(root_ui.ctx(), |area| {
                area.set_clip_rect(rect);
                area.painter().rect_filled(rect, 0.0, palette.layer);
                let mut bar = area.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(egui::vec2(super::toolbar::BAR_INLINE_PAD, 0.0))),
                );
                bar.horizontal_centered(|row| {
                    // The numbers stack; the chart stands beside both rows,
                    // spanning them, since it annotates the pair.
                    row.vertical(|column| {
                        // The two rows are the bar: any spacing between them
                        // is height they then have to share.
                        column.spacing_mut().item_spacing.y = 0.0;
                        // A vertical fills the height it is given and lays out
                        // from the top, so the pair is centred by hand.
                        column.add_space((STATUS_H - 2.0 * READOUT_ROW_H) / 2.0);
                        self.paint_readouts(column, palette);
                    });
                    self.paint_sparkline(row, palette);
                });
                // In the window's corner, the same inset off each edge: it
                // reports on the run rather than on either row of figures.
                let mut corner = area.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink(super::toolbar::BAR_INLINE_PAD))
                        .layout(egui::Layout::right_to_left(egui::Align::Max)),
                );
                self.paint_hot_chip(&mut corner);
            });
    }

    /// What the host spent, over what the widgets asked of it.
    fn paint_readouts(&self, column: &mut egui::Ui, palette: &super::theme::Palette) {
        let avg_us = if self.perf.recent_frame_us.is_empty() {
            0
        } else {
            let sum: u32 = self.perf.recent_frame_us.iter().sum();
            sum / self.perf.recent_frame_us.len() as u32
        };
        let fps = if avg_us > 0 {
            1_000_000.0 / avg_us as f32
        } else {
            0.0
        };
        // Summed, not sampled: every open view spends its own wasm time on
        // the same host frame, and one view's figure would speak for none.
        let live: Vec<&DeviceView> = self.stage.tiles().iter().filter(|v| v.is_live()).collect();
        let wasm_us: u32 = live
            .iter()
            .filter_map(|view| view.last_timings())
            .map(|t| t.wasm_us)
            .sum();
        // The worst across the views: one view falling behind is the thing
        // worth surfacing, and an average would bury it.
        let slip = live.iter().filter_map(|view| view.last_slip_ms()).max();

        readout_row(column, |row| {
            stat_group(
                row,
                "host frame",
                &[format!("{avg_us:>6} µs"), format!("{fps:>5.1} fps")],
            );
        });
        readout_row(column, |row| {
            stat_group(
                row,
                "widgets",
                &[
                    format!("{wasm_us:>6} µs"),
                    format!("{:>2} views", live.len()),
                ],
            );
            super::ui_helpers::group_divider(row, palette.border_subtle, READOUT_ROW_H);
            stat_group(
                row,
                "slip",
                &[slip.map_or_else(|| "   — ms".to_owned(), |ms| format!("{ms:>4} ms"))],
            );
        });
    }

    fn paint_sparkline(&self, row: &mut egui::Ui, palette: &super::theme::Palette) {
        if self.perf.samples.is_empty() {
            return;
        }
        let (rect, response) = row.allocate_exact_size(
            egui::vec2(SPARK_W, STATUS_H - 2.0 * SPARK_INSET),
            egui::Sense::hover(),
        );
        // `painter_at` clips the chart so a spike cannot bleed into the row.
        paint_timing_chart(&row.painter_at(rect), rect, &self.perf.samples, palette);
        // The bar has no room for a key, so the chart carries its own.
        response.on_hover_ui(|tip| {
            let (key, _) =
                tip.allocate_exact_size(egui::vec2(LEGEND_W, LEGEND_H), egui::Sense::hover());
            paint_timing_legend(tip.painter(), key, tip.visuals().text_color(), palette);
        });
    }
}

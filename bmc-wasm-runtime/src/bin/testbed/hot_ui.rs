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

//! What the rebuild cycle looks like: a chip in the status bar, and — while a
//! build is failing — what cargo said, over the canvas.
//!
//! The widget on screen is the last one that built, so a failure has to say
//! so: without it the canvas looks like an edit that simply did nothing.

use super::TestbedApp;
use super::hot::{BuildFailure, HotPhase};
use super::theme::spacing;

/// The box the chip's mark is drawn in, and the gap between it and the words.
const MARK: f32 = 9.0;
const MARK_GAP: f32 = 5.0;

/// The failure box: the widest it grows to, the share of the canvas its log
/// may take before scrolling, and its inner inset.
const BOX_MAX_W: f32 = 720.0;
const BOX_MAX_H_SHARE: f32 = 0.65;
const BOX_PAD: i8 = spacing::S05 as i8;

/// The header's mark and its type.
const HEAD_ICON: f32 = 16.0;
const HEAD_TEXT: f32 = 15.0;

/// How a phase reads on the chip.
struct Chip {
    text: String,
    colour: egui::Color32,
    hover: String,
}

impl TestbedApp {
    /// The chip: where the cycle has got to, in a word.
    ///
    /// Its mark is drawn rather than typed, because a glyph comes from
    /// whichever fallback face has it, at that face's weight and baseline.
    pub(super) fn paint_hot_chip(&mut self, ui: &mut egui::Ui) {
        let palette = self.theme.palette(ui.ctx());
        let phase = self.hot_reload.status.phase();
        let chip = describe(&phase, palette);
        let words = ui.painter().layout_no_wrap(
            chip.text,
            egui::TextStyle::Body.resolve(ui.style()),
            chip.colour,
        );

        let size = egui::vec2(MARK + MARK_GAP + words.size().x, words.size().y.max(MARK));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        let mark = egui::pos2(rect.left() + MARK / 2.0, rect.center().y);
        paint_mark(ui.painter(), mark, &phase, chip.colour);
        ui.painter().galley(
            egui::pos2(
                mark.x + MARK / 2.0 + MARK_GAP,
                rect.center().y - words.size().y / 2.0,
            ),
            words,
            chip.colour,
        );
        response.on_hover_text(chip.hover);
    }

    /// What cargo said, over the canvas, for as long as the build is failing.
    ///
    /// A foreground area rather than a panel: egui stacks the device windows
    /// above panels, and one of them would cover it.
    pub(super) fn paint_build_failure(&mut self, ctx: &egui::Context) {
        let HotPhase::Failed(failure) = self.hot_reload.status.phase() else {
            return;
        };
        let palette = self.theme.palette(ctx);
        let icon = &mut self.icons.warning;

        // Last frame's: the CentralPanel measures the canvas further down,
        // and only a sidebar toggle moves it.
        let over = self.canvas.rect;
        // The box is content-sized, so its centre is reached as an offset
        // from the rect `Area` anchors within.
        let off_centre = over.center() - ctx.content_rect().center();
        let width = BOX_MAX_W.min(over.width());
        let inner = width - f32::from(BOX_PAD) * 2.0;
        let budget = over.height() * BOX_MAX_H_SHARE;

        egui::Area::new(egui::Id::new("build_failure"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, off_centre)
            .show(ctx, |area| {
                area.painter()
                    .with_clip_rect(over)
                    .rect_filled(over, 0.0, palette.backdrop);
                // `Area` sizes this ui from the previous frame, so a `ScrollArea`
                // in it never grows past what it first settled at.
                area.set_max_height(budget);
                egui::Frame::NONE
                    .fill(palette.layer)
                    .stroke(egui::Stroke::new(1.0_f32, palette.border_subtle))
                    .shadow(super::theme::OVERLAY_SHADOW)
                    .show(area, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        egui::Frame::NONE
                            .fill(palette.support_error_wash())
                            .inner_margin(egui::Margin::same(BOX_PAD))
                            .show(ui, |head| {
                                head.set_width(inner);
                                head.horizontal(|row| {
                                    row.spacing_mut().item_spacing.x = spacing::S03;
                                    let (mark, _) = row.allocate_exact_size(
                                        egui::Vec2::splat(HEAD_ICON),
                                        egui::Sense::hover(),
                                    );
                                    icon.paint(row, mark, palette.support_error);
                                    row.label(
                                        egui::RichText::new(headline(&failure))
                                            .color(palette.support_error)
                                            .strong()
                                            .size(HEAD_TEXT),
                                    );
                                });
                            });
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::same(BOX_PAD))
                            .show(ui, |body| {
                                body.set_width(inner);
                                egui::ScrollArea::vertical()
                                    .max_height(budget)
                                    // Width fills; height collapses for a short log.
                                    .auto_shrink([false, true])
                                    .show(body, |ui| paint_report(ui, &failure, palette));
                            });
                    });
            });
    }
}

/// The mark beside the words: a dot at rest, a breathing one at work, a tick
/// or a cross at the end.
fn paint_mark(painter: &egui::Painter, at: egui::Pos2, phase: &HotPhase, colour: egui::Color32) {
    let radius = MARK / 2.0;
    let line = |from: egui::Vec2, to: egui::Vec2| {
        painter.line_segment(
            [at + from * radius, at + to * radius],
            egui::Stroke::new(1.6_f32, colour),
        );
    };
    match phase {
        HotPhase::Watching => {
            painter.circle_filled(at, radius * 0.5, colour);
        }
        HotPhase::Changed | HotPhase::Building { .. } | HotPhase::Swapping { .. } => {
            // Breathing, because a build under way should not look like one
            // at rest.
            let breath = 0.45 + 0.55 * (painter.ctx().input(|i| i.time) as f32 * 4.0).sin().abs();
            painter.circle_filled(at, radius * 0.8, colour.gamma_multiply(breath));
        }
        HotPhase::Reloaded { .. } => {
            line(egui::vec2(-0.85, 0.05), egui::vec2(-0.2, 0.6));
            line(egui::vec2(-0.2, 0.6), egui::vec2(0.85, -0.65));
        }
        HotPhase::Failed(_) | HotPhase::Stopped { .. } => {
            line(egui::vec2(-0.7, -0.7), egui::vec2(0.7, 0.7));
            line(egui::vec2(-0.7, 0.7), egui::vec2(0.7, -0.7));
        }
    }
}

fn describe(phase: &HotPhase, palette: &super::theme::Palette) -> Chip {
    let (text, colour, hover) = match phase {
        HotPhase::Watching => (
            "Watching".to_owned(),
            palette.text_disabled,
            "editing the widget's source rebuilds it".to_owned(),
        ),
        HotPhase::Changed => (
            "Changed".to_owned(),
            palette.support_warning,
            "an edit landed; building once it stops".to_owned(),
        ),
        HotPhase::Building { since } => (
            format!("Building {:.1}s", since.elapsed().as_secs_f32()),
            palette.support_warning,
            "cargo is rebuilding the widget".to_owned(),
        ),
        HotPhase::Swapping { .. } => (
            "Swapping".to_owned(),
            palette.support_warning,
            "built; loading it into the views".to_owned(),
        ),
        HotPhase::Reloaded { took, .. } => (
            format!("Reloaded · {:.1}s", took.as_secs_f32()),
            palette.support_success,
            "what the views run is the edit's".to_owned(),
        ),
        HotPhase::Failed(_) => (
            "Build failed".to_owned(),
            palette.support_error,
            "the views run the last widget that built".to_owned(),
        ),
        HotPhase::Stopped { why } => (
            "Not watching".to_owned(),
            palette.support_error,
            why.clone(),
        ),
    };
    Chip {
        text,
        colour,
        hover,
    }
}

/// What the bar says: the count where the messages carry one, and the fact
/// where they do not.
fn headline(failure: &BuildFailure) -> String {
    match failure.errors {
        0 => "Build failed".to_owned(),
        1 => "Build failed — 1 error".to_owned(),
        errors => format!("Build failed — {errors} errors"),
    }
}

/// The whole of what cargo said, coloured by rustc's own markup.
fn paint_report(ui: &mut egui::Ui, failure: &BuildFailure, palette: &super::theme::Palette) {
    ui.spacing_mut().item_spacing.y = spacing::S03;
    let theme = report_theme(ui, palette);
    for message in &failure.messages {
        ui.label(egui_sgr::ansi_to_layout_job(
            message.text.trim_end(),
            &theme,
        ));
    }
}

/// rustc's SGR palette, re-pointed at the theme's.
///
/// Only the eight basic slots move, with their bright twins: rustc colours
/// diagnostics out of those. Blue deliberately lands on `text_disabled`,
/// since rustc spends it on the gutter and the `-->` line, not on content.
fn report_theme(ui: &egui::Ui, palette: &super::theme::Palette) -> egui_sgr::EguiAnsiTheme {
    let base = ui.visuals().text_color();
    let mut theme = egui_sgr::EguiAnsiTheme::xterm();
    theme.default_format = egui::TextFormat {
        font_id: egui::TextStyle::Monospace.resolve(ui.style()),
        color: base,
        ..Default::default()
    };
    theme.default_foreground = base;
    theme.default_background = egui::Color32::TRANSPARENT;
    for (slot, colour) in [
        palette.text_disabled,
        palette.support_error,
        palette.support_success,
        palette.support_warning,
        palette.text_disabled,
        palette.action_primary,
        palette.action_primary,
        base,
    ]
    .into_iter()
    .enumerate()
    {
        theme.palette[slot] = colour;
        theme.palette[slot + 8] = colour;
    }
    theme
}

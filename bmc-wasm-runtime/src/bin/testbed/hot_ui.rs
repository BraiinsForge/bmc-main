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
//! build is failing — a bar over the canvas that opens what cargo said.
//!
//! The widget on screen is the last one that built, so a failure has to say
//! so: without it the canvas looks like an edit that simply did nothing.

use super::TestbedApp;
use super::hot::{BuildFailure, HotPhase, MessageLevel};

/// Gold while something is happening, green for a cycle that came out, red
/// for one that did not.
const BUSY: egui::Color32 = egui::Color32::from_rgb(0xC8, 0x9B, 0x3C);
const DONE: egui::Color32 = egui::Color32::from_rgb(0x6C, 0xC8, 0x7A);
const BROKEN: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x6C, 0x6C);

/// What the failure bar is filled with: dark enough not to glow, loud enough
/// not to read as chrome.
const BAR_FILL: egui::Color32 = egui::Color32::from_rgb(0x4A, 0x1D, 0x1D);

/// The box the chip's mark is drawn in, and the gap between it and the words.
const MARK: f32 = 9.0;
const MARK_GAP: f32 = 5.0;

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
        let phase = self.hot_reload.status.phase();
        let chip = describe(&phase);
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

    /// The bar over the canvas while a build is failing, and the report a
    /// click on it opens.
    pub(super) fn paint_build_failure(&mut self, ctx: &egui::Context) {
        let HotPhase::Failed(failure) = self.hot_reload.status.phase() else {
            self.hot_reload.report_open = false;
            return;
        };

        let mut clicked = false;
        egui::TopBottomPanel::top("build_failure")
            .frame(
                egui::Frame::NONE
                    .fill(BAR_FILL)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                let bar = ui
                    .horizontal(|row| {
                        row.label(
                            egui::RichText::new(headline(&failure))
                                .color(BROKEN)
                                .strong(),
                        );
                        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |end| {
                            end.label(
                                egui::RichText::new("Click for what cargo said")
                                    .color(end.visuals().weak_text_color()),
                            );
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if bar.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                clicked = bar.clicked();
            });
        if clicked {
            self.hot_reload.report_open = !self.hot_reload.report_open;
        }

        if self.hot_reload.report_open {
            let report = egui::Modal::new(egui::Id::new("build_report")).show(ctx, |ui| {
                // The modal's bound, not the report's, so a long build log
                // scrolls rather than growing the window off the screen.
                ui.set_max_size(ctx.content_rect().size() * 0.8);
                paint_report(ui, &failure);
            });
            if report.should_close() {
                self.hot_reload.report_open = false;
            }
        }
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

fn describe(phase: &HotPhase) -> Chip {
    let (text, colour, hover) = match phase {
        HotPhase::Watching => (
            "Watching".to_owned(),
            egui::Color32::from_gray(140),
            "editing the widget's source rebuilds it".to_owned(),
        ),
        HotPhase::Changed => (
            "Changed".to_owned(),
            BUSY,
            "an edit landed; building once it stops".to_owned(),
        ),
        HotPhase::Building { since } => (
            format!("Building {:.1}s", since.elapsed().as_secs_f32()),
            BUSY,
            "cargo is rebuilding the widget".to_owned(),
        ),
        HotPhase::Swapping { .. } => (
            "Swapping".to_owned(),
            BUSY,
            "built; loading it into the views".to_owned(),
        ),
        HotPhase::Reloaded { took, .. } => (
            format!("Reloaded · {:.1}s", took.as_secs_f32()),
            DONE,
            "what the views run is the edit's".to_owned(),
        ),
        HotPhase::Failed(_) => (
            "Build failed".to_owned(),
            BROKEN,
            "the views run the last widget that built".to_owned(),
        ),
        HotPhase::Stopped { why } => ("Not watching".to_owned(), BROKEN, why.clone()),
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

/// The whole of what cargo said, each message in the colour of what it weighs.
fn paint_report(ui: &mut egui::Ui, failure: &BuildFailure) {
    ui.horizontal(|row| {
        row.label(
            egui::RichText::new("What cargo said")
                .color(BROKEN)
                .strong(),
        );
        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |end| {
            if end.button("Close").clicked() {
                end.close();
            }
        });
    });
    ui.separator();
    egui::ScrollArea::both().show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        for message in &failure.messages {
            let colour = match message.level {
                MessageLevel::Error => BROKEN,
                MessageLevel::Warning => BUSY,
                MessageLevel::Note => ui.visuals().text_color(),
            };
            ui.label(
                egui::RichText::new(message.text.trim_end())
                    .monospace()
                    .color(colour),
            );
        }
    });
}

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

//! Knob rendering for the Controls panel (egui-dependent).
//!
//! Extracted from `StoryCtx` so that egui types stay out of the API crate
//! (which crosses the dlopen boundary).

use bmc_render::colors::Color;
use bmc_storybook_api::knobs::{Knob, StoryCtx};

/// Render all registered knobs as egui widgets in the controls panel.
///
/// Returns `true` if any knob value changed this frame.
pub fn render_knobs_ui(ctx: &mut StoryCtx, ui: &mut egui::Ui) -> bool {
    let mut changed = false;
    if ctx.knobs_mut().is_empty() {
        ui.label(
            egui::RichText::new("No controls for this story")
                .color(crate::to_egui(bmc_render::colors::GRAY_60))
                .italics(),
        );
        return false;
    }
    egui::Grid::new("knobs_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            // `prev_was_group` lets us skip the inter-knob gap right after a
            // `Group` header — its own separator already provides the break.
            let mut prev_was_group = false;
            for (i, knob) in ctx.knobs_mut().iter_mut().enumerate() {
                changed |= render_knob(ui, knob, i == 0, prev_was_group);
                prev_was_group = matches!(knob, Knob::Group { .. });
            }
        });
    changed
}

/// Decimal places a step needs for its slider label (`1` → 0, `0.1` → 1,
/// `0.01` → 2). Iterated, not `ceil(-log10)`: f32 rounding makes `log10(0.01)`
/// ≈ -2.0000001, so `ceil` would give a wrong 3.
fn step_decimals(step: f32) -> usize {
    let mut decimals = 0;
    let mut scaled = f64::from(step);
    while decimals < 8 && (scaled - scaled.round()).abs() > 1e-6 {
        scaled *= 10.0;
        decimals += 1;
    }
    decimals
}

/// Render a slider (stepped when `step > 0`) and report whether it changed.
fn slider_ui(ui: &mut egui::Ui, value: &mut f32, min: f32, max: f32, step: f32) -> bool {
    let mut slider = egui::Slider::new(value, min..=max);
    if step > 0.0 {
        // egui shows two decimals by default; match the step so "5" isn't "5.00".
        slider = slider
            .step_by(f64::from(step))
            .max_decimals(step_decimals(step));
    }
    ui.add(slider).changed()
}

/// Render a single knob in the grid. Returns `true` if the value changed.
fn render_knob(ui: &mut egui::Ui, knob: &mut Knob, first: bool, prev_was_group: bool) -> bool {
    let mut changed = false;
    // Visual gap between consecutive non-grouped knobs so two radio groups
    // (or any pair of multi-row knobs) don't blur into a single column.
    // Skipped on the first knob and right after a `Group` header (which
    // already inserts its own gap + separator).
    if !first && !prev_was_group && !matches!(knob, Knob::Group { .. }) {
        ui.allocate_space(egui::vec2(0.0, 8.0));
        ui.end_row();
    }
    match knob {
        Knob::Group { label } => {
            if !first {
                ui.allocate_space(egui::vec2(0.0, 8.0));
                ui.end_row();
            }
            ui.label(
                egui::RichText::new(label.as_str())
                    .size(16.0)
                    .color(crate::to_egui(bmc_render::colors::GRAY_60))
                    .small(),
            );
            ui.separator();
        }
        Knob::Text { label, value } => {
            ui.label(label.as_str());
            changed = ui
                .add(egui::TextEdit::singleline(value).margin(egui::Margin::symmetric(4, 4)))
                .changed();
        }
        Knob::Slider {
            label,
            value,
            min,
            max,
            step,
        } => {
            ui.label(label.as_str());
            changed = slider_ui(ui, value, *min, *max, *step);
        }
        Knob::Toggle { label, value } => {
            // Make the label itself clickable so the user can hit either
            // the label or the checkbox to toggle the value.
            let label_resp = ui.add(egui::Label::new(label.as_str()).sense(egui::Sense::click()));
            if label_resp.clicked() {
                *value = !*value;
                changed = true;
            }
            changed |= ui.checkbox(value, "").changed();
        }
        Knob::Select {
            label,
            value,
            options,
            radio,
        } => {
            ui.label(label.as_str());
            if *radio {
                ui.vertical(|ui| {
                    for (i, opt) in options.iter().enumerate() {
                        changed |= ui.radio_value(value, i, opt.as_str()).changed();
                    }
                });
            } else {
                let selected = options.get(*value).map_or("", String::as_str);
                egui::ComboBox::from_id_salt(label.as_str())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, opt) in options.iter().enumerate() {
                            changed |= ui.selectable_value(value, i, opt.as_str()).changed();
                        }
                    });
            }
        }
        Knob::Color { label, value } => {
            let r = f32::from(value.red()) / 255.0;
            let g = f32::from(value.green()) / 255.0;
            let b = f32::from(value.blue()) / 255.0;
            let mut color = [r, g, b];
            ui.label(label.as_str());
            if ui.color_edit_button_rgb(&mut color).changed() {
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let hex = ((color[0] * 255.0) as u32) << 16
                    | ((color[1] * 255.0) as u32) << 8
                    | (color[2] * 255.0) as u32;
                *value = Color::from_hex(hex);
                changed = true;
            }
        }
        Knob::Pad2D {
            label,
            x,
            y,
            min_x,
            max_x,
            min_y,
            max_y,
            invert_y,
        } => {
            ui.label(label.as_str());
            changed |= render_pad2d(ui, x, y, *min_x, *max_x, *min_y, *max_y, *invert_y);
        }
    }
    ui.end_row();
    changed
}

/// Pad size in egui logical pixels — fixed (not stretched to column width)
/// so pads don't dominate the controls panel when the column is wide.
const PAD2D_SIZE: f32 = 80.0;

/// Render a 2-axis touchpad knob and update `*x`, `*y` on drag/click.
/// Returns `true` if the value changed.
///
/// `invert_y = true` flips screen-Y → value-Y so dragging up gives `max_y`
/// (Blender-style camera/orientation pads). `false` keeps the direct
/// mapping (top of pad = `min_y`).
#[expect(
    clippy::too_many_arguments,
    reason = "two values + two ranges + axis-orientation flag — not a meaningful target for a struct"
)]
fn render_pad2d(
    ui: &mut egui::Ui,
    x: &mut f32,
    y: &mut f32,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    invert_y: bool,
) -> bool {
    let mut changed = false;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(PAD2D_SIZE, PAD2D_SIZE),
        egui::Sense::click_and_drag(),
    );

    let range_x = max_x - min_x;
    let range_y = max_y - min_y;
    // `screen_y` ∈ [0, 1] = pointer's vertical position within the pad
    // (0 = top edge of pad, 1 = bottom). `to_value_y` and `from_value_y`
    // are the only two places `invert_y` is consulted, so the orientation
    // choice lives in one place per direction.
    let to_value_y = |screen_y: f32| -> f32 {
        let t = if invert_y { 1.0 - screen_y } else { screen_y };
        min_y + t * range_y
    };
    let from_value_y = |value_y: f32| -> f32 {
        if range_y <= 0.0 {
            return 0.5;
        }
        let t = (value_y - min_y) / range_y;
        if invert_y { 1.0 - t } else { t }
    };

    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let nx = ((pos.x - rect.min.x) / PAD2D_SIZE).clamp(0.0, 1.0);
        let ny = ((pos.y - rect.min.y) / PAD2D_SIZE).clamp(0.0, 1.0);
        let new_x = min_x + nx * range_x;
        let new_y = to_value_y(ny);
        if (new_x - *x).abs() > f32::EPSILON || (new_y - *y).abs() > f32::EPSILON {
            *x = new_x;
            *y = new_y;
            changed = true;
        }
    }

    let painter = ui.painter_at(rect);
    let bg = crate::to_egui(bmc_render::colors::GRAY_90);
    let border = crate::to_egui(bmc_render::colors::GRAY_70);
    let cross = crate::to_egui(bmc_render::colors::GRAY_80);
    let dot_color = crate::to_egui(bmc_render::colors::BLUE_50);

    painter.rect_filled(rect, 4.0, bg);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );

    let mid = rect.center();
    painter.line_segment(
        [
            egui::pos2(mid.x, rect.min.y + 4.0),
            egui::pos2(mid.x, rect.max.y - 4.0),
        ],
        egui::Stroke::new(1.0_f32, cross),
    );
    painter.line_segment(
        [
            egui::pos2(rect.min.x + 4.0, mid.y),
            egui::pos2(rect.max.x - 4.0, mid.y),
        ],
        egui::Stroke::new(1.0_f32, cross),
    );

    let norm_x = if range_x > 0.0 {
        (*x - min_x) / range_x
    } else {
        0.5
    };
    let norm_y = from_value_y(*y);
    let dot = egui::pos2(
        rect.min.x + norm_x.clamp(0.0, 1.0) * PAD2D_SIZE,
        rect.min.y + norm_y.clamp(0.0, 1.0) * PAD2D_SIZE,
    );
    painter.circle_filled(dot, 5.0, dot_color);

    changed
}

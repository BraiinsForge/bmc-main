// Copyright (C) 2026  Braiins Systems s.r.o.

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
            for (i, knob) in ctx.knobs_mut().iter_mut().enumerate() {
                changed |= render_knob(ui, knob, i == 0);
            }
        });
    changed
}

/// Render a single knob in the grid. Returns `true` if the value changed.
fn render_knob(ui: &mut egui::Ui, knob: &mut Knob, first: bool) -> bool {
    let mut changed = false;
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
        } => {
            ui.label(label.as_str());
            changed = ui.add(egui::Slider::new(value, *min..=*max)).changed();
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
    }
    ui.end_row();
    changed
}

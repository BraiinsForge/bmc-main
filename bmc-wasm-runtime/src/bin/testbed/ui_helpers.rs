// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared widget primitives for the testbed's right sidebar + stats panel.

/// Monospace 11pt gray field-label. Non-selectable, click-sensible.
///
/// ```text
/// timezone        [ Europe/Prague        ]
/// ^^^^^^^^
/// ```
pub(super) fn key_label(text: &str, gray: u8) -> egui::Label {
    egui::Label::new(
        egui::RichText::new(text)
            .font(egui::FontId::monospace(11.0))
            .color(egui::Color32::from_gray(gray)),
    )
    .selectable(false)
    .sense(egui::Sense::click())
}

/// Outer `ComboBox` shell. `populate` runs only while the popup is open
/// and returns whether the user picked a different option.
///
/// ```text
/// [ Hour24      ▼ ]   ← combo_cell
/// ┌───────────────┐
/// │ Hour12        │     ← populate(): for v in variants { selectable_label }
/// │▶ Hour24       │
/// └───────────────┘
/// ```
pub(super) fn combo_cell(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    selected_text: impl Into<egui::WidgetText>,
    populate: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(width)
        .show_ui(ui, populate)
        .inner
        .unwrap_or(false)
}

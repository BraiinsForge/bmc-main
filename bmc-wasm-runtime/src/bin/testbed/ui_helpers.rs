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

/// Enum params with at most this many variants render as an always-visible
/// radio group (`radio_group_cell`); larger sets fall back to a `combo_cell`
/// dropdown. Tweak the threshold here — there's only one call site per
/// enum kind in `params_ui.rs` / `system_ui.rs`.
pub(super) const RADIO_GROUP_MAX_VARIANTS: usize = 5;

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

/// Always-visible vertical radio group for small enum-valued params.
/// `populate` adds one `radio_value` per option and returns whether the
/// selection changed. No popup, no collapsed state — every variant
/// reachable in a single click.
///
/// ```text
/// (•) Analog (round)        ← populate(): for v in variants { radio_value }
/// ( ) Analog (rectangular)
/// (•) Digital
/// ```
pub(super) fn radio_group_cell(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    populate: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    ui.push_id(id_salt, |scope| {
        scope
            .vertical(|col| {
                col.set_min_width(width);
                col.set_max_width(width);
                populate(col)
            })
            .inner
    })
    .inner
}

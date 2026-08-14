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

//! Shared widget primitives for the testbed's chrome — the sidebar's fields
//! and the bars' groupings.

/// How far a group divider stops short of its bar's edges.
const DIVIDER_INSET: f32 = 9.0;

/// Separate two groups of controls or readouts.
///
/// Painted rather than `Separator`-ed: egui's rules the bar's full height,
/// which partitions regions instead of parting neighbours.
pub(super) fn group_divider(ui: &mut egui::Ui, color: egui::Color32, bar_h: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.spacing().item_spacing.x * 2.0, bar_h),
        egui::Sense::hover(),
    );
    ui.painter().vline(
        rect.center().x,
        egui::Rangef::new(rect.top() + DIVIDER_INSET, rect.bottom() - DIVIDER_INSET),
        egui::Stroke::new(1.0_f32, color),
    );
}

/// Monospace 11pt gray field-label. Non-selectable, click-sensible:
/// clicking one acts on the input beside it.
///
/// ```text
/// timezone        [ Europe/Prague        ]
/// ^^^^^^^^
/// ```
pub(super) fn key_label(text: &str) -> egui::Label {
    key_caption(text).sense(egui::Sense::click())
}

/// The same label with nothing beside it to act on — a readout's caption.
/// Unsensed on purpose: a hover highlight promises a click that never comes.
pub(super) fn key_caption(text: &str) -> egui::Label {
    // No explicit colour: the label inherits the theme's text tone.
    // Weak proved too faint on the light panel, plain reads on both.
    egui::Label::new(egui::RichText::new(text).font(egui::FontId::monospace(11.0)))
        .selectable(false)
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

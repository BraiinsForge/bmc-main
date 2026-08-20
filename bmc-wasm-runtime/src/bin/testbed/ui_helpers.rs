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

//! Shared widget primitives for the testbed's chrome — the sidebar's fields,
//! the bars' groupings, and the dialogs.
//!
//! egui themes through `Style`, which `theme::apply` sets once, but the token
//! layer has holes: `TextEdit` sizes from a margin baked into its own builder
//! and never reads `spacing.interact_size`, and a `Modal`'s surface and footer
//! are per-call. Anything the tokens cannot reach becomes a primitive here, so
//! it is decided once rather than at each call site — egui's own answer, the
//! `Widget` trait and reusable `Frame` constructors, is the same shape.

use bmc_wasm_runtime::platform_catalog::{Platform, Target};

use super::theme::{Palette, size, spacing};

/// How the chrome names a device, and one of its viewports.
///
/// The product code as a product code — `BMC100`, not the catalog's lowercase
/// key — and the viewport by its label rather than its id, so a name reads as
/// a name. The id form (`bmc100:full`) stays where it belongs: config files,
/// `--record`, and the directory a fixture's frames land in.
pub(super) fn platform_name(platform: &Platform) -> String {
    platform.id.to_uppercase()
}

pub(super) fn target_name(target: Target) -> String {
    format!(
        "{} · {}",
        platform_name(target.platform),
        target.viewport.label
    )
}

/// Give a control the cursor its state implies.
///
/// egui has no style switch for this — the cursor is set per response — so
/// every button the chrome paints goes through here rather than each call site
/// remembering.
pub(super) fn with_pointer(response: egui::Response) -> egui::Response {
    if response.enabled() {
        if response.hovered() {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    } else if response.contains_pointer() {
        // `hovered` is false for a disabled widget by definition, so the state
        // that most needs a cursor is the one that cannot ask for it that way.
        // `contains_pointer` is the geometric test `on_disabled_hover_text`
        // already uses to reach the same widgets.
        response.ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
    }
    response
}

/// A button carrying a solid accent: the accent at rest, a deeper step under
/// the pointer.
///
/// `Button::fill` would paint every state the same colour, so the fill rides
/// `weak_bg_fill` per state instead. Disabled needs no entry — egui fades a
/// disabled `Ui` whole.
pub(super) fn accent_button(
    ui: &mut egui::Ui,
    label: &str,
    accent: Accent,
    enabled: bool,
    palette: &Palette,
) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;
        widgets.inactive.weak_bg_fill = accent.rest;
        widgets.hovered.weak_bg_fill = accent.hover;
        widgets.active.weak_bg_fill = accent.hover;

        let label = egui::RichText::new(label)
            .color(palette.text_on_color)
            .strong();
        with_pointer(ui.add_enabled(enabled, egui::Button::new(label)))
    })
    .inner
}

/// A fill and the step it takes under the pointer.
#[derive(Clone, Copy)]
pub(super) struct Accent {
    rest: egui::Color32,
    hover: egui::Color32,
}

impl Accent {
    pub(super) fn record(palette: &Palette) -> Self {
        Self {
            rest: palette.accent_record,
            hover: palette.accent_record_hover,
        }
    }
}

/// How far a group divider stops short of its bar's edges.
const DIVIDER_INSET: f32 = 9.0;

/// A field's text inset from its own edge, which is also what gives the field
/// its height: `TextEdit` sizes itself to one text row plus this margin.
const FIELD_PAD_X: i8 = spacing::S03 as i8;
const FIELD_PAD_Y: i8 = spacing::S03 as i8;

/// Hit area of a close cross, and half the length of each arm.
pub(super) const CLOSE_SIZE: f32 = 16.0;
const CLOSE_ARM: f32 = 3.5;

/// The dismiss control every piece of chrome shares — a window's title strip,
/// a banner.
///
/// A cross rather than a labelled button: it goes where a label has no room,
/// and its hint carries what dismissing means there.
pub(super) fn close_button(ui: &mut egui::Ui, centre: egui::Pos2, hint: &str) -> bool {
    let hit = egui::Rect::from_center_size(centre, egui::Vec2::splat(CLOSE_SIZE));
    let response = ui
        .interact(hit, ui.id().with(("close", hint)), egui::Sense::click())
        .on_hover_text(hint);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let colour = if response.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let arm = CLOSE_ARM;
    let stroke = egui::Stroke::new(1.5_f32, colour);
    ui.painter().line_segment(
        [centre - egui::vec2(arm, arm), centre + egui::vec2(arm, arm)],
        stroke,
    );
    ui.painter().line_segment(
        [
            centre + egui::vec2(arm, -arm),
            centre - egui::vec2(arm, -arm),
        ],
        stroke,
    );
    response.clicked()
}

/// The height [`text_field`] takes, from the same padding it is built with.
pub(super) fn field_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + f32::from(FIELD_PAD_Y) * 2.0
}

/// A single-line text input at the chrome's field height.
///
/// The height comes from the margin, never from `add_sized`: forcing a taller
/// rect leaves the galley at its top, so the field grows and its text does not
/// follow. Padding it moves both.
pub(super) fn text_field(
    ui: &mut egui::Ui,
    width: f32,
    value: &mut String,
    hint: &str,
) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width)
            .margin(egui::Margin::symmetric(FIELD_PAD_X, FIELD_PAD_Y)),
    )
}

/// A dialog's heading and the sentence under it, plus the gap separating the
/// pair from the body it introduces.
///
/// The gap belongs to the header rather than to each caller: egui inserts
/// `item_spacing.y` between every label on top of any `add_space`, so a header
/// assembled at the call site measures a few pixels more than it reads and
/// drifts from the next dialog's. Callers zero that spacing (see
/// [`dialog_body`]) and every gap is then exactly the step it names.
pub(super) fn dialog_header(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.label(egui::RichText::new(title).heading().strong());
    ui.add_space(spacing::S02);
    ui.label(egui::RichText::new(body).weak());
    ui.add_space(spacing::S06);
}

/// The dialog's inset content, with egui's implicit row spacing switched off
/// so the gaps are only the ones the body asks for.
pub(super) fn dialog_body<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(DIALOG_PAD)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

/// A dialog's surface: flat, square and unshadowed.
///
/// The backdrop separates it from the canvas, so a border and a drop shadow
/// on top of that only muddy the edge. Carries no inner margin — the body
/// insets itself, which is what lets a footer run edge to edge.
pub(super) fn dialog_surface(palette: &Palette) -> egui::Frame {
    egui::Frame {
        fill: palette.layer,
        ..Default::default()
    }
}

/// The dialog body's inset from its surface.
pub(super) const DIALOG_PAD: i8 = spacing::S05 as i8;

/// A dialog's confirming action, as its footer paints it.
#[derive(Clone, Copy)]
pub(super) struct DialogPrimary {
    pub(super) label: &'static str,
    pub(super) fill: egui::Color32,
    pub(super) hover: egui::Color32,
    pub(super) enabled: bool,
}

/// Which of a dialog footer's two buttons was pressed.
pub(super) enum FooterClick {
    None,
    Cancel,
    Primary,
}

/// The dialog footer: two buttons splitting the full width, flush to the
/// surface's bottom edge. No separator and no margin — the colour break is
/// the divide, and a gap above them leaves the surface looking unfinished.
pub(super) fn dialog_footer(
    ui: &mut egui::Ui,
    primary: DialogPrimary,
    palette: &Palette,
) -> FooterClick {
    let cell = egui::vec2(ui.available_width() / 2.0, size::LG);
    let cancel = FooterButton {
        label: "Cancel",
        fill: palette.field,
        hover: palette.field_hover,
        text: ui.visuals().strong_text_color(),
        enabled: true,
    };
    let confirm = FooterButton {
        label: primary.label,
        fill: primary.fill,
        hover: primary.hover,
        text: palette.text_on_color,
        enabled: primary.enabled,
    };
    let mut click = FooterClick::None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if footer_button(ui, cell, cancel, palette).clicked() {
            click = FooterClick::Cancel;
        }
        if footer_button(ui, cell, confirm, palette).clicked() {
            click = FooterClick::Primary;
        }
    });
    click
}

/// One footer button as its painter needs it, the action's colours resolved.
#[derive(Clone, Copy)]
struct FooterButton {
    label: &'static str,
    fill: egui::Color32,
    hover: egui::Color32,
    text: egui::Color32,
    enabled: bool,
}

/// One footer button, its label at the leading edge.
///
/// Hand-painted because `egui::Button` centres its text in whatever rect it is
/// given, which strands the label mid-button at half a dialog's width.
fn footer_button(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    button: FooterButton,
    palette: &Palette,
) -> egui::Response {
    let sense = if button.enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    // Unavailable means the whole control goes flat, not a dimmed version of
    // the action — a tinted primary still reads as the thing to click.
    let (fill, text) = match (button.enabled, response.hovered()) {
        (false, _) => (palette.action_disabled, palette.text_disabled),
        (true, true) => (button.hover, button.text),
        (true, false) => (button.fill, button.text),
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter().text(
        egui::pos2(rect.left() + spacing::S05, rect.center().y),
        egui::Align2::LEFT_CENTER,
        button.label,
        egui::FontId::proportional(FOOTER_FONT),
        text,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(if button.enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::NotAllowed
        });
    }
    response
}

const FOOTER_FONT: f32 = 14.0;

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

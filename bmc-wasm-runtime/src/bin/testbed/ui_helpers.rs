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

use super::theme::{Palette, Tone, size, spacing};

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

pub(super) const ICON_SIZE: f32 = 14.0;
const INLINE_GAP: f32 = 6.0;
const STACK_GAP: f32 = 2.0;

/// So a one-word control and its three-word neighbour match on the bar.
const MIN_STACK_W: f32 = 54.0;

#[derive(Clone, Copy)]
enum Layout {
    Inline,
    /// Keeps the icon's row whether or not the button has one.
    Stacked,
}

/// Laid out by hand rather than as an `egui::Button`.
/// The icon is a tinted mask that [`super::icon`] snaps to the pixel grid,
/// and a `Button` would place it at some fraction of a point instead.
pub(super) struct Button<'a> {
    label: &'a str,
    icon: Option<&'a mut super::icon::Icon>,
    tone: Option<Tone>,
    layout: Layout,
    enabled: bool,
}

impl<'a> Button<'a> {
    pub(super) fn bar(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            tone: None,
            layout: Layout::Stacked,
            enabled: true,
        }
    }

    pub(super) fn inline(label: &'a str) -> Self {
        Self {
            layout: Layout::Inline,
            ..Self::bar(label)
        }
    }

    pub(super) fn icon(mut self, icon: &'a mut super::icon::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Left unsaid, a button wears [`Tone::secondary`].
    pub(super) fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    pub(super) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Disabled needs no colours of its own: `add_enabled_ui` fades the
    /// painter, so fill, icon and label dim together.
    pub(super) fn show(self, ui: &mut egui::Ui, palette: &Palette) -> egui::Response {
        let Self {
            label,
            icon,
            tone,
            layout,
            enabled,
        } = self;
        ui.add_enabled_ui(enabled, |ui| {
            let tone = tone.unwrap_or_else(|| Tone::secondary(palette));
            let (rect, response) =
                allocate_slot(ui, label, layout, icon.is_some(), egui::Sense::click());
            let corner_radius = ui.style().interact(&response).corner_radius;
            ui.painter()
                .rect_filled(rect, corner_radius, tone_fill(tone, &response));
            paint_slot(ui, rect, icon, label, layout, tone.ink);
            with_pointer(response)
        })
        .inner
    }
}

/// Shaped like a stacked button but inert, so a reading
/// on the bar sits level with the controls beside it.
pub(super) fn bar_readout(
    ui: &mut egui::Ui,
    icon: Option<&mut super::icon::Icon>,
    label: &str,
    palette: &Palette,
) -> egui::Response {
    let (rect, response) = allocate_slot(
        ui,
        label,
        Layout::Stacked,
        icon.is_some(),
        egui::Sense::hover(),
    );
    paint_slot(
        ui,
        rect,
        icon,
        label,
        Layout::Stacked,
        palette.text_secondary,
    );
    response
}

/// Held first: the pointer is over a button it is pressing,
/// so asking about hover would answer for both.
fn tone_fill(tone: Tone, response: &egui::Response) -> egui::Color32 {
    if response.is_pointer_button_down_on() {
        tone.pressed
    } else if response.hovered() {
        tone.hover
    } else {
        tone.rest
    }
}

fn inline_icon_advance(has_icon: bool) -> f32 {
    if has_icon {
        ICON_SIZE + INLINE_GAP
    } else {
        0.0
    }
}

fn allocate_slot(
    ui: &mut egui::Ui,
    label: &str,
    layout: Layout,
    has_icon: bool,
    sense: egui::Sense,
) -> (egui::Rect, egui::Response) {
    let padding = ui.spacing().button_padding;
    let text = button_label(ui, label);
    let (content, floor) = match layout {
        Layout::Inline => (
            egui::vec2(
                inline_icon_advance(has_icon) + text.size().x,
                // So a label-only button stands as tall as the one beside it.
                text.size().y.max(ICON_SIZE),
            ),
            0.0,
        ),
        Layout::Stacked => (
            egui::vec2(
                text.size().x.max(ICON_SIZE),
                ICON_SIZE + STACK_GAP + text.size().y,
            ),
            MIN_STACK_W,
        ),
    };
    let size = egui::vec2(
        (content.x + 2.0 * padding.x).max(floor),
        content.y + 2.0 * padding.y,
    );
    ui.allocate_exact_size(size, sense)
}

fn paint_slot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: Option<&mut super::icon::Icon>,
    label: &str,
    layout: Layout,
    colour: egui::Color32,
) {
    let inner = rect.shrink2(ui.spacing().button_padding);
    let text = button_label(ui, label);
    let (icon_min, text_min) = match layout {
        Layout::Inline => {
            let advance = inline_icon_advance(icon.is_some());
            let left = inner.center().x - f32::midpoint(advance, text.size().x);
            (
                egui::pos2(left, inner.center().y - ICON_SIZE / 2.0),
                egui::pos2(left + advance, inner.center().y - text.size().y / 2.0),
            )
        }
        Layout::Stacked => (
            egui::pos2(inner.center().x - ICON_SIZE / 2.0, inner.min.y),
            egui::pos2(
                inner.center().x - text.size().x / 2.0,
                inner.max.y - text.size().y,
            ),
        ),
    };
    if let Some(icon) = icon {
        icon.paint(
            ui,
            egui::Rect::from_min_size(icon_min, egui::Vec2::splat(ICON_SIZE)),
            colour,
        );
    }
    ui.painter().galley(text_min, text, colour);
}

fn button_label(ui: &egui::Ui, label: &str) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        egui::Color32::PLACEHOLDER,
    )
}

/// How far a group divider stops short of its bar's edges.
const DIVIDER_INSET: f32 = 9.0;

/// A field's text inset from its own edge, which is also what gives the field
/// its height: `TextEdit` sizes itself to one text row plus this margin.
const FIELD_PAD_X: i8 = spacing::S03 as i8;
const FIELD_PAD_Y: i8 = spacing::S03 as i8;

/// Nominal box of a close cross, which is what places it.
pub(super) const CLOSE_SIZE: f32 = 16.0;

/// Under [`ICON_SIZE`]: the cross is drawn edge to edge where a toolbar icon
/// carries its own margin, so equal numbers do not render as equal weight.
const CLOSE_GLYPH: f32 = 12.0;

/// Larger than any surface offers, so the surface is what decides.
const CLOSE_HIT: f32 = 32.0;

/// The dismiss control every piece of chrome shares — a window's title strip,
/// a banner.
///
/// The target fills `within` and clips to it. This only interacts and never
/// allocates, so one reaching past its own surface would take the clicks
/// meant for whatever lies under it.
pub(super) fn close_button(
    ui: &mut egui::Ui,
    icon: &mut super::icon::Icon,
    centre: egui::Pos2,
    within: egui::Rect,
    hint: &str,
) -> bool {
    let hit = egui::Rect::from_center_size(centre, egui::Vec2::splat(CLOSE_HIT)).intersect(within);
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
    icon.paint(
        ui,
        egui::Rect::from_center_size(centre, egui::Vec2::splat(CLOSE_GLYPH)),
        colour,
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
    pub(super) tone: Tone,
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
        tone: Tone::secondary(palette),
        enabled: true,
    };
    let confirm = FooterButton {
        label: primary.label,
        tone: primary.tone,
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
    tone: Tone,
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
    let (fill, text) = if button.enabled {
        (tone_fill(button.tone, &response), button.tone.ink)
    } else {
        (palette.action_disabled, palette.text_disabled)
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

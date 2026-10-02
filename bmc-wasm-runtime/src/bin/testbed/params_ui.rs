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

//! Right-side params sidebar: SidePanel housing, per-row type-appropriate
//! inputs (text / number with optional GIMP-style filled slider / dropdown /
//! checkbox / clear-to-null toggle), and the `apply_params_update` delivery
//! path that drives `WasmWidgetRuntime::deliver_params_update` on every tile
//! plus appends a `ParamDelivery` event when recording is active.
//!
//! Only params the shared validator passes are delivered, as on the device;
//! the sidebar lists the violations that hold the rest back.

use std::collections::BTreeMap;

use bmc_wasm_runtime::unified_fixture::UnifiedEvent;
use bmc_widget_manifest::{
    ArrayParam, ItemKind, ItemShape, Manifest, MissingValues, ObjectParam, ParamDefinition,
    ParamKey, ParamValue, Scalar, Shape, Violation, validate_values,
};

use super::icon::Icons;
use super::system_ui::{field_margin, row_height};
use super::theme::Palette;
use super::ui_helpers::{Button, combo_cell, key_label, radio_group_cell};
use super::view::{Delivery, ViewCommand};
use super::{PARAM_PANEL_W, TestbedApp};

impl TestbedApp {
    /// Push a new params snapshot to every tile's runtime via `deliver_params_update`,
    /// update the local cache, and (when recording is active) append a `ParamDelivery`
    /// event to the timeline plus a debounced auto-`Capture`.
    /// See [`super::recording::record_delivery`] for the debounce semantics.
    ///
    /// A no-op when the new snapshot matches the cached one.
    fn apply_params_update(
        &mut self,
        new_params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
    ) {
        if new_params == self.state().params {
            return;
        }
        // Fire `on_params_update` on every view: an operator-driven change
        // applies to every previewed viewport, not just the one being recorded.
        for view in self.stage.tiles_mut() {
            view.send(ViewCommand::Deliver(Delivery::Params(new_params.clone())));
        }
        self.recording_mode
            .record_delivery(|| UnifiedEvent::ParamDelivery {
                params: new_params
                    .iter()
                    .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
                    .collect(),
            });
        self.state_mut().params = new_params;
    }

    /// Render the unified right-side sidebar:
    ///  - per-widget Params (when the manifest declares any) on top,
    ///  - deck-wide System always below.
    ///
    /// Both sections share a single vertical [`egui::ScrollArea`]
    /// so long param/system lists scroll together rather than stealing
    /// each other's height.
    pub(super) fn paint_right_panel(&mut self, root_ui: &mut egui::Ui) {
        let palette = self.theme.palette(root_ui.ctx());
        let section_fill = palette.layer_inset;
        let has_params = !self.manifest.params.is_empty();
        // Take the current snapshots out so we can mutate while
        // the egui closure borrows `self`, then put them back
        // via `apply_params_update` / `apply_system_update`
        // which detect diffs and propagate to every tile.
        let mut working_params = self.state().params_draft.clone();
        let mut validated = None;
        let manifest = &self.manifest;
        let mut working_system = self.state().system.clone();
        let mut working_credentials = self.state().credentials.clone();
        let credential_slots = self.credential_slots();
        let icons = &mut self.icons;
        let mut credentials_changed = false;
        let mut params_changed = false;
        let mut system_changed = false;

        // egui stacks windows above panels, so the fill and the widgets both
        // go in a foreground area; the panel itself only reserves space.
        let panel = egui::SidePanel::right("right_panel")
            .resizable(false)
            .exact_width(PARAM_PANEL_W)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show_inside(root_ui, |_| {});
        let rect = panel.response.rect;
        egui::Area::new(egui::Id::new("sidebar_chrome"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(root_ui.ctx(), |area| {
                area.set_clip_rect(rect);
                area.painter().rect_filled(rect, 0.0, palette.layer);
                // `new_child` allocates nothing here, which leaves the area's layer empty,
                // and an empty layer is never under the pointer to take the wheel.
                area.expand_to_include_rect(rect);
                let mut ui = area.new_child(egui::UiBuilder::new().max_rect(rect));
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(&mut ui, |scroll| {
                        if has_params {
                            section_header_bar(scroll, "Params", section_fill);
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::same(SECTION_PAD))
                                .show(scroll, |inner| {
                                    params_changed = paint_params_section(
                                        inner,
                                        manifest,
                                        &mut working_params,
                                        icons,
                                        palette,
                                    );
                                    let outcome = validate_draft(manifest, &working_params);
                                    if let Err(violations) = &outcome {
                                        paint_violations(inner, violations, palette);
                                    }
                                    validated = Some(outcome);
                                });
                            scroll.add_space(12.0);
                        }
                        section_header_bar(scroll, "System", section_fill);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::same(SECTION_PAD))
                            .show(scroll, |inner| {
                                system_changed =
                                    Self::paint_system_section(inner, &mut working_system);
                            });
                        scroll.add_space(12.0);
                        credentials_changed = Self::paint_credentials_section(
                            scroll,
                            &credential_slots,
                            &mut working_credentials,
                            palette,
                        );
                    });
            });

        if params_changed {
            self.state_mut().params_draft = working_params;
            if let Some(Ok(params)) = validated {
                self.apply_params_update(params);
            }
        }
        if system_changed {
            self.apply_system_update(working_system);
        }
        if credentials_changed {
            self.apply_credentials_update(working_credentials);
        }
    }
}

/// Inset of a section's controls from the sidebar's edges.
const SECTION_PAD: i8 = 8;

/// Render a section header as a full-width horizontal accent banner with
/// black text — no left stripe.
///
/// Painted directly via [`egui::Painter`] into a single allocated rect
/// rather than via a nested `Frame` + `Layout`: the latter let the inner
/// ui's min-size expand into the surrounding ScrollArea's full vertical,
/// turning the banner into a panel-tall solid block.
pub(super) fn section_header_bar(ui: &mut egui::Ui, text: &str, fill: egui::Color32) {
    let width = ui.available_width();
    let bar_height: f32 = 26.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, bar_height), egui::Sense::hover());
    let text_colour = ui.visuals().strong_text_color();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, fill);
    painter.text(
        rect.min + egui::vec2(10.0, bar_height / 2.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(14.0),
        text_colour,
    );
}

/// What the device would deliver for the sidebar's params,
/// or every violation it would refuse them with.
fn validate_draft(
    manifest: &Manifest,
    draft: &BTreeMap<ParamKey, ParamValue>,
) -> Result<BTreeMap<ParamKey, ParamValue>, Vec<Violation>> {
    let values = draft
        .iter()
        .map(|(key, value)| (key.as_str().to_owned(), Ok(value.clone())))
        .collect();
    validate_values(&manifest.params, &values, MissingValues::Reject)
}

fn paint_violations(ui: &mut egui::Ui, violations: &[Violation], palette: &Palette) {
    ui.add_space(super::system_ui::ROW_GAP);
    ui.label(
        egui::RichText::new("Not delivered; the widget keeps its last valid params:")
            .color(palette.support_error),
    );
    for violation in violations {
        ui.label(
            egui::RichText::new(format!("{}: {}", violation.path, violation.message))
                .color(palette.support_error),
        );
    }
}

// ── Param-mutation inputs ───────────────────────────────────────────

fn paint_params_section(
    ui: &mut egui::Ui,
    manifest: &Manifest,
    values: &mut BTreeMap<ParamKey, ParamValue>,
    icons: &mut Icons,
    palette: &Palette,
) -> bool {
    let mut changed = false;
    egui::Grid::new("params_grid")
        .num_columns(2)
        .spacing([12.0, super::system_ui::ROW_GAP])
        .min_col_width(0.0)
        .show(ui, |grid| {
            for (key, def) in &manifest.params {
                if let Shape::Scalar(scalar) = def.kind.shape() {
                    let value = entry_or_default(values, key, def);
                    changed |= paint_param_row(grid, key.as_str(), def, scalar, value);
                }
            }
        });
    // A list needs the sidebar's full width, not the grid's value column.
    for (key, def) in &manifest.params {
        if let Shape::Array(array) = def.kind.shape() {
            let value = entry_or_default(values, key, def);
            changed |= paint_list_block(ui, key.as_str(), array, value, icons, palette);
        }
    }
    changed
}

fn entry_or_default<'a>(
    values: &'a mut BTreeMap<ParamKey, ParamValue>,
    key: &ParamKey,
    def: &ParamDefinition,
) -> &'a mut ParamValue {
    values
        .entry(key.clone())
        .or_insert_with(|| ParamValue::from_param_kind_default(&def.kind))
}

/// Render one row inside the params Grid: monospace key in the left column,
/// type-appropriate input + optional clear-to-null toggle in the right column.
///
/// Returns `true` when the operator changed the value this frame.
fn paint_param_row(
    grid: &mut egui::Ui,
    key: &str,
    def: &ParamDefinition,
    scalar: Scalar<'_>,
    value: &mut ParamValue,
) -> bool {
    let mut changed = false;
    // Centred on the control's first row, so a tall radio group
    // anchors its label at the first option rather than halfway down.
    let label_resp = grid
        .with_layout(egui::Layout::left_to_right(egui::Align::TOP), |cell| {
            let first_row = egui::vec2(0.0, row_height(cell));
            cell.allocate_ui_with_layout(
                first_row,
                egui::Layout::left_to_right(egui::Align::Center),
                |slot| slot.add(key_label(key)),
            )
            .inner
        })
        .inner;

    // Top-align inside the row so a tall multi-line input (radio group)
    // can only extend downward; the default `horizontal()` centers
    // children vertically, which overflows tall inputs both up and down
    // into adjacent Grid rows.
    grid.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |row| {
        if def.is_optional {
            let is_null = matches!(value, ParamValue::Null);
            // Plain-text labels — `✗` and similar dingbats aren't in egui's bundled font
            // and render as a missing-glyph box.
            let label = if is_null { "(unset)" } else { "clear" };
            if null_toggle(row, label) {
                if is_null {
                    *value = seed_value(scalar);
                } else {
                    *value = ParamValue::Null;
                }
                changed = true;
            }
            if matches!(value, ParamValue::Null) {
                // Nothing to render after the (unset) button when the value is cleared.
                return;
            }
        }
        changed |= paint_typed_input(row, key, scalar, value, Some(&label_resp), false);
    });
    grid.end_row();
    changed
}

/// "(unset)" or "clear", as tall as the input it stands in for.
fn null_toggle(ui: &mut egui::Ui, label: &str) -> bool {
    let size = egui::vec2(0.0, row_height(ui));
    ui.add(egui::Button::new(label).min_size(size)).clicked()
}

/// A value the validator passes, to edit in an input without a default:
/// an enum's first option, a number's zero clamped into its bounds,
/// [`SEED_TIMEZONE`] for a timezone, or else an empty string or `false`.
fn seed_without_default(scalar: Scalar<'_>) -> ParamValue {
    match scalar {
        Scalar::String(p) => ParamValue::String(
            p.enum_values
                .first()
                .map_or_else(String::new, |option| option.value.clone()),
        ),
        Scalar::Timezone(_) => ParamValue::String(SEED_TIMEZONE.to_owned()),
        Scalar::Integer(p) => ParamValue::Integer(p.enum_values.first().map_or_else(
            || 0.clamp(p.min.unwrap_or(i32::MIN), p.max.unwrap_or(i32::MAX)),
            |option| option.value,
        )),
        Scalar::Double(p) => ParamValue::Double(p.enum_values.first().map_or_else(
            || 0.0_f64.clamp(p.min.unwrap_or(f64::MIN), p.max.unwrap_or(f64::MAX)),
            |option| option.value,
        )),
        Scalar::Boolean(_) => ParamValue::Boolean(false),
    }
}

/// UTC under its name in the device's curated list, which has no plain `UTC`.
const SEED_TIMEZONE: &str = "Etc/GMT";

#[derive(Clone, Copy)]
enum RowAction {
    Up(usize),
    Down(usize),
    Remove(usize),
}

/// A list param across the sidebar's full width: its key, a row per item, then `add`.
fn paint_list_block(
    ui: &mut egui::Ui,
    key: &str,
    array: &ArrayParam,
    value: &mut ParamValue,
    icons: &mut Icons,
    palette: &Palette,
) -> bool {
    ui.add_space(super::system_ui::ROW_GAP);
    ui.add(key_label(key));
    let ParamValue::List(items) = value else {
        return paint_type_mismatch(ui);
    };
    let can_remove = items.len() > array.min_items;
    let can_add = items.len() < array.max_items;
    let last = items.len().saturating_sub(1);
    let mut changed = false;
    let mut action = None;
    for (i, item) in items.iter_mut().enumerate() {
        let item_key = format!("{key}[{i}]");
        match array.items.shape() {
            ItemShape::Scalar(scalar) => {
                buttons_row(ui, |row| {
                    action = action.or(row_buttons(row, i, last, can_remove, icons, palette));
                    row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |cell| {
                        changed |= paint_typed_input(cell, &item_key, scalar, item, None, true);
                    });
                });
            }
            ItemShape::Object(object) => {
                egui::Frame::group(ui.style()).show(ui, |group| {
                    buttons_row(group, |head| {
                        action = action.or(row_buttons(head, i, last, can_remove, icons, palette));
                        head.with_layout(
                            egui::Layout::left_to_right(egui::Align::Center),
                            |cell| {
                                cell.label(format!("#{}", i + 1));
                            },
                        );
                    });
                    changed |= paint_object_fields(group, &item_key, object, item);
                });
            }
        }
    }
    if Button::inline("add")
        .icon(&mut icons.add)
        .enabled(can_add)
        .min_height(row_height(ui))
        .show(ui, palette)
        .clicked()
    {
        items.push(seed_item(&array.items));
        changed = true;
    }
    match action {
        Some(RowAction::Up(i)) => items.swap(i - 1, i),
        Some(RowAction::Down(i)) => items.swap(i, i + 1),
        Some(RowAction::Remove(i)) => {
            items.remove(i);
        }
        None => {}
    }
    changed || action.is_some()
}

/// One row tall and filled from the right, so every row's buttons line up.
///
/// Not `with_layout`: that takes all the remaining height and centres the row in it.
fn buttons_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let size = egui::vec2(ui.available_width(), row_height(ui));
    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), add);
}

/// Right to left, as the row lays them out: remove, move down, move up.
///
/// Packed into one set, as the toolbar packs its own,
/// so the row's usual gap sets them apart from the input.
fn row_buttons(
    ui: &mut egui::Ui,
    i: usize,
    last: usize,
    can_remove: bool,
    icons: &mut Icons,
    palette: &Palette,
) -> Option<RowAction> {
    ui.scope(|set| {
        set.spacing_mut().item_spacing.x = 1.0;
        let mut action = None;
        if icon_button(set, &mut icons.remove, can_remove, "Remove", palette) {
            action = Some(RowAction::Remove(i));
        }
        if icon_button(set, &mut icons.move_down, i < last, "Move down", palette) {
            action = Some(RowAction::Down(i));
        }
        if icon_button(set, &mut icons.move_up, i > 0, "Move up", palette) {
            action = Some(RowAction::Up(i));
        }
        action
    })
    .inner
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: &mut super::icon::Icon,
    enabled: bool,
    hint: &str,
    palette: &Palette,
) -> bool {
    Button::icon_only(icon)
        .enabled(enabled)
        .min_height(row_height(ui))
        .show(ui, palette)
        .on_hover_text(hint)
        .clicked()
}

fn paint_object_fields(
    ui: &mut egui::Ui,
    key: &str,
    object: &ObjectParam,
    value: &mut ParamValue,
) -> bool {
    let ParamValue::Object(fields) = value else {
        return paint_type_mismatch(ui);
    };
    let mut changed = false;
    egui::Grid::new(key)
        .num_columns(2)
        .spacing([12.0, super::system_ui::ROW_GAP])
        .show(ui, |grid| {
            for (field_key, field) in &object.fields {
                let scalar = field.kind.as_scalar();
                grid.label(&field.name);
                grid.horizontal(|cell| {
                    let is_set = fields
                        .get(field_key)
                        .is_some_and(|v| !matches!(v, ParamValue::Null));
                    if !is_set {
                        if null_toggle(cell, "(unset)") {
                            fields.insert(field_key.clone(), seed_value(scalar));
                            changed = true;
                        }
                        return;
                    }
                    if field.is_optional && null_toggle(cell, "clear") {
                        fields.insert(field_key.clone(), ParamValue::Null);
                        changed = true;
                        return;
                    }
                    if let Some(entry) = fields.get_mut(field_key) {
                        let field_path = format!("{key}[{}]", field_key.as_str());
                        changed |= paint_typed_input(cell, &field_path, scalar, entry, None, true);
                    }
                });
                grid.end_row();
            }
        });
    changed
}

fn seed_value(scalar: Scalar<'_>) -> ParamValue {
    let seed = ParamValue::from_scalar_default(scalar);
    if matches!(seed, ParamValue::Null) {
        seed_without_default(scalar)
    } else {
        seed
    }
}

/// Required object fields start set, so a fresh row validates.
fn seed_item(kind: &ItemKind) -> ParamValue {
    match kind.shape() {
        ItemShape::Scalar(scalar) => seed_value(scalar),
        ItemShape::Object(object) => ParamValue::Object(
            object
                .fields
                .iter()
                .map(|(key, field)| {
                    let scalar = field.kind.as_scalar();
                    let value = if field.is_optional {
                        ParamValue::from_scalar_default(scalar)
                    } else {
                        seed_value(scalar)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
    }
}

/// A well-formed manifest rules a mismatch out; a label beats crashing the testbed.
fn paint_type_mismatch(ui: &mut egui::Ui) -> bool {
    ui.label(
        egui::RichText::new("(type mismatch)")
            .color(egui::Color32::from_rgb(200, 80, 80))
            .font(egui::FontId::monospace(10.0)),
    );
    false
}

/// A slider track filling `cell_w`, beside its own number box.
///
/// `Slider`'s built-in box sizes to the text it shows, so it outgrows any reserve for it.
/// This box is as wide as the widest value in `range`: it never overflows the cell,
/// and the track keeps its length while dragged across a change in digit count.
fn slider_cell<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    cell_w: f32,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    step: f64,
    decimals: usize,
    unit: Option<&str>,
) -> egui::Response {
    let row_h = row_height(ui);
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    let number_w = [lo, hi]
        .into_iter()
        .map(|end| number_width(ui, end, decimals, unit))
        .fold(ui.spacing().interact_size.x, f32::max);
    // A continuous range has no step to drag the box by.
    let speed = if step > 0.0 { step } else { (hi - lo) / 100.0 };
    ui.allocate_ui_with_layout(
        egui::vec2(cell_w, row_h),
        egui::Layout::right_to_left(egui::Align::Center),
        |slot| {
            let number = with_unit(
                egui::DragValue::new(value)
                    .range(range.clone())
                    .speed(speed)
                    .fixed_decimals(decimals),
                unit,
            );
            let number = slot.add_sized([number_w, row_h], number);
            slot.spacing_mut().slider_width = slot.available_width();
            let track = slot.add(
                egui::Slider::new(value, range)
                    .step_by(step)
                    .trailing_fill(true)
                    .show_value(false),
            );
            number | track
        },
    )
    .inner
}

/// A `DragValue`'s width showing `value` and its unit, measured the way it lays itself out.
fn number_width(ui: &egui::Ui, value: f64, decimals: usize, unit: Option<&str>) -> f32 {
    let style = ui.style();
    let font = style.drag_value_text_style.resolve(style);
    let text_width = |text: String| {
        ui.painter()
            .layout_no_wrap(text, font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let number = text_width(style.number_formatter.format(value, decimals..=decimals));
    let unit = unit.map_or(0.0, |unit| text_width(unit_suffix(unit)));
    number + unit + 2.0 * ui.spacing().button_padding.x
}

fn with_unit<'a>(number: egui::DragValue<'a>, unit: Option<&str>) -> egui::DragValue<'a> {
    match unit {
        Some(unit) => number.suffix(unit_suffix(unit)),
        None => number,
    }
}

/// `DragValue` lays its suffix flush against the number, so the space is the suffix's own.
fn unit_suffix(unit: &str) -> String {
    format!(" {unit}")
}

/// What a double shows when it has no step to take its decimals from.
const CONTINUOUS_DECIMALS: usize = 2;

/// As many decimals as the step has, so every value it lands on reads in full.
fn step_decimals(step: f64) -> usize {
    step.to_string()
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
}

/// Inner dispatch: actual editable widget per kind. Caller has already drawn the label
/// and (when applicable) the optional toggle. Each branch fills the cell at [`row_height`],
/// so inputs line up with the sidebar's selects and with each other.
///
/// Control width comes from `ui.available_width()` — the parent Grid + horizontal layout
/// has already reserved space for the key label and any optional toggle, so what's left is
/// exactly what we want the input to fill. No constant, layout naturally follows sidebar
/// resizes or label changes.
///
/// List items pass no `label_resp`: one label click must not toggle every item.
/// They also pass `compact`, so an enum stays one row tall as a dropdown.
///
/// `too_many_lines` is `expect`ed because the match is one arm per `Scalar` variant +
/// enum-or-not split; pulling each branch into its own function would obscure the otherwise
/// trivial widget construction at every site.
#[expect(
    clippy::too_many_lines,
    reason = "linear Scalar dispatch — splitting hurts readability"
)]
fn paint_typed_input(
    ui: &mut egui::Ui,
    key: &str,
    scalar: Scalar<'_>,
    value: &mut ParamValue,
    label_resp: Option<&egui::Response>,
    compact: bool,
) -> bool {
    use bmc_widget_manifest::{DoubleParam, EnumControl, IntegerParam, StringParam};

    let radio = |control: EnumControl| !compact && control == EnumControl::Radio;
    let row_h = row_height(ui);
    let cell_w = ui.available_width();
    let cell = egui::vec2(cell_w, row_h);
    let label_clicked = label_resp.is_some_and(egui::Response::clicked);
    let focus_on_label_click = |r: &egui::Response| {
        if label_clicked {
            r.request_focus();
        }
    };
    match (scalar, value) {
        (
            Scalar::String(StringParam {
                enum_values,
                enum_control,
                ..
            }),
            ParamValue::String(s),
        ) if !enum_values.is_empty() => {
            // Snapshot the collapsed-state label before `populate`
            // captures `s` mutably; the radio branch ignores it.
            let combo_label = enum_values
                .iter()
                .find(|o| o.value == *s)
                .map_or_else(|| s.clone(), |o| o.label.clone());
            let populate = |inner: &mut egui::Ui| {
                let mut changed = false;
                for opt in enum_values {
                    if inner
                        .radio_value(s, opt.value.clone(), &opt.label)
                        .changed()
                    {
                        changed = true;
                    }
                }
                changed
            };
            if radio(*enum_control) {
                radio_group_cell(ui, key, cell_w, populate)
            } else {
                combo_cell(ui, key, cell_w, combo_label, populate)
            }
        }
        (Scalar::String(_) | Scalar::Timezone(_), ParamValue::String(s)) => {
            let resp = ui.add(
                egui::TextEdit::singleline(s)
                    .hint_text(scalar.placeholder().unwrap_or_default())
                    .desired_width(cell_w)
                    .margin(field_margin(ui)),
            );
            focus_on_label_click(&resp);
            resp.changed()
        }
        (
            Scalar::Integer(IntegerParam {
                enum_values,
                enum_control,
                ..
            }),
            ParamValue::Integer(n),
        ) if !enum_values.is_empty() => {
            // Combo collapsed-state label snapshot before `populate`
            // captures `n` mutably (the radio branch doesn't read it).
            let combo_label = enum_values
                .iter()
                .find(|o| o.value == *n)
                .map_or_else(|| n.to_string(), |o| o.label.clone());
            let populate = |inner: &mut egui::Ui| {
                let mut changed = false;
                for opt in enum_values {
                    if inner.radio_value(n, opt.value, &opt.label).changed() {
                        changed = true;
                    }
                }
                changed
            };
            if radio(*enum_control) {
                radio_group_cell(ui, key, cell_w, populate)
            } else {
                combo_cell(ui, key, cell_w, combo_label, populate)
            }
        }
        (
            Scalar::Integer(IntegerParam {
                min,
                max,
                step,
                unit,
                ..
            }),
            ParamValue::Integer(n),
        ) => {
            // Bounded ranges use a `Slider` with `trailing_fill` so the cell shows
            // the value as a progress fill against `min..=max` (the GIMP-style look).
            // Unbounded integers fall back to a `DragValue` since `Slider` requires a finite range.
            let unit = unit.as_deref();
            if let (Some(lo), Some(hi)) = (min, max) {
                let step = step.map_or(1.0, f64::from);
                let resp = slider_cell(ui, cell_w, n, *lo..=*hi, step, 0, unit);
                focus_on_label_click(&resp);
                resp.changed()
            } else {
                let mut dv = with_unit(
                    egui::DragValue::new(n).speed(step.map_or(1.0, f64::from)),
                    unit,
                );
                if let Some(lo) = min {
                    dv = dv.range(*lo..=i32::MAX);
                } else if let Some(hi) = max {
                    dv = dv.range(i32::MIN..=*hi);
                }
                let resp = ui.add_sized(cell, dv);
                focus_on_label_click(&resp);
                resp.changed()
            }
        }
        (
            Scalar::Double(DoubleParam {
                enum_values,
                enum_control,
                ..
            }),
            ParamValue::Double(f),
        ) if !enum_values.is_empty() => {
            // Combo collapsed-state label snapshot before `populate`
            // captures `f` mutably (the radio branch doesn't read it).
            let combo_label = enum_values
                .iter()
                .find(|o| (o.value - *f).abs() < f64::EPSILON)
                .map_or_else(|| format!("{f}"), |o| o.label.clone());
            let populate = |inner: &mut egui::Ui| {
                let mut changed = false;
                for opt in enum_values {
                    // `radio_value` requires `PartialEq`; f64 is `PartialEq`
                    // but its equality is bit-exact.
                    //
                    // The manifest values round-trip cleanly through serde
                    // so this is fine for typical enums (Linear / Mac / sRGB etc.)
                    // — if a future manifest needs near-equality, switch
                    // to a `selectable_value` with epsilon comparison.
                    if inner.radio_value(f, opt.value, &opt.label).changed() {
                        changed = true;
                    }
                }
                changed
            };
            if radio(*enum_control) {
                radio_group_cell(ui, key, cell_w, populate)
            } else {
                combo_cell(ui, key, cell_w, combo_label, populate)
            }
        }
        (
            Scalar::Double(DoubleParam {
                min,
                max,
                step,
                unit,
                ..
            }),
            ParamValue::Double(f),
        ) => {
            // Same dispatch as Integer: bounded ranges get
            // the filled-slider treatment, unbounded fall back to DragValue.
            let unit = unit.as_deref();
            if let (Some(lo), Some(hi)) = (min, max) {
                let decimals = step.map_or(CONTINUOUS_DECIMALS, step_decimals);
                let step = step.unwrap_or(0.0);
                let resp = slider_cell(ui, cell_w, f, *lo..=*hi, step, decimals, unit);
                focus_on_label_click(&resp);
                resp.changed()
            } else {
                let mut dv = with_unit(egui::DragValue::new(f).speed(step.unwrap_or(0.1)), unit);
                if let Some(lo) = min {
                    dv = dv.range(*lo..=f64::INFINITY);
                } else if let Some(hi) = max {
                    dv = dv.range(f64::NEG_INFINITY..=*hi);
                }
                let resp = ui.add_sized(cell, dv);
                focus_on_label_click(&resp);
                resp.changed()
            }
        }
        // Checkbox stays at its natural icon size — stretching it
        // would make the entire row a giant click target with
        // the box ghosted in the corner.
        (Scalar::Boolean(_), ParamValue::Boolean(b)) => {
            let cb_changed = ui.checkbox(b, "").changed();
            if label_clicked {
                *b = !*b;
            }
            cb_changed || label_clicked
        }
        _ => paint_type_mismatch(ui),
    }
}

#[cfg(test)]
mod layout_tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::super::icon::Icons;
    use super::super::theme;
    use super::{
        Manifest, PARAM_PANEL_W, SECTION_PAD, TestbedApp, number_width, paint_params_section,
        with_unit,
    };

    const SECTION_W: f32 = PARAM_PANEL_W - 2.0 * SECTION_PAD as f32;

    /// Every kind the sidebar paints, at values that make its controls widest.
    fn manifest() -> Manifest {
        let tones = json!([
            {"value": "info", "label": "Info"},
            {"value": "warning", "label": "Warning"},
        ]);
        json!({
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "0.1.0",
            "name": "X",
            "description": "Layout fixture",
            "binary": "bin/x",
            "supported_viewports": [{
                "type": "rectangular",
                "min_width": 317,
                "max_width": 317,
                "min_height": 238,
                "max_height": 238,
                "min_dpi": 1,
                "max_dpi": 1,
            }],
            "params": {
                "free_string": {"type": "string", "name": "S", "default_value": "Hello"},
                "string_enum": {"type": "string", "name": "E", "default_value": "info", "enum_values": tones},
                "radio_enum": {
                    "type": "string", "name": "R", "default_value": "info", "enum_values": tones,
                    "enum_control": "radio",
                },
                "integer_range": {
                    "type": "integer", "name": "I", "min": -100_000, "max": 100_000, "default_value": -100_000,
                },
                "double_range": {
                    "type": "double", "name": "D", "min": 0.0, "max": 1.0, "step": 0.001, "default_value": 0.125,
                },
                "boolean_flag": {"type": "boolean", "name": "B", "default_value": true},
                "tz": {"type": "timezone", "name": "T", "default_value": "Europe/Prague"},
                "optional_set": {
                    "type": "integer", "name": "O", "optional": true, "min": 0, "max": 100, "default_value": 50,
                },
                "optional_unset": {"type": "string", "name": "U", "optional": true},
                "string_list": {
                    "type": "array", "name": "SL", "items": {"type": "string"}, "max_items": 5,
                    "default_value": ["BTC"],
                },
                "integer_list": {
                    "type": "array", "name": "IL",
                    "items": {"type": "integer", "min": -100_000, "max": 100_000},
                    "max_items": 5, "default_value": [-100_000],
                },
                "double_list": {
                    "type": "array", "name": "DL",
                    "items": {"type": "double", "min": 0.0, "max": 1.0, "step": 0.001},
                    "max_items": 5, "default_value": [0.125],
                },
                "boolean_list": {
                    "type": "array", "name": "BL", "items": {"type": "boolean"}, "max_items": 5,
                    "default_value": [true],
                },
                "enum_list": {
                    "type": "array", "name": "EL", "items": {"type": "string", "enum_values": tones},
                    "max_items": 5, "default_value": ["warning"],
                },
                "links": {
                    "type": "array", "name": "L", "max_items": 5,
                    "items": {"type": "object", "fields": {
                        "label": {"type": "string", "name": "Label"},
                        "url": {"type": "string", "name": "URL", "optional": true},
                        "missing": {"type": "string", "name": "Missing", "optional": true},
                        "tone": {"type": "string", "name": "Tone", "enum_values": tones},
                    }},
                    "default_value": [{"label": "Braiins", "url": "https://braiins.com", "tone": "info"}],
                },
            },
        })
        .to_string()
        .parse()
        .expect("BUG: the layout fixture must be a valid manifest")
    }

    /// How far `paint` lays out past either side of a `width`-wide column.
    /// Several frames, as a `Grid` sizes its columns from the frame before.
    fn overflow(width: f32, mut paint: impl FnMut(&mut egui::Ui)) -> f32 {
        let ctx = egui::Context::default();
        theme::apply(&ctx, &theme::DARK);
        let column = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(width, 4_000.0));
        let mut overflow = 0.0;
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |root| {
                let mut ui = root.new_child(egui::UiBuilder::new().max_rect(column));
                paint(&mut ui);
                let used = ui.min_rect();
                overflow = f32::max(column.left() - used.left(), used.right() - column.right());
            });
        }
        overflow.max(0.0)
    }

    #[test]
    fn number_width_matches_the_laid_out_box() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, &theme::DARK);
        for unit in [None, Some("%"), Some("km/h")] {
            let (mut measured, mut laid_out) = (0.0, 0.0);
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut value = -12_345.678_f64;
                measured = number_width(ui, value, 3, unit);
                let number = with_unit(egui::DragValue::new(&mut value).fixed_decimals(3), unit);
                laid_out = ui.horizontal(|row| row.add(number).rect.width()).inner;
            });
            assert!(
                (measured - laid_out).abs() < 0.5,
                "unit {unit:?}: measured {measured} px, laid out {laid_out} px"
            );
        }
    }

    #[test]
    fn every_param_kind_fits_the_sidebar() {
        let manifest = manifest();
        let mut icons = Icons::new();
        let mut values = BTreeMap::new();
        let overflow = overflow(SECTION_W, |ui| {
            paint_params_section(ui, &manifest, &mut values, &mut icons, &theme::DARK);
        });
        assert!(
            overflow < 0.5,
            "params overflow the sidebar by {overflow} px"
        );
    }

    #[test]
    fn the_system_section_fits_the_sidebar() {
        let mut system = bmc_wasm_runtime::SystemSnapshot::default();
        let overflow = overflow(SECTION_W, |ui| {
            TestbedApp::paint_system_section(ui, &mut system);
        });
        assert!(
            overflow < 0.5,
            "the system section overflows the sidebar by {overflow} px"
        );
    }
}

#[cfg(test)]
mod seed_tests {
    use serde_json::json;

    use super::{ItemKind, seed_item};

    fn seeded(kind: serde_json::Value) -> serde_json::Value {
        let kind: ItemKind = serde_json::from_value(kind).expect("BUG: the item kind parses");
        seed_item(&kind).to_json_value()
    }

    #[test]
    fn an_enum_item_without_a_default_seeds_its_first_option() {
        for (kind, first) in [
            (
                json!({"type": "string", "enum_values": [
                    {"value": "info", "label": "Info"},
                    {"value": "warning", "label": "Warning"},
                ]}),
                json!("info"),
            ),
            (
                json!({"type": "integer", "enum_values": [
                    {"value": 5, "label": "Five"},
                    {"value": 10, "label": "Ten"},
                ]}),
                json!(5),
            ),
            (
                json!({"type": "double", "enum_values": [
                    {"value": 0.5, "label": "Half"},
                    {"value": 1.5, "label": "One and a half"},
                ]}),
                json!(0.5),
            ),
        ] {
            assert_eq!(seeded(kind.clone()), first, "{kind}");
        }
    }

    #[test]
    fn a_new_row_seeds_a_required_enum_field_with_its_first_option() {
        let row = seeded(json!({"type": "object", "fields": {
            "label": {"type": "string", "name": "Label"},
            "tone": {"type": "string", "name": "Tone", "enum_values": [
                {"value": "info", "label": "Info"},
            ]},
            "note": {"type": "string", "name": "Note", "optional": true},
        }}));

        assert_eq!(row, json!({"label": "", "tone": "info", "note": null}));
    }

    #[test]
    fn a_plain_item_without_a_default_seeds_its_zero() {
        assert_eq!(seeded(json!({"type": "string"})), json!(""));
        assert_eq!(seeded(json!({"type": "integer"})), json!(0));
    }

    #[test]
    fn a_bounded_number_without_a_default_seeds_its_zero_clamped_into_range() {
        assert_eq!(
            seeded(json!({"type": "integer", "min": 5, "max": 10})),
            json!(5)
        );
        assert_eq!(seeded(json!({"type": "integer", "max": -3})), json!(-3));
        assert_eq!(
            seeded(json!({"type": "integer", "min": -5, "max": 5})),
            json!(0)
        );
        assert_eq!(seeded(json!({"type": "double", "min": 0.5})), json!(0.5));
    }
}

#[cfg(test)]
mod draft_tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{Manifest, ParamKey, ParamValue, Shape, seed_item, validate_draft};

    fn manifest(params: &serde_json::Value) -> Manifest {
        json!({
            "uid": "550e8400-e29b-41d4-a716-446655440000",
            "version": "0.1.0",
            "name": "X",
            "description": "Draft fixture",
            "binary": "bin/x",
            "supported_viewports": [{
                "type": "rectangular",
                "min_width": 317,
                "max_width": 317,
                "min_height": 238,
                "max_height": 238,
            }],
            "params": params,
        })
        .to_string()
        .parse()
        .expect("BUG: the draft fixture must be a valid manifest")
    }

    #[test]
    fn every_seed_passes_the_validator() {
        let kinds = [
            json!({"type": "string"}),
            json!({"type": "timezone"}),
            json!({"type": "integer", "min": 5, "max": 10}),
            json!({"type": "integer", "max": -3}),
            json!({"type": "double", "min": 0.5}),
            json!({"type": "boolean"}),
            json!({"type": "object", "fields": {
                "zone": {"type": "timezone", "name": "Zone"},
                "count": {"type": "integer", "name": "Count", "min": 1},
            }}),
        ];
        let params: serde_json::Map<String, serde_json::Value> = kinds
            .into_iter()
            .enumerate()
            .map(|(i, items)| {
                let list = json!({"type": "array", "name": "K", "items": items, "max_items": 1});
                (format!("k{i}"), list)
            })
            .collect();
        let manifest = manifest(&params.into());
        let draft: BTreeMap<ParamKey, ParamValue> = manifest
            .params
            .iter()
            .map(|(key, def)| {
                let Shape::Array(array) = def.kind.shape() else {
                    panic!("BUG: every fixture param is a list");
                };
                (key.clone(), ParamValue::List(vec![seed_item(&array.items)]))
            })
            .collect();

        assert_eq!(validate_draft(&manifest, &draft), Ok(draft.clone()));
    }

    #[test]
    fn a_draft_the_device_would_refuse_names_every_violation() {
        let manifest = manifest(&json!({
            "zones": {"type": "array", "name": "Z", "items": {"type": "timezone"}, "max_items": 2},
            "count": {"type": "integer", "name": "C", "min": 1, "default_value": 1},
            "ratio": {"type": "double", "name": "R", "default_value": 0.5},
        }));
        let value = |key: &str| match key {
            "zones" => ParamValue::List(vec![ParamValue::String(String::new())]),
            "count" => ParamValue::Integer(0),
            _ => ParamValue::Double(f64::INFINITY),
        };
        let draft = manifest
            .params
            .keys()
            .map(|key| (key.clone(), value(key.as_str())))
            .collect();

        let violations =
            validate_draft(&manifest, &draft).expect_err("BUG: every value is invalid");
        let mut found: Vec<String> = violations
            .iter()
            .map(|v| format!("{}: {}", v.path, v.message))
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                r#"["count"]: Must be at least 1"#,
                r#"["ratio"]: Must be a finite number"#,
                r#"["zones"][0]: Must be a valid timezone"#,
            ]
        );
    }
}

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

#![expect(
    clippy::cast_precision_loss,
    reason = "PARAM_PANEL_W u32 → f32 cast on a fixed UI constant"
)]

use bmc_wasm_runtime::unified_fixture::UnifiedEvent;

use super::recording::record_delivery;
use super::ui_helpers::{RADIO_GROUP_MAX_VARIANTS, combo_cell, key_label, radio_group_cell};
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
        if new_params == self.params {
            return;
        }
        // Fire `on_params_update` on every tile — operator-driven changes apply to all
        // size variants previewed in the testbed, not just the active recording tile.
        for tile in &mut self.tiles {
            if !tile.dead
                && let Some(runtime) = tile.runtime.as_mut()
            {
                runtime.deliver_params_update(new_params.clone());
            }
        }
        self.params = new_params;

        if let Some(rec) = self.recording_mode.state.as_mut() {
            let json_params: serde_json::Map<String, serde_json::Value> = self
                .params
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
                .collect();
            record_delivery(rec, || UnifiedEvent::ParamDelivery {
                params: json_params,
            });
        }
    }

    /// Render the unified right-side sidebar:
    ///  - per-widget Params (when the manifest declares any) on top,
    ///  - deck-wide System always below.
    ///
    /// The window's outer width is extended by `PARAM_PANEL_W` at startup
    /// to host this panel without compressing the central tile area.
    ///
    /// Both sections share a single vertical [`egui::ScrollArea`]
    /// so long param/system lists scroll together rather than stealing
    /// each other's height.
    pub(super) fn paint_right_panel(&mut self, root_ui: &mut egui::Ui) {
        let style = root_ui.ctx().style();
        let has_params = !self.manifest.params.is_empty();
        // Take the current snapshots out so we can mutate while
        // the egui closure borrows `self`, then put them back
        // via `apply_params_update` / `apply_system_update`
        // which detect diffs and propagate to every tile.
        let mut working_params = self.params.clone();
        let manifest_params = self.manifest.params.clone();
        let mut working_system = self.system.clone();
        let mut working_credentials = self.credentials.clone();
        let credential_slots = self.credential_slots();
        let mut credentials_changed = false;
        let mut working_offline = self.offline;
        let mut working_offset_ms = self.clock.offset_ms;
        let mut working_monotonic_offset_ms = self.clock.monotonic_offset_ms;
        let mut params_changed = false;
        let mut system_changed = false;
        let mut chosen_platform: Option<String> = None;

        egui::SidePanel::right("right_panel")
            .resizable(false)
            .exact_width(PARAM_PANEL_W as f32)
            .frame(egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::ZERO))
            .show_inside(root_ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |scroll| {
                        section_header_bar(scroll, "Platform", PARAMS_ACCENT);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::same(8))
                            .show(scroll, |inner| {
                                chosen_platform = self.paint_platform_selector(inner);
                            });
                        scroll.add_space(12.0);

                        if has_params {
                            section_header_bar(scroll, "Params", PARAMS_ACCENT);
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::same(8))
                                .show(scroll, |inner| {
                                    egui::Grid::new("params_grid")
                                        .num_columns(2)
                                        .spacing([12.0, 4.0])
                                        .min_col_width(0.0)
                                        .show(inner, |grid| {
                                            for (key, def) in &manifest_params {
                                                let current = working_params
                                                    .entry(key.clone())
                                                    .or_insert_with(|| {
                                                        bmc_widget_manifest::ParamValue::from_param_kind_default(
                                                            &def.kind,
                                                        )
                                                    });
                                                if paint_param_row(grid, key.as_str(), def, current) {
                                                    params_changed = true;
                                                }
                                            }
                                        });
                                });
                            scroll.add_space(12.0);
                        }
                        section_header_bar(scroll, "System", SYSTEM_ACCENT);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::same(8))
                            .show(scroll, |inner| {
                                system_changed =
                                    Self::paint_system_section(inner, &mut working_system);
                            });
                        scroll.add_space(12.0);
                        credentials_changed = Self::paint_credentials_section(
                            scroll,
                            &credential_slots,
                            &mut working_credentials,
                        );
                        section_header_bar(scroll, "Simulation", SYSTEM_ACCENT);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::same(8))
                            .show(scroll, |inner| {
                                Self::paint_sim_section(
                                    inner,
                                    &mut working_offline,
                                    &mut working_offset_ms,
                                    &mut working_monotonic_offset_ms,
                                );
                            });
                    });
            });

        if params_changed {
            self.apply_params_update(working_params);
        }
        if system_changed {
            self.apply_system_update(working_system);
        }
        if credentials_changed {
            self.apply_credentials_update(working_credentials);
        }
        self.offline = working_offline;
        self.clock.offset_ms = working_offset_ms;
        self.clock.monotonic_offset_ms = working_monotonic_offset_ms;
        if let Some(target) = chosen_platform {
            let ctx = root_ui.ctx().clone();
            self.switch_platform(&target, &ctx);
        }
    }

    /// Offline seal + clock fast-forward, to reach time-gated states like staleness.
    /// An advance bumps both the display and monotonic offsets; "reset" zeroes only
    /// the display one so the monotonic clock never rewinds past pending deadlines.
    fn paint_sim_section(
        ui: &mut egui::Ui,
        offline: &mut bool,
        offset_ms: &mut u64,
        monotonic_offset_ms: &mut u64,
    ) {
        ui.checkbox(offline, "Offline (seal live I/O)");
        ui.add_space(4.0);
        let secs = *offset_ms / 1_000;
        ui.horizontal(|row| {
            row.label(format!("Clock +{}:{:02}", secs / 60, secs % 60));
            if row.button("+1m").clicked() {
                *offset_ms += 60_000;
                *monotonic_offset_ms += 60_000;
            }
            if row.button("+5m").clicked() {
                *offset_ms += 300_000;
                *monotonic_offset_ms += 300_000;
            }
            if row.button("reset").clicked() {
                *offset_ms = 0;
            }
        });
    }
}

// TODO(BDK-476): replace with named palette references (`ORANGE_40` / `TEAL_40`)
// once `Color` is extracted to a shared crate that the testbed (egui consumer)
// can depend on without dragging the host into the wasmi-wire protocol's dep tree.
// The hex values below mirror those two palette swatches verbatim.
const PARAMS_ACCENT: egui::Color32 = egui::Color32::from_rgb(0xFE, 0x84, 0x31);
pub(super) const SYSTEM_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0xBA, 0xC5);

/// Render a section header as a full-width horizontal accent banner with
/// black text — no left stripe.
///
/// Painted directly via [`egui::Painter`] into a single allocated rect
/// rather than via a nested `Frame` + `Layout`: the latter let the inner
/// ui's min-size expand into the surrounding ScrollArea's full vertical,
/// turning the banner into a panel-tall solid block.
pub(super) fn section_header_bar(ui: &mut egui::Ui, text: &str, accent: egui::Color32) {
    let width = ui.available_width();
    let bar_height: f32 = 26.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, bar_height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
    painter.text(
        rect.min + egui::vec2(10.0, bar_height / 2.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(14.0),
        accent,
    );
}

// ── Param-mutation inputs ───────────────────────────────────────────

/// Render one row inside the params Grid: monospace key in the left column,
/// type-appropriate input + optional clear-to-null toggle in the right column.
///
/// Returns `true` when the operator changed the value this frame.
///
/// Caller (`paint_params_panel`) wraps this in `egui::Grid::show`
/// so the two columns stay aligned across rows regardless
/// of key length or input width.
fn paint_param_row(
    grid: &mut egui::Ui,
    key: &str,
    def: &bmc_widget_manifest::ParamDefinition,
    value: &mut bmc_widget_manifest::ParamValue,
) -> bool {
    use bmc_widget_manifest::{ParamKind, ParamValue};

    let mut changed = false;
    // Top-align the key label within its cell so a tall row (radio group)
    // has its label anchored at the first option, not vertically centred
    // halfway down the group. `left_to_right` keeps the label on a single
    // line; `Align::TOP` anchors it at the row's top edge.
    let label_resp = grid
        .with_layout(egui::Layout::left_to_right(egui::Align::TOP), |row| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    top: 3,
                    ..Default::default()
                })
                .show(row, |inner| inner.add(key_label(key, 180)))
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
            if row.small_button(label).clicked() {
                if is_null {
                    *value = ParamValue::from_param_kind_default(&def.kind);
                    // If the default is also Null (optional-without-default),
                    // seed with a type-appropriate zero so the input below
                    // has something to edit.
                    if matches!(value, ParamValue::Null) {
                        *value = match &def.kind {
                            ParamKind::String { .. } | ParamKind::Timezone { .. } => {
                                ParamValue::String(String::new())
                            }
                            ParamKind::Integer { .. } => ParamValue::Integer(0),
                            ParamKind::Double { .. } => ParamValue::Double(0.0),
                            ParamKind::Boolean { .. } => ParamValue::Boolean(false),
                        };
                    }
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
        changed |= paint_typed_input(row, key, &def.kind, value, &label_resp);
    });
    grid.end_row();
    changed
}

/// Render an `egui::Slider` so its compound widget (track + value box) fills `cell_w`.
/// `Slider` doesn't honour `add_sized` for its track width — the track size comes from
/// `ui.spacing().slider_width`. We pre-allocate the cell, scope a temporary `slider_width`
/// equal to the cell minus the value-box estimate, then run the slider inside the scoped
/// ui so the operator sees a track that actually fills the column.
///
/// The value-box reserve is derived from `ui.spacing().interact_size.x` (egui's default
/// minimum interactable width for value-like widgets), not a constant.
fn stretched_slider<R>(ui: &mut egui::Ui, cell_w: f32, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let row_h = ui.spacing().interact_size.y;
    let value_box_reserve = ui.spacing().interact_size.x + ui.spacing().item_spacing.x;
    let track_w = (cell_w - value_box_reserve).max(0.0);
    let result = ui.allocate_ui(egui::vec2(cell_w, row_h), |slot| {
        slot.spacing_mut().slider_width = track_w;
        f(slot)
    });
    result.inner
}

/// Inner dispatch: actual editable widget per kind. Caller has already drawn the label
/// and (when applicable) the optional toggle. Each branch wraps its widget in `add_sized`
/// so the column lines up visually with the longest TextEdit-style input.
///
/// Control width comes from `ui.available_width()` — the parent Grid + horizontal layout
/// has already reserved space for the key label and any optional toggle, so what's left is
/// exactly what we want the input to fill. No constant, layout naturally follows sidebar
/// resizes or label changes.
///
/// `too_many_lines` is `expect`ed because the match is one arm per `ParamKind` variant +
/// enum-or-not split; pulling each branch into its own function would obscure the otherwise
/// trivial widget construction at every site.
#[expect(
    clippy::too_many_lines,
    reason = "linear ParamKind dispatch — splitting hurts readability"
)]
fn paint_typed_input(
    ui: &mut egui::Ui,
    key: &str,
    kind: &bmc_widget_manifest::ParamKind,
    value: &mut bmc_widget_manifest::ParamValue,
    label_resp: &egui::Response,
) -> bool {
    use bmc_widget_manifest::{ParamKind, ParamValue};

    let row_h = ui.spacing().interact_size.y;
    let cell_w = ui.available_width();
    let cell = egui::vec2(cell_w, row_h);
    let focus_on_label_click = |r: &egui::Response| {
        if label_resp.clicked() {
            r.request_focus();
        }
    };
    match (kind, value) {
        (ParamKind::String { enum_values, .. }, ParamValue::String(s))
            if !enum_values.is_empty() =>
        {
            // Snapshot the collapsed-state label before `populate`
            // captures `s` mutably; the radio branch ignores it.
            let combo_label = s.clone();
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
            if enum_values.len() <= RADIO_GROUP_MAX_VARIANTS {
                radio_group_cell(ui, key, cell_w, populate)
            } else {
                combo_cell(ui, key, cell_w, combo_label, populate)
            }
        }
        (ParamKind::String { .. } | ParamKind::Timezone { .. }, ParamValue::String(s)) => {
            let resp = ui.add_sized(cell, egui::TextEdit::singleline(s));
            focus_on_label_click(&resp);
            resp.changed()
        }
        (ParamKind::Integer { enum_values, .. }, ParamValue::Integer(n))
            if !enum_values.is_empty() =>
        {
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
            if enum_values.len() <= RADIO_GROUP_MAX_VARIANTS {
                radio_group_cell(ui, key, cell_w, populate)
            } else {
                combo_cell(ui, key, cell_w, combo_label, populate)
            }
        }
        (ParamKind::Integer { min, max, step, .. }, ParamValue::Integer(n)) => {
            // Bounded ranges use a `Slider` with `trailing_fill` so the cell shows
            // the value as a progress fill against `min..=max` (the GIMP-style look).
            // Unbounded integers fall back to a `DragValue` since `Slider` requires a finite range.
            if let (Some(lo), Some(hi)) = (min, max) {
                stretched_slider(ui, cell_w, |sl| {
                    let resp = sl.add(
                        egui::Slider::new(n, *lo..=*hi)
                            .step_by(step.map_or(1.0, f64::from))
                            .trailing_fill(true),
                    );
                    focus_on_label_click(&resp);
                    resp.changed()
                })
            } else {
                let mut dv = egui::DragValue::new(n).speed(step.map_or(1.0, f64::from));
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
        (ParamKind::Double { enum_values, .. }, ParamValue::Double(f))
            if !enum_values.is_empty() =>
        {
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
            if enum_values.len() <= RADIO_GROUP_MAX_VARIANTS {
                radio_group_cell(ui, key, cell_w, populate)
            } else {
                combo_cell(ui, key, cell_w, combo_label, populate)
            }
        }
        (ParamKind::Double { min, max, step, .. }, ParamValue::Double(f)) => {
            // Same dispatch as Integer: bounded ranges get
            // the filled-slider treatment, unbounded fall back to DragValue.
            if let (Some(lo), Some(hi)) = (min, max) {
                stretched_slider(ui, cell_w, |sl| {
                    let resp = sl.add(
                        egui::Slider::new(f, *lo..=*hi)
                            .step_by(step.unwrap_or(0.0))
                            .trailing_fill(true),
                    );
                    focus_on_label_click(&resp);
                    resp.changed()
                })
            } else {
                let mut dv = egui::DragValue::new(f).speed(step.unwrap_or(0.1));
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
        (ParamKind::Boolean { .. }, ParamValue::Boolean(b)) => {
            let cb_changed = ui.checkbox(b, "").changed();
            let label_clicked = label_resp.clicked();
            if label_clicked {
                *b = !*b;
            }
            cb_changed || label_clicked
        }
        // Type mismatch (value's variant doesn't match kind)
        // — shouldn't happen with a well-formed manifest + default-init path,
        // but render a read-only label rather than crashing if it does.
        _ => {
            ui.label(
                egui::RichText::new("(type mismatch)")
                    .color(egui::Color32::from_rgb(200, 80, 80))
                    .font(egui::FontId::monospace(10.0)),
            );
            false
        }
    }
}
